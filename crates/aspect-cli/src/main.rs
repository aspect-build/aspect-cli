mod cmd_tree;
mod flags;
mod helpers;
mod trace;

use std::collections::HashMap;
use std::env::var;
use std::path::PathBuf;
use std::process::ExitCode;

use aspect_telemetry::{cargo_pkg_short_version, cargo_pkg_version, do_not_track, send_telemetry};
use axl_runtime::engine::config::ConfiguredTask;
use axl_runtime::engine::task_arg::TaskArg;
use axl_runtime::engine::task_args::TaskArgs;
use axl_runtime::eval::{self, FrozenTaskModuleLike, ModuleScope, execute_task_with_args};
use axl_runtime::module::{AXL_MODULE_FILE, AXL_ROOT_MODULE_NAME};
use axl_runtime::module::{AxlModuleEvaluator, DiskStore};
use clap::{Arg, ArgAction, Command};
use miette::{IntoDiagnostic, miette};
use starlark::environment::FrozenModule;

use starlark::values::ValueLike;
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd_tree::{BUILTIN_COMMAND_DISPLAY_ORDER, CommandTree, make_command_from_task};
use crate::helpers::{
    find_repo_root, get_default_axl_search_paths, parse_axl_config_env, search_sources,
};

// Helper function to check if debug mode is enabled based on the ASPECT_DEBUG environment variable.
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
async fn main() -> miette::Result<ExitCode> {
    // Honor DO_NOT_TRACK
    if !do_not_track() {
        let _ = task::spawn(send_telemetry());
    }

    // Initialize tracing for logging and instrumentation.
    let _tracing = trace::init();
    // Enter the root tracing span for the entire application.
    let _root = info_span!("root").entered();

    // Get the current working directory.
    let current_work_dir = std::env::current_dir().into_diagnostic()?;

    // Find the repository root directory asynchronously.
    let repo_root = find_repo_root(&current_work_dir)
        .await
        .map_err(|err| miette!("could not find root directory: {:?}", err))?;

    // Create a DiskStore for managing module storage on disk.
    let disk_store = DiskStore::new(repo_root.clone());

    // Initialize the AxlModuleEvaluator for evaluating AXL modules.
    let module_eval = AxlModuleEvaluator::new(repo_root.clone());

    // Enter a tracing span for expanding the module store.
    let _ = info_span!("expand_module_store").enter();

    // Creates the module store and evaluates the root MODULE.aspect (if it exists) for axl_*_deps, use_task, etc...
    let root_module_store = module_eval
        .evaluate(AXL_ROOT_MODULE_NAME.to_string(), repo_root.clone())
        .into_diagnostic()?;

    // Expand all module dependencies (including the builtin @aspect module) to the disk store and collect their root paths.
    // This results in a Vec of (String, PathBuf) such as
    // [
    //     ( "aspect", "/Users/username/Library/Caches/axl/deps/27e6d838c365a7c5d79674a7b6c7ec7b8d22f686dbcc8088a8d1454a6489a9ae/aspect" ),
    //     ( "experimental", "/Users/username/Library/Caches/axl/deps/27e6d838c365a7c5d79674a7b6c7ec7b8d22f686dbcc8088a8d1454a6489a9ae/experimental" ),
    //     ( "local", "/Users/username/Library/Caches/axl/deps/27e6d838c365a7c5d79674a7b6c7ec7b8d22f686dbcc8088a8d1454a6489a9ae/local" ),
    // ]
    let module_roots = disk_store
        .expand_store(&root_module_store)
        .await
        .into_diagnostic()?;

    // Collect root and dependency modules into a vector of modules with exported tasks.
    let mut modules = vec![(
        root_module_store.module_name,
        root_module_store.module_root,
        root_module_store.tasks.take(),
    )];

    for (name, root) in module_roots {
        let module_store = module_eval.evaluate(name, root).into_diagnostic()?;
        if debug_mode() {
            eprintln!(
                "module @{} at {:?}",
                module_store.module_name, module_store.module_root
            );
        };
        modules.push((
            module_store.module_name,
            module_store.module_root,
            module_store.tasks.take(),
        ))
    }

    // Scan for .axl scripts and .config.axl files in the default search paths
    // (based on current directory and repo root).
    let search_paths = get_default_axl_search_paths(&current_work_dir, &repo_root);
    let (scripts, configs) = search_sources(&search_paths).await.into_diagnostic()?;

    // Get additional configs from AXL_CONFIG environment variable
    let env_configs = parse_axl_config_env().await.into_diagnostic()?;

    // Enter a tracing span for evaluation of scripts and configs.
    let espan = info_span!("eval");

    // Starlark thread for command execution that is spawned via spawn_blocking will allow Starlark
    // code run on a blocking thread pool separate from the threads that drive the async work.
    let out = spawn_blocking(move || {
        let _enter = espan.enter();

        let axl_deps_root = disk_store.deps_path();
        let cli_version = cargo_pkg_short_version();

        // Evaluate all scripts to find tasks and configs. The order of task discovery will be load bearing in the future
        // when task overloading is supported
        // 1. repository axl_sources
        // 2. use_task in the root module
        // 3. auto_use_tasks from the @aspect built-in module (if not overloaded by an dep in the root MODULE.aspect)
        // 4. auto_use_tasks from axl module deps in the root MODULE.aspect
        let axl_loader = eval::Loader::new(&cli_version, &repo_root, &axl_deps_root);

        // Create evaluators for tasks and configs.
        let task_eval = eval::task::TaskEvaluator::new(&axl_loader);
        let config_eval = eval::config::ConfigEvaluator::new(&axl_loader);

        // Evaluate auto-discovered AXL scripts to scan for tasks (returns FrozenModule)
        let task_modules: Vec<FrozenModule> = scripts
            .iter()
            .map(|path| {
                let rel_path = path.strip_prefix(&repo_root).unwrap().to_path_buf();
                task_eval.eval(
                    ModuleScope {
                        name: AXL_ROOT_MODULE_NAME.to_string(),
                        path: repo_root.clone(),
                    },
                    &rel_path,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;

        // Evaluate AXL scripts from use_task statements in MODULE.aspect files.
        let mut use_task_modules: Vec<(
            String,
            PathBuf,
            HashMap<PathBuf, (FrozenModule, String, Vec<String>)>,
        )> = vec![];

        for (module_name, module_root, map) in modules.into_iter() {
            let mut mmap = HashMap::new();
            for (path, (label, symbols)) in map.into_iter() {
                let rel_path = path.strip_prefix(&module_root).unwrap().to_path_buf();
                let frozen_module = task_eval
                    .eval(
                        ModuleScope {
                            name: module_name.clone(),
                            path: module_root.clone(),
                        },
                        &rel_path,
                    )
                    .into_diagnostic()?;
                mmap.insert(path, (frozen_module, label, symbols));
            }
            use_task_modules.push((module_name, module_root, mmap));
        }

        // Collect tasks from evaluated scripts.
        let mut tasks: Vec<ConfiguredTask> = Vec::new();

        for (i, frozen_module) in task_modules.iter().enumerate() {
            let path = scripts.get(i).unwrap();
            for symbol in frozen_module.tasks() {
                let task_mut =
                    ConfiguredTask::from_frozen_module(frozen_module, &symbol, path.clone())
                        .into_diagnostic()?;
                tasks.push(task_mut);
            }
        }

        for (module_name, module_root, map) in use_task_modules.iter() {
            for (path, (frozen_module, label, symbols)) in map.iter() {
                for symbol in symbols {
                    if frozen_module.has_name(symbol) {
                        if !frozen_module.has_task(symbol) {
                            return Err(miette!(
                                "invalid use_task({:?}, {:?}) call in @{} module at {}/{}",
                                label,
                                symbol,
                                module_name,
                                module_root.display(),
                                AXL_MODULE_FILE
                            ));
                        };
                        let task_mut =
                            ConfiguredTask::from_frozen_module(frozen_module, symbol, path.clone())
                                .into_diagnostic()?;
                        tasks.push(task_mut);
                    } else {
                        return Err(miette!(
                            "task symbol {:?} not found in @{} module use_task({:?}, {:?}) at {}/{}",
                            symbol,
                            module_name,
                            label,
                            symbol,
                            module_root.display(),
                            AXL_MODULE_FILE
                        ));
                    }
                }
            }
        }

        // Build scoped configs list combining regular configs and env configs
        let root_scope = ModuleScope {
            name: AXL_ROOT_MODULE_NAME.to_string(),
            path: repo_root.clone(),
        };

        let mut scoped_configs: Vec<(ModuleScope, PathBuf)> = configs
            .iter()
            .map(|path| (root_scope.clone(), path.clone()))
            .collect();

        // Run environment configs, each with scope derived from parent directory
        if debug_mode() && !env_configs.is_empty() {
            eprintln!("AXL_CONFIG configs:");
            for path in &env_configs {
                eprintln!(
                    "  - {} (scope: {})",
                    path.display(),
                    path.parent()
                        .map_or("repo root".to_string(), |p| p.display().to_string())
                );
            }
        }

        for config_path in env_configs.iter() {
            let parent = config_path.parent().unwrap_or(&repo_root);
            let scope = ModuleScope {
                name: "".to_string(),
                path: parent.to_path_buf(),
            };
            scoped_configs.push((scope, config_path.clone()));
        }

        // Run all config functions, passing in vector of tasks for configuration
        let tasks = config_eval
            .run_all(scoped_configs, tasks)
            .into_diagnostic()?;

        // Build the command tree from the evaluated and configured tasks.
        let mut tree = CommandTree::default();

        // Create the base Clap command for the 'aspect' CLI.
        // TODO: add .about()
        let cmd = Command::new("aspect")
            // set binary name to "aspect" in help
            .bin_name("aspect")
            // add an about string
            .about("Aspect's programmable task runner built on top of Bazel\n{ Correct, Fast, Usable } -- Choose three")
            // customize the subcommands section title to "Tasks:"
            .subcommand_help_heading("Tasks")
            // customize the usage string to use <TASK>
            .subcommand_value_name("TASK")
            // handle --version and -v flags
            .version(cargo_pkg_short_version())
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

        // Convert each task into a Clap subcommand and insert into the command tree.
        for (i, task) in tasks.iter().enumerate() {
            let name = task.get_name();
            let def = task.as_task().unwrap();
            let group = def.group();
            let task_path = task.path.clone();
            let rel_path = match task_path.strip_prefix(&repo_root) {
                Ok(p) => p.to_path_buf(),
                Err(_) => task_path.clone(),
            };
            let mut found = None;
            for (module_name, module_root, _) in &use_task_modules {
                if task_path.starts_with(module_root) {
                    if module_name == AXL_ROOT_MODULE_NAME {
                        continue;
                    }
                    let module_rel_path = match task_path.strip_prefix(module_root) {
                        Ok(p) => p.to_path_buf(),
                        Err(_) => task_path.clone(),
                    };
                    found = Some((module_name.clone(), module_rel_path));
                    break;
                }
            }
            let defined_in = if let Some((module_name, rel_path)) = found {
                format!("@{}//{}", module_name, rel_path.display())
            } else {
                format!("{}", rel_path.display())
            };
            let cmd = make_command_from_task(&name, &defined_in, i.to_string(), def);
            tree.insert(&name, group, group, &defined_in, cmd)
                .into_diagnostic()?;
        }

        // Convert the command tree into a full Clap command with subcommands.
        let cmd = tree.as_command(cmd, &[]).into_diagnostic()?;

        // Parse command-line arguments against the Clap command structure.
        let matches = match cmd.try_get_matches() {
            Ok(m) => m,
            Err(err) => {
                err.print().into_diagnostic()?;
                return Ok(ExitCode::from(err.exit_code() as u8));
            }
        };

        // Handle the built-in 'version' subcommand if present.
        if let Some("version") = matches.subcommand_name() {
            println!("Aspect CLI {:}", cargo_pkg_version());
            return Ok(ExitCode::SUCCESS);
        }

        // Extract the deepest subcommand from the matches (drilling down through groups).
        let mut cmd = matches.subcommand().expect("failed to get command");
        while let Some(subcmd) = cmd.1.subcommand() {
            cmd = subcmd;
        }

        // Get the task name and arguments from the final subcommand.
        let (name, cmdargs) = cmd;
        let id: usize = tree.get_task_id(cmdargs);
        let task = tasks.get(id).expect("task must exist at the indice");
        let definition = task.as_task().unwrap();

        // Enter a tracing span for task execution.
        let span = info_span!("task", name = name);
        let _enter = span.enter();

        // Create an AxlStore for task execution
        let store = axl_loader.new_store(task.path.clone());

        // Execute the selected task using the new execution function
        let exit_code = execute_task_with_args(task, store, |heap| {
            let mut args = TaskArgs::new();
            for (k, v) in definition.args().iter() {
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
                    TaskArg::StringList { .. } => heap.alloc(TaskArgs::alloc_list(
                        cmdargs
                            .get_many::<String>(k.as_str())
                            .unwrap_or_default()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>(),
                    )),
                    TaskArg::BooleanList { .. } => heap.alloc(TaskArgs::alloc_list(
                        cmdargs
                            .get_many::<bool>(k.as_str())
                            .unwrap_or_default()
                            .cloned()
                            .collect::<Vec<_>>(),
                    )),
                    TaskArg::IntList { .. } => heap.alloc(TaskArgs::alloc_list(
                        cmdargs
                            .get_many::<i32>(k.as_str())
                            .unwrap_or_default()
                            .cloned()
                            .collect::<Vec<_>>(),
                    )),
                    TaskArg::UIntList { .. } => heap.alloc(TaskArgs::alloc_list(
                        cmdargs
                            .get_many::<u32>(k.as_str())
                            .unwrap_or_default()
                            .cloned()
                            .collect::<Vec<_>>(),
                    )),
                };
                args.insert(k.clone(), val);
            }
            args
        })
        .into_diagnostic()?;

        Ok(ExitCode::from(exit_code.unwrap_or(0)))
    });

    // Await the blocking task result and handle any join errors.
    match out.await {
        Ok(result) => {
            drop(_root);
            drop(_tracing);
            result
        }
        Err(err) => panic!("{:?}", err),
    }
}
