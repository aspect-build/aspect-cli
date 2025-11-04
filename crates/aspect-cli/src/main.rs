mod cmd_tree;
mod flags;
mod helpers;
mod telemetry;
mod trace;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use aspect_config::cli_version;
use axl_runtime::engine::task::{AsTaskLike, FrozenTask, Task};
use axl_runtime::engine::task_arg::TaskArg;
use axl_runtime::engine::task_args::TaskArgs;
use axl_runtime::eval::{AxlScriptEvaluator, EvaluatedAxlScript};
use axl_runtime::module::{AxlModuleEvaluator, DiskStore};
use clap::{Arg, ArgAction, Command};
use miette::{miette, IntoDiagnostic};
use starlark::values::ValueLike;
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd_tree::{make_command_from_task, CommandTree, BUILTIN_COMMAND_DISPLAY_ORDER};
use crate::helpers::{find_axl_scripts, find_repo_root, get_default_axl_search_paths};

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
async fn main() -> miette::Result<ExitCode> {
    let _ = task::spawn(telemetry::send_telemetry());
    let _tracing = trace::init();
    let _root = info_span!("root").entered();

    let current_work_dir = std::env::current_dir().into_diagnostic()?;

    let repo_root = find_repo_root(&current_work_dir)
        .await
        .map_err(|_| miette!("Could not find repository root, running inside a module?"))?;

    let disk_store = DiskStore::new(repo_root.clone());

    let extension_eval = AxlModuleEvaluator::new(repo_root.clone());

    let _ = info_span!("expand_module_store").enter();

    // Creates the module store and evaluates the root MODULE.aspect (if it exists) for axl_*_deps, use_task, etc...
    let module_store = extension_eval
        .evaluate("_root_".to_string(), repo_root.clone())
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

    // Gather all tasks from use_task calls in the repository root axl module
    let mut use_tasks = vec![(
        module_store.module_name,
        module_store.module_root,
        module_store.tasks.take(),
    )];

    // Gather all tasks from use_task calls in the axl module deps
    for (name, root) in module_roots {
        let module_store = extension_eval.evaluate(name, root).into_diagnostic()?;
        use_tasks.push((
            module_store.module_name,
            module_store.module_root,
            module_store.tasks.take(),
        ))
    }

    let deps_path = disk_store.deps_path();

    // Get the default search paths given the current working directory and the repository root
    let search_paths = get_default_axl_search_paths(&current_work_dir, &repo_root);
    // TODO: allow user to configure additonal search paths in the future?

    // Scan for .axl files in the search paths
    let axl_sources = find_axl_scripts(&search_paths).await.into_diagnostic()?;

    let espan = info_span!("eval");

    // Starlark thread for command execution that is spawned via spawn_blocking will allow Starlark
    // code run on a blocking thread pool separate from the threads that drive the async work.
    let out = spawn_blocking(move || {
        let _enter = espan.enter();

        // TODO: add .about()
        let cmd = Command::new("aspect")
            // set binary name to "aspect" in help
            .bin_name("aspect")
            // customize the subcommands section title to "Tasks:"
            .subcommand_help_heading("Tasks")
            // customize the usage string to use <TASK>
            .subcommand_value_name("TASK")
            // handle --version and -v flags
            .version(cli_version())
            .disable_version_flag(true) // disable auto -V / --version
            .arg(
                Arg::new("version")
                    .short('v')
                    .long("version")
                    .action(ArgAction::Version)
                    .help("Print version"),
            )
            // add version command
            .subcommand(
                Command::new("version")
                    .about("Print version")
                    .display_order(BUILTIN_COMMAND_DISPLAY_ORDER),
            );

        // Collect tasks into tree
        let mut tree = CommandTree::default();
        let mut tasks: HashMap<String, EvaluatedAxlScript> = HashMap::new();

        // First gather tasks from use_task calls in axl modules
        for (module_name, module_root, use_tasks) in use_tasks {
            let te = AxlScriptEvaluator::new(module_root.clone(), deps_path.clone());

            for (relative_path, symbol) in use_tasks {
                let path = module_root.join(&relative_path);
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
                            module_name,
                            module_root
                        ));
                    };

                    let name = if def.name().is_empty() {
                        symbol.clone()
                    } else {
                        def.name().clone()
                    };
                    let rel_path = &path
                        .strip_prefix(&module_root)
                        .expect("failed make path relative")
                        .as_os_str()
                        .to_str()
                        .expect("failed to encode path");
                    let group = def.group();
                    let defined_in = format!("@{}/{}", module_name, rel_path);
                    let cmd = make_command_from_task(&name, &defined_in, &path, &symbol, def);
                    tree.insert(&name, &group, &group, &path, cmd)
                        .into_diagnostic()?;
                    tasks.insert(path.to_str().unwrap().to_string(), script);
                }
            }
        }

        // Next gather tasks from axl sources in the repository
        let te = AxlScriptEvaluator::new(repo_root.clone(), deps_path.clone());
        for path in axl_sources.iter() {
            let rel_path = path
                .strip_prefix(&repo_root)
                .map(|p| p.to_path_buf())
                .into_diagnostic()?;

            let script = te.eval(&rel_path).into_diagnostic()?;

            'inner: for symbol in script.module.names() {
                if let Some(task_val) = script.module.get(symbol.as_str()) {
                    let def = if let Some(task) = task_val.downcast_ref::<Task>() {
                        task.as_task()
                    } else if let Some(task) = task_val.downcast_ref::<FrozenTask>() {
                        task.as_task()
                    } else {
                        continue 'inner;
                    };

                    let name = if def.name().is_empty() {
                        symbol.as_str().to_string()
                    } else {
                        def.name().clone()
                    };
                    let group = def.group();
                    let defined_in = path
                        .strip_prefix(&repo_root)
                        .expect("failed make path relative")
                        .as_os_str()
                        .to_str()
                        .expect("failed to encode path");
                    let cmd = make_command_from_task(
                        &name,
                        defined_in,
                        path,
                        &symbol.as_str().to_string(),
                        def,
                    );
                    tree.insert(&name, &group, &group, &path, cmd)
                        .into_diagnostic()?;
                }
            }
            tasks.insert(path.to_str().unwrap().to_string(), script);
        }

        // Turn the command tree into a command with subcommands.
        let cmd = tree.as_command(cmd, &[]).into_diagnostic()?;

        // Match command line arguments to available commands
        let matches = match cmd.try_get_matches() {
            Ok(m) => m,
            Err(err) => {
                err.print().into_diagnostic()?;
                return Ok(ExitCode::from(err.exit_code() as u8));
            }
        };

        // If the top-level subcommand name is 'version' then print out the version information and exit success
        if let Some("version") = matches.subcommand_name() {
            println!("Aspect CLI {:}", cli_version());
            return Ok(ExitCode::SUCCESS);
        }

        // We're expecting a valid subcommand since subcommand_required is set on commands
        let mut cmd = matches.subcommand().expect("failed to get command");

        // Drill down through all command groups
        while let Some(subcmd) = cmd.1.subcommand() {
            cmd = subcmd;
        }

        let (name, cmdargs) = cmd;
        let task_path = tree.get_task_path(&cmdargs);
        let task_symbol = tree.get_task_symbol(&cmdargs);
        let task_script = tasks.get(&task_path).unwrap();
        let def = task_script.task_definition(&task_symbol).into_diagnostic()?;

        let span = info_span!("task", name = name, path = task_path, symbol = task_symbol);

        let _enter = span.enter();
        let exit_code = task_script
            .execute_task(&task_symbol, |heap| {
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
                            .alloc(cmdargs.get_one::<i32>(k.as_str()).unwrap_or(&0).to_owned())
                            .to_value(),
                        TaskArg::UInt { .. } => heap
                            .alloc(cmdargs.get_one::<u32>(k.as_str()).unwrap_or(&0).to_owned())
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
