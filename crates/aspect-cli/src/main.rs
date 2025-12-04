mod cmd_tree;
mod flags;
mod helpers;
mod trace;

use std::collections::HashMap;
use std::env::var;
use std::path::PathBuf;
use std::process::ExitCode;

use aspect_telemetry::{cargo_pkg_short_version, cargo_pkg_version, do_not_track, send_telemetry};
use axl_runtime::engine::config::TaskMut;
use axl_runtime::engine::task_arg::TaskArg;
use axl_runtime::engine::task_args::TaskArgs;
use axl_runtime::eval::{self, task::TaskModuleLike, ModuleScope};
use axl_runtime::module::{AxlModuleEvaluator, DiskStore};
use axl_runtime::module::{AXL_MODULE_FILE, AXL_ROOT_MODULE_NAME};
use clap::{Arg, ArgAction, Command};
use miette::{miette, IntoDiagnostic};
use starlark::environment::Module;

use starlark::values::ValueLike;
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd_tree::{make_command_from_task, CommandTree, BUILTIN_COMMAND_DISPLAY_ORDER};
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
async fn main() -> miette::Result<ExitCode> {
    // Honor DO_NOT_TRACK
    if !do_not_track() {
        let _ = task::spawn(send_telemetry());
    }

    let _tracing = trace::init();
    let _root = info_span!("root").entered();

    let current_work_dir = std::env::current_dir().into_diagnostic()?;

    let repo_root = find_repo_root(&current_work_dir)
        .await
        .map_err(|_| miette!("could not find repository root, running inside a module?"))?;

    let disk_store = DiskStore::new(repo_root.clone());

    let module_eval = AxlModuleEvaluator::new(repo_root.clone());

    let _ = info_span!("expand_module_store").enter();

    // Creates the module store and evaluates the root MODULE.aspect (if it exists) for axl_*_deps, use_task, etc...
    let module_store = module_eval
        .evaluate(AXL_ROOT_MODULE_NAME.to_string(), repo_root.clone())
        .into_diagnostic()?;

    // Expand all module deps (including the builtin @aspect module) to the disk store and return the module roots on disk.
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

    // Gather evaluated root and deps modules into modules vec
    let mut modules = vec![(
        module_store.module_name,
        module_store.module_root,
        module_store.tasks.take(),
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

    let axl_deps_root = disk_store.deps_path();

    // Get the default search paths given the current working directory and the repository root
    let search_paths = get_default_axl_search_paths(&current_work_dir, &repo_root);
    // TODO: allow user to configure additonal search paths in the future?

    // Scan for .axl files in the search paths
    let (scripts, configs) = search_sources(&search_paths).await.into_diagnostic()?;

    let espan = info_span!("eval");

    // Starlark thread for command execution that is spawned via spawn_blocking will allow Starlark
    // code run on a blocking thread pool separate from the threads that drive the async work.
    let out = spawn_blocking(move || {
        let _enter = espan.enter();

        // Evaluate all scripts to find tasks and configs. The order of task discovery will be load bearing in the future
        // when task overloading is supported
        // 1. repository axl_sources
        // 2. use_task in the root module
        // 3. auto_use_tasks from the @aspect built-in module (if not overloaded by an dep in the root MODULE.aspect)
        // 4. auto_use_tasks from axl module deps in the root MODULE.aspect
        let loader = eval::Loader::new(&axl_deps_root);
        let eval = eval::task::TaskEvaluator::new(&loader);
        let ceval = eval::config::ConfigEvaluator::new(&loader);

        let mut tasks: Vec<TaskMut> = Vec::new();

        let mods: Vec<Module> = scripts
            .iter()
            .map(|path| {
                let rel_path = path.strip_prefix(&repo_root).unwrap().to_path_buf();

                eval.eval(
                    ModuleScope {
                        name: AXL_ROOT_MODULE_NAME.to_string(),
                        path: repo_root.clone(),
                    },
                    &rel_path,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;

        for (i, module) in mods.iter().enumerate() {
            let path = scripts.get(i).unwrap();
            for symbol in module.tasks() {
                let val = module
                    .get(symbol)
                    .expect("symbol should have been defined.");

                let def = module
                    .get_task(symbol)
                    .expect("symbol should have been defined.");
                let name = if def.name().is_empty() {
                    symbol.to_string()
                } else {
                    def.name().clone()
                };

                tasks.push(TaskMut::new(
                    &module,
                    path.to_string_lossy().to_string(),
                    name,
                    def.group().clone(),
                    val,
                ));
            }
        }

        let mut usemods: Vec<(
            String,
            PathBuf,
            HashMap<PathBuf, (Module, String, Vec<String>)>,
        )> = vec![];

        for (module_name, module_root, map) in modules.into_iter() {
            let mut mmap = HashMap::new();
            for (path, (label, symbols)) in map.into_iter() {
                let rel_path = path.strip_prefix(&module_root).unwrap().to_path_buf();
                let module = eval
                    .eval(
                        ModuleScope {
                            name: module_name.clone(),
                            path: module_root.clone(),
                        },
                        &rel_path,
                    )
                    .into_diagnostic()?;
                mmap.insert(path, (module, label, symbols));
            }
            usemods.push((module_name, module_root, mmap));
        }

        for (module_name, module_root, map) in usemods.iter() {
            for (path, (module, label, symbols)) in map.iter() {
                for symbol in symbols {
                    if module.has_name(&symbol) {
                        if !module.has_task(&symbol) {
                            return Err(miette!(
                                "invalid use_task({:?}, {:?}) call in @{} module at {}/{}",
                                label,
                                symbol,
                                module_name,
                                module_root.display(),
                                AXL_MODULE_FILE
                            ));
                        };
                        let val = module
                            .get(symbol.as_str())
                            .expect("symbol should have been defined.");

                        let def = module
                            .get_task(symbol.as_str())
                            .expect("symbol should have been defined.");
                        let name = if def.name().is_empty() {
                            symbol.as_str().to_string()
                        } else {
                            def.name().clone()
                        };

                        tasks.push(TaskMut::new(
                            &module,
                            path.to_string_lossy().to_string(),
                            name,
                            def.group().clone(),
                            val,
                        ));
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

        // Call config.axl config() functions
        let config = ceval
            .run_all(
                ModuleScope {
                    name: AXL_ROOT_MODULE_NAME.to_string(),
                    path: repo_root.clone(),
                },
                configs.iter().map(|p| p.as_path()).collect(),
                tasks,
            )
            .into_diagnostic()?;

        let tasks = config.tasks();

        // Iterate through tasks after any config mutations and create the command with make_command_from_task
        let mut tree = CommandTree::default();

        // TODO: add .about()
        let cmd = Command::new("aspect")
            // set binary name to "aspect" in help
            .bin_name("aspect")
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

        for (i, task) in tasks.iter().enumerate() {
            let name = task.name.borrow();
            let def = task.as_task().unwrap();

            let group = def.group();
            // let defined_in = if script.scope.name == AXL_ROOT_MODULE_NAME {
            //     format!("{}", script.path.display())
            // } else {
            //     format!("@{}/{}", script.scope.name, script.path.display())
            // };
            let cmd = make_command_from_task(&name.to_string(), &String::new(), i.to_string(), def);
            tree.insert(
                &name,
                group,
                group,
                &task.path.to_string_lossy().to_string(),
                cmd,
            )
            .into_diagnostic()?;
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
            println!("Aspect CLI {:}", cargo_pkg_version());
            return Ok(ExitCode::SUCCESS);
        }

        // We're expecting a valid subcommand since subcommand_required is set on commands
        let mut cmd = matches.subcommand().expect("failed to get command");

        // Drill down through all command groups
        while let Some(subcmd) = cmd.1.subcommand() {
            cmd = subcmd;
        }

        let (name, cmdargs) = cmd;
        let id: usize = tree.get_task_id(&cmdargs);
        let task = tasks.get(id).expect("task must exist at the indice");
        let definition = task.as_task().unwrap();

        let span = info_span!("task", name = name);

        let _enter = span.enter();
        let exit_code = task
            .module
            .execute_task(loader.store(), task, |heap| {
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
