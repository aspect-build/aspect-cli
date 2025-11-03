mod cmd_tree;
mod flags;
mod helpers;
mod telemetry;
mod trace;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use aspect_config::cli_version;
use axl_runtime::engine::task_arg::TaskArg;
use axl_runtime::engine::task_args::TaskArgs;
use axl_runtime::engine::task::{AsTaskLike, FrozenTask, Task};
use axl_runtime::eval::{AxlScriptEvaluator, EvaluatedAxlScript};
use axl_runtime::module::{AxlModuleEvaluator, DiskStore};
use clap::{Arg, Command};
use miette::{miette, IntoDiagnostic};
use starlark::values::ValueLike;
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd_tree::{make_command, CommandTree};
use crate::helpers::{find_axl_scripts, find_repo_root, get_default_axl_search_paths};

// Must use a multi thread runtime with at least 3 threads for following reasons;
//
// Main thread (1) which drives the async runtime and all the other machinery shall
// not be starved of cpu time to perform async tasks, its sole purpose is to
// execute Rust code that drives the async runtime.
//
// Starlark thread which is spawned via spawn_blocking will allow Starlark code run on
// a blocking thread pool separate from the threads that drive the async work.
//
// On the other hand, all the other async tasks, including those spawned by Starlark
// async machinery get to run on any of these worker threads until they are ready.
//
// As a special exception the build event machinery and build event sinks get
// their own threads to react to IO streams in a timely manner.
//
// TODO: create a diagram of how all this ties together.
#[tokio::main(flavor = "multi_thread", worker_threads = 3)]
async fn main() -> miette::Result<ExitCode> {
    let _ = task::spawn(telemetry::send_telemetry());
    let _tracing = trace::init();
    let _root = info_span!("root").entered();

    let current_work_dir = std::env::current_dir().into_diagnostic()?;

    let repo_dir = find_repo_root(&current_work_dir)
        .await
        .map_err(|_| miette!("Could not find repository root, running inside a module?"))?;

    let disk_store = DiskStore::new(repo_dir.clone());

    let extension_eval = AxlModuleEvaluator::new(repo_dir.clone());

    let mut cmd = Command::new("aspect");

    cmd = cmd.arg(
        Arg::new("version")
            .short('v')
            .long("version")
            .action(clap::ArgAction::SetTrue),
    );

    let _ = info_span!("expand_module_store").enter();

    // Creates the module store and evaluates the root MODULE.aspect (if it exists) for axl_*_deps, use_task, etc...
    let module_store = extension_eval
        .evaluate("_root_".to_string(), repo_dir.clone())
        .into_diagnostic()?;

    // Expand all modules (including the builtin @aspect module) to the disk store and return the module roots on disk.
    // This results in a Vec of (String, PathBuf) such as
    // [
    //     ( "aspect", "/Users/username/Library/Caches/axl/deps/27e6d838c365a7c5d79674a7b6c7ec7b8d22f686dbcc8088a8d1454a6489a9ae/aspect" ),
    //     ( "experimental", "/Users/username/Library/Caches/axl/deps/27e6d838c365a7c5d79674a7b6c7ec7b8d22f686dbcc8088a8d1454a6489a9ae/experimental" ),
    //     ( "local", "/Users/username/Library/Caches/axl/deps/27e6d838c365a7c5d79674a7b6c7ec7b8d22f686dbcc8088a8d1454a6489a9ae/local" ),
    // ]
    let module_roots = disk_store
        .expand_store(&module_store)
        .await
        .into_diagnostic()?;

    let mut use_tasks = vec![(
        module_store.repo_name,
        module_store.repo_path,
        module_store.tasks.take(),
    )];

    for (name, root) in module_roots {
        let module_store = extension_eval.evaluate(name, root).into_diagnostic()?;
        use_tasks.push((
            module_store.repo_name,
            module_store.repo_path,
            module_store.tasks.take(),
        ))
    }

    let deps_path = disk_store.deps_path();

    // Get the default search paths given the current working directory and the repository root
    let search_paths = get_default_axl_search_paths(&current_work_dir, &repo_dir);
    // TODO: allow user to configure additonal search paths in the future?

    // Scan for .axl files in the search paths
    let axl_sources = find_axl_scripts(&search_paths)
        .await
        .into_diagnostic()?;

    let espan = info_span!("eval");

    let out = spawn_blocking(move || {
        let _enter = espan.enter();

        // Collect tasks into tree
        let mut tree = CommandTree::default();
        let mut tasks: HashMap<String, EvaluatedAxlScript> = HashMap::new();

        for (repo_name, repo_root, usetasks) in use_tasks {
            let te = AxlScriptEvaluator::new(repo_root.clone(), deps_path.clone());

            for (relative_path, symbol) in usetasks {
                let path = repo_root.join(&relative_path);
                let script = te.eval(&PathBuf::from(&relative_path)).into_diagnostic()?;
                if let Some(task_val) = script.module.get(symbol.as_str()) {
                    let def = if let Some(task) = task_val.downcast_ref::<Task>() {
                        task.as_task()
                    } else if let Some(task) = task_val.downcast_ref::<FrozenTask>() {
                        task.as_task()
                    } else {
                        return Err(miette!(
                            "invalid use_task({}, {}) call in {} at {:?}",
                            relative_path,
                            symbol,
                            repo_name,
                            repo_root
                        ));
                    };

                    let name = symbol;
                    let rel_path = &path
                        .strip_prefix(&repo_root)
                        .expect("failed make path relative")
                        .as_os_str()
                        .to_str()
                        .expect("failed to encode path");
                    let groups = def.groups();
                    let defined_in = format!("@{}/{}", repo_name, rel_path);
                    let cmd = make_command(&name, &defined_in, &path, def);
                    tree.insert(&groups, name, &path, cmd).into_diagnostic()?;
                    tasks.insert(path.to_str().unwrap().to_string(), script);
                }
            }
        }

        let te = AxlScriptEvaluator::new(repo_dir.clone(), deps_path.clone());

        for path in axl_sources.iter() {
            let rel_path = path
                .strip_prefix(&repo_dir)
                .map(|p| p.to_path_buf())
                .into_diagnostic()?;

            let script = te.eval(&rel_path).into_diagnostic()?;

            'inner: for name in script.module.names() {
                if let Some(task_val) = script.module.get(name.as_str()) {
                    let def = if let Some(task) = task_val.downcast_ref::<Task>() {
                        task.as_task()
                    } else if let Some(task) = task_val.downcast_ref::<FrozenTask>() {
                        task.as_task()
                    } else {
                        continue 'inner;
                    };

                    let name = name.as_str().to_string();
                    let groups = def.groups();
                    let defined_in = path
                        .strip_prefix(&repo_dir)
                        .expect("failed make path relative")
                        .as_os_str()
                        .to_str()
                        .expect("failed to encode path");
                    let cmd = make_command(&name, defined_in, path, def);
                    tree.insert(&groups, name, &path, cmd).into_diagnostic()?;
                }
            }

            assert!(tasks
                .insert(path.to_str().unwrap().to_string(), script)
                .is_none());
        }

        // Turn the command tree into a command with subcommands.
        cmd = tree.as_command(cmd);
        // Add version command
        cmd = cmd.subcommand(Command::new("version"));

        let matches = cmd.try_get_matches();

        if let Ok(matches) = matches {
            if let Some("version") = matches.subcommand_name() {
                let v = cli_version();
                println!("Aspect CLI {v:}");
                return Ok(ExitCode::SUCCESS);
            }

            if let Some((name, cmdargs)) = matches.subcommand() {
                let task_path = tree.get_task_path(&cmdargs);
                let task = tasks.get(&task_path).unwrap();
                let def = task.definition(name).into_diagnostic()?;

                let span = info_span!("task", name = name, path = task_path);

                let _enter = span.enter();
                let exit_code = task
                    .execute(name, |heap| {
                        let mut args = TaskArgs::new();
                        for (k, v) in def.args().iter() {
                            let val = match v {
                                TaskArg::String { .. } => heap
                                    .alloc_str(
                                        cmdargs
                                            .get_one::<String>(k.as_str())
                                            .unwrap_or(&String::new()),
                                    )
                                    .to_value(),
                                TaskArg::Int { .. } => heap
                                    .alloc(
                                        cmdargs.get_one::<i32>(k.as_str()).unwrap_or(&0).to_owned(),
                                    )
                                    .to_value(),
                                TaskArg::UInt { .. } => heap
                                    .alloc(
                                        cmdargs.get_one::<u32>(k.as_str()).unwrap_or(&0).to_owned(),
                                    )
                                    .to_value(),
                                TaskArg::Boolean { .. } => heap.alloc(
                                    cmdargs
                                        .get_one::<bool>(k.as_str())
                                        .unwrap_or(&false)
                                        .to_owned(),
                                ),
                                TaskArg::Positional { .. } => heap.alloc(TaskArgs::alloc_list(
                                    cmdargs
                                        .get_many::<String>(k.as_str())
                                        .map_or(vec![], |f| f.map(|s| s.as_str()).collect()),
                                )),
                                TaskArg::TrailingVarArgs => heap.alloc(TaskArgs::alloc_list(
                                    cmdargs
                                        .get_many::<String>(k.as_str())
                                        .map_or(vec![], |f| f.map(|s| s.as_str()).collect()),
                                )),
                            };
                            args.insert(k.clone(), val);
                        }
                        args
                    })
                    .into_diagnostic()?;

                return Ok(ExitCode::from(exit_code.unwrap_or(0)));
            }

            eprintln!("unknown command {:?}", matches.subcommand_name());
            return Ok(ExitCode::FAILURE);
        } else {
            let err = matches.unwrap_err();
            err.print().into_diagnostic()?;
            return Ok(ExitCode::from(err.exit_code() as u8));
        }
    });

    match out.await {
        Ok(err) => {
            drop(_root);
            drop(_tracing);
            err
        }
        Err(err) => panic!("{:?}", err),
    }
}
