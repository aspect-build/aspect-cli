mod builtins;
mod cmd;
mod helpers;
mod trace;

use std::env::var;
use std::process::ExitCode;

use aspect_telemetry::{cargo_pkg_short_version, do_not_track, send_telemetry};
use axl_runtime::eval::{Loader, ModuleEnv, MultiPhaseEval};
use axl_runtime::module::{AXL_ROOT_MODULE_NAME, Mod};
use axl_runtime::module::{DiskStore, ModEvaluator};
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd::Cmd;
use crate::helpers::{find_repo_root, get_default_axl_search_paths, search_sources};

fn debug_mode() -> bool {
    match var("ASPECT_DEBUG") {
        Ok(val) => !val.is_empty(),
        _ => false,
    }
}

// Must use a multi thread runtime with at least 3 threads for following reasons;
//
// Main thread (1) which drives the async runtime and all the other machinery shall
// not be starved of cpu time to perform async tasks, its sole purpose is to
// execute Rust code that drives the async runtime.
//
// Starlark thread (2) for command execution that is spawned via spawn_blocking will allow Starlark
// code run on a blocking thread pool separate from the threads that drive the async work.
//
// On the other hand, all the other async tasks, including those spawned by Starlark
// async machinery get to run on any of these worker threads (3+) until they are ready.
//
// As a special exception the build event machinery and build event sinks get
// their own threads (3+) to react to IO streams in a timely manner.
//
// TODO: create a diagram of how all this ties together.
#[tokio::main(flavor = "multi_thread", worker_threads = 3)]
async fn run() -> Result<ExitCode, anyhow::Error> {
    if !do_not_track() {
        let _ = task::spawn(send_telemetry());
    }

    let _tracing = trace::init();
    let _root = info_span!("root").entered();

    let current_work_dir = std::env::current_dir()?;
    let repo_root = find_repo_root(&current_work_dir)
        .await
        .map_err(|_| anyhow::anyhow!("could not find root directory"))?;

    let disk_store = DiskStore::new(repo_root.clone());
    let mode = ModEvaluator::new(repo_root.clone());

    let _ = info_span!("expand_module_store").enter();

    let root_mod = mode.evaluate(AXL_ROOT_MODULE_NAME.to_string(), repo_root.clone())?;
    let builtins = builtins::expand_builtins(repo_root.clone(), disk_store.builtins_path())?;
    let module_roots = disk_store.expand_store(&root_mod, builtins).await?;

    let mut modules: Vec<Mod> = vec![];
    for (name, root) in module_roots {
        let r#mod = mode.evaluate(name, root)?;
        if debug_mode() {
            eprintln!("module @{} at {:?}", r#mod.name, r#mod.root);
        };
        modules.push(r#mod)
    }

    let search_paths = get_default_axl_search_paths(&current_work_dir, &repo_root);
    let (scripts, configs) = search_sources(&search_paths).await?;

    let espan = info_span!("eval");

    let out = spawn_blocking(move || -> Result<ExitCode, anyhow::Error> {
        let _enter = espan.enter();

        let cli_version = cargo_pkg_short_version();

        ModuleEnv::with(|env| -> Result<ExitCode, anyhow::Error> {
            let loader = Loader::new(cli_version.clone(), repo_root.clone(), &modules);
            let mut mpe = MultiPhaseEval::new(env, &loader);

            // Phase 1: discover tasks and features.
            mpe.eval(&scripts, &root_mod, &modules)
                .map_err(anyhow::Error::from)?;

            // Phase 2: run config files.
            mpe.execute_configs(&configs, &root_mod)
                .map_err(anyhow::Error::from)?;

            // Build the CLI surface from current eval state.
            let cmd = Cmd {
                tasks: mpe.tasks(),
                features: mpe.features(),
                repo_root: &repo_root,
                modules: &modules,
            };
            let mut root_cmd = cmd.build(&cli_version)?;
            let mut cmd_for_help = root_cmd.clone();

            let matches = match root_cmd.try_get_matches_from_mut(std::env::args_os()) {
                Ok(m) => m,
                Err(err) => {
                    err.print().ok();
                    return Ok(ExitCode::from(err.exit_code() as u8));
                }
            };

            match matches.subcommand_name() {
                Some("version") => {
                    println!("{}", cargo_pkg_short_version());
                    return Ok(ExitCode::SUCCESS);
                }
                Some("help") => {
                    cmd_for_help.print_help()?;
                    return Ok(ExitCode::SUCCESS);
                }
                _ => {}
            }

            let dispatch = cmd.dispatch(matches)?;

            let span = info_span!("task");
            let _enter = span.enter();

            // Phase 3: run enabled feature impls.
            mpe.execute_features_with_args(|f, h| dispatch.feature_args(f, h))
                .map_err(anyhow::Error::from)?;

            // Phase 4: execute the selected task.
            let exit = mpe
                .execute_tasks_with_args(
                    dispatch.task_id,
                    dispatch.task_key.clone(),
                    dispatch.task_uuid.clone(),
                    |t, h| dispatch.task_args(t, h),
                )
                .map_err(anyhow::Error::from)?;

            mpe.finish();
            Ok(ExitCode::from(exit.unwrap_or(0)))
        })
    });

    match out.await {
        Ok(result) => {
            drop(_root);
            drop(_tracing);
            result
        }
        Err(err) => panic!("{:?}", err),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:?}");
            ExitCode::FAILURE
        }
    }
}
