mod builtins;
mod cmd_tree;
mod flags;
mod helpers;
mod trace;

use std::env::var;
use std::path::PathBuf;
use std::process::ExitCode;

use aspect_telemetry::{cargo_pkg_short_version, do_not_track, send_telemetry};
use axl_runtime::engine::arg::Arg;
use axl_runtime::engine::cli_args::CliArgs;
use axl_runtime::engine::config::ConfiguredTask;
use axl_runtime::engine::config::feature_map::FeatureMap;
use axl_runtime::engine::types::feature::{
    extract_feature_args, extract_feature_description, extract_feature_display_name,
    extract_feature_identifier, extract_feature_name, extract_feature_summary,
    feature_instance_effective_defaults, to_command_name,
};
use axl_runtime::eval::{self, ModuleEnv, ModuleScope, ModuleTaskSpec, MultiPhaseEval};
use axl_runtime::module::AXL_ROOT_MODULE_NAME;
use axl_runtime::module::{DiskStore, ModuleEvaluator};
use clap::parser::ValueSource;
use clap::{Arg as ClapArg, ArgAction, Command};
use starlark::collections::SmallMap;
use starlark::values::ValueLike;
use starlark::values::list::ListRef;
use tokio::task;
use tokio::task::spawn_blocking;
use tracing::info_span;

use crate::cmd_tree::{CommandTree, make_command_from_task};
use crate::helpers::{find_repo_root, get_default_axl_search_paths, search_sources};

// Generate a short human-readable task key.
fn generate_task_key() -> String {
    names::Generator::with_naming(names::Name::Plain)
        .next()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..8].to_string())
}

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
    // Honor DO_NOT_TRACK
    if !do_not_track() {
        let _ = task::spawn(send_telemetry());
    }

    // Initialize tracing for logging and instrumentation.
    let _tracing = trace::init();
    // Enter the root tracing span for the entire application.
    let _root = info_span!("root").entered();

    // Get the current working directory.
    let current_work_dir = std::env::current_dir()?;

    // Find the repository root directory asynchronously.
    let repo_root = find_repo_root(&current_work_dir)
        .await
        .map_err(|_| anyhow::anyhow!("could not find root directory"))?;

    // Create a DiskStore for managing module storage on disk.
    let disk_store = DiskStore::new(repo_root.clone());

    // Initialize the ModuleEvaluator for evaluating AXL modules.
    let module_eval = ModuleEvaluator::new(repo_root.clone());

    // Enter a tracing span for expanding the module store.
    let _ = info_span!("expand_module_store").enter();

    // Creates the module store and evaluates the root MODULE.aspect (if it exists) for axl_*_deps, use_task, etc...
    let root_module_store =
        module_eval.evaluate(AXL_ROOT_MODULE_NAME.to_string(), repo_root.clone())?;

    // Expand builtins to disk and pass them to the store expander.
    let builtins = builtins::expand_builtins(repo_root.clone(), disk_store.builtins_path())?;

    // Expand all module dependencies (including builtins) to the disk store.
    let module_roots = disk_store
        .expand_store(&root_module_store, builtins)
        .await?;

    // Collect root and dependency modules into a vector of modules with exported tasks and features.
    let mut modules = vec![(
        root_module_store.module_name,
        root_module_store.module_root,
        root_module_store.tasks.take(),
        root_module_store.features.take(),
    )];

    for (name, root) in module_roots {
        let module_store = module_eval.evaluate(name, root)?;
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
            module_store.features.take(),
        ))
    }

    // Scan for .axl scripts and .config.axl files in the default search paths
    // (based on current directory and repo root).
    let search_paths = get_default_axl_search_paths(&current_work_dir, &repo_root);
    let (scripts, configs) = search_sources(&search_paths).await?;

    // Enter a tracing span for evaluation of scripts and configs.
    let espan = info_span!("eval");

    // Starlark thread for command execution that is spawned via spawn_blocking will allow Starlark
    // code run on a blocking thread pool separate from the threads that drive the async work.
    let out = spawn_blocking(move || -> Result<ExitCode, anyhow::Error> {
        let _enter = espan.enter();

        let axl_deps_root = disk_store.deps_path();
        let cli_version = cargo_pkg_short_version();

        let axl_loader = eval::Loader::new(cli_version, repo_root.clone(), axl_deps_root);

        // Build module task specs from the evaluated MODULE.aspect stores.
        // Keep (module_name, module_root) pairs for Clap help-text "defined_in" lookup.
        let mut module_roots_for_clap: Vec<(String, PathBuf)> = Vec::new();
        let module_specs: Vec<ModuleTaskSpec> = modules
            .into_iter()
            .map(|(name, root, use_tasks, use_features)| {
                module_roots_for_clap.push((name.clone(), root.clone()));
                ModuleTaskSpec {
                    name,
                    root,
                    use_tasks,
                    use_features,
                }
            })
            .collect();

        let root_scope = ModuleScope {
            name: AXL_ROOT_MODULE_NAME.to_string(),
            path: repo_root.clone(),
        };

        ModuleEnv::with(|env| -> Result<ExitCode, anyhow::Error> {
            let mut mpe = MultiPhaseEval::new(env, &axl_loader);

            // Phase 1: discover tasks (returns live Value<'v> refs on shared heap)
            let task_values = mpe
                .eval(&scripts, root_scope.clone(), module_specs)
                .map_err(anyhow::Error::from)?;

            // Phase 2: construct feature instances onto the shared heap
            mpe.eval_features().map_err(anyhow::Error::from)?;

            // Phase 3: run config files; may add tasks via ctx.tasks.add().
            // Returns the full task list (Phase 1 tasks + dynamically added ones).
            let all_task_values = mpe
                .eval_config(&configs, &task_values, &root_scope)
                .map_err(anyhow::Error::from)?;

            // Build a type_id -> instance effective_defaults map from the live feature map.
            // Used below to show config.axl-overridden defaults in --help (e.g. enabled=false).
            let feature_effective_defaults: std::collections::HashMap<
                u64,
                std::collections::HashMap<String, Vec<String>>,
            > = mpe
                .feature_map()
                .and_then(|fm| fm.downcast_ref::<FeatureMap>())
                .map(|fm| {
                    fm.entries()
                        .into_iter()
                        .map(|(id, _, instance)| {
                            (id, feature_instance_effective_defaults(instance))
                        })
                        .collect()
                })
                .unwrap_or_default();

            // Build feature flag specs for command help.
            // Each entry is (args_map, heading, description_line, prefix); features with no args are skipped.
            let feature_arg_specs_for_cmd: Vec<(SmallMap<String, Arg>, String, String, String)> =
                mpe.feature_types_with_paths()
                    .iter()
                    .filter_map(|(type_id, val, path, symbol)| {
                        let mut args = extract_feature_args(*val)?;
                        if args.is_empty() {
                            return None;
                        }
                        let display_name = extract_feature_display_name(*val).unwrap_or_default();
                        let identifier = extract_feature_identifier(*val).unwrap_or_default();
                        let rel_path = match path.strip_prefix(&repo_root) {
                            Ok(p) => p.to_path_buf(),
                            Err(_) => path.clone(),
                        };
                        let mut label = None;
                        for (module_name, module_root) in &module_roots_for_clap {
                            if path.starts_with(module_root) {
                                if module_name == AXL_ROOT_MODULE_NAME {
                                    continue;
                                }
                                let module_rel = match path.strip_prefix(module_root) {
                                    Ok(p) => p.to_path_buf(),
                                    Err(_) => path.clone(),
                                };
                                label = Some(format!("@{}//{}", module_name, module_rel.display()));
                                break;
                            }
                        }
                        let defined_in = label.unwrap_or_else(|| format!("{}", rel_path.display()));
                        let heading = format!("{} Options", display_name);
                        let user_summary = extract_feature_summary(*val).unwrap_or_default();
                        let user_description =
                            extract_feature_description(*val).unwrap_or_default();
                        let context = format!(
                            "\x1b[3m{}\x1b[0m feature defined in \x1b[3m{}\x1b[0m",
                            identifier, defined_in
                        );
                        // Build the text shown under the section heading, following the same
                        // matrix as tasks: summary → description → context line, blank-line separated.
                        // Clap wraps the whole heading in bold+underline including \n continuations,
                        // so \x1b[0m right after \n resets that before our indented lines.
                        // \x1b[8m (conceal) hides the ":" clap appends after the heading.
                        let body = if !user_description.is_empty() {
                            user_description
                        } else if !user_summary.is_empty() {
                            user_summary
                        } else {
                            String::new()
                        };
                        let desc_text = if body.is_empty() {
                            context
                        } else {
                            format!("{}\n\n      {}", body, context)
                        };
                        let description_line = format!("\x1b[0m      {}\n\x1b[8m", desc_text);
                        // Prefer the name the feature type set on itself (via `export_as` or
                        // explicit `name =` kwarg). Fall back to the symbol name from the
                        // `use_feature(...)` declaration, which is always available and correct.
                        let prefix =
                            extract_feature_name(*val).unwrap_or_else(|| to_command_name(symbol));

                        // Apply config.axl overrides to arg defaults so --help shows the
                        // effective default (e.g. `enabled = false` in config.axl → [default: false]).
                        if let Some(overrides) = feature_effective_defaults.get(type_id) {
                            for (k, vals) in overrides {
                                if let Some(arg) = args.get_mut(k.as_str()) {
                                    if let Some(first) = vals.first() {
                                        match arg {
                                            Arg::Boolean { default, .. } => {
                                                *default = first == "true";
                                            }
                                            Arg::String { default, .. } => {
                                                *default = first.clone();
                                            }
                                            Arg::Int { default, .. } => {
                                                if let Ok(v) = first.parse() {
                                                    *default = v;
                                                }
                                            }
                                            Arg::UInt { default, .. } => {
                                                if let Ok(v) = first.parse() {
                                                    *default = v;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }

                        Some((args, heading, description_line, prefix))
                    })
                    .collect();

            // Build the command tree from ALL tasks (including dynamically added ones).
            let mut tree = CommandTree::default();

            // Create the base Clap command for the 'aspect' CLI.
            let cmd = Command::new("aspect")
                .bin_name("aspect")
                .about("Aspect's programmable task runner built on top of Bazel\n{ Correct, Fast, Usable } -- Choose three")
                .subcommand_value_name("TASK|GROUP|COMMAND")
                .disable_help_subcommand(true)
                .version(cargo_pkg_short_version())
                .disable_version_flag(true)
                .arg(
                    ClapArg::new("version")
                        .short('v')
                        .long("version")
                        .action(ArgAction::Version)
                        .help("Print version"),
                )
                .arg(
                    ClapArg::new("task-key")
                        .long("task-key")
                        .value_name("KEY")
                        .global(true)
                        .value_parser(|s: &str| {
                            if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                                Ok(s.to_string())
                            } else {
                                Err(format!("'{}' contains invalid characters (allowed: A-Za-z0-9, _, -)", s))
                            }
                        })
                        .help("A short key identifying this task invocation. Allowed characters: A-Za-z0-9, _, -. Useful when the same task runs multiple times in one pipeline (e.g. 'backend', 'frontend'). Auto-generated if not set."),
                )
                .arg(
                    ClapArg::new("task-id")
                        .long("task-id")
                        .value_name("UUID")
                        .global(true)
                        .value_parser(|s: &str| {
                            uuid::Uuid::parse_str(s)
                                .map(|u| u.to_string())
                                .map_err(|_| format!("'{}' is not a valid UUID (expected xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)", s))
                        })
                        .help("A UUID uniquely identifying this task invocation. Auto-generated if not set."),
                )
                .subcommand(
                    Command::new("version")
                        .about("Print version")
                        .hide(true),
                )
                .subcommand(
                    Command::new("help")
                        .about("Print this message or the help of the given subcommand(s)")
                        .hide(true),
                );

            // Convert each task value into a Clap subcommand and insert into the command tree.
            for (i, task_val) in all_task_values.iter().enumerate() {
                let ct = task_val
                    .downcast_ref::<ConfiguredTask>()
                    .expect("task_values contains ConfiguredTask");
                let name = ct.get_name();
                let def = ct.as_task().unwrap();
                let group = def.group();
                let task_path = ct.path.clone();
                let rel_path = match task_path.strip_prefix(&repo_root) {
                    Ok(p) => p.to_path_buf(),
                    Err(_) => task_path.clone(),
                };

                // Determine "defined_in" label for help text
                let mut found = None;
                for (module_name, module_root) in &module_roots_for_clap {
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

                // Compute effective defaults: config_overrides as typed string slices for Clap.
                //
                // Scalar values (string, int, bool) become a single-element Vec<String>.
                // List values are expanded into their individual element strings.
                // Booleans are lowercased because Clap's `value_parser!(bool)` expects
                // "true"/"false" while Starlark's to_string() gives "True"/"False".
                let effective_defaults: std::collections::HashMap<String, Vec<String>> = ct
                    .config_overrides
                    .borrow()
                    .iter()
                    .map(|(k, owned)| {
                        let v = owned.value();
                        let elements: Vec<String> = if let Some(list) = ListRef::from_value(v) {
                            list.iter()
                                .map(|elem| {
                                    let s = elem.to_string();
                                    if elem.get_type() == "bool" {
                                        s.to_lowercase()
                                    } else {
                                        s
                                    }
                                })
                                .collect()
                        } else {
                            let s = v.to_string();
                            let s = if v.get_type() == "bool" {
                                s.to_lowercase()
                            } else {
                                s
                            };
                            vec![s]
                        };
                        (k.clone(), elements)
                    })
                    .collect();

                let cmd = make_command_from_task(
                    &name,
                    &defined_in,
                    i.to_string(),
                    def,
                    &feature_arg_specs_for_cmd,
                    &effective_defaults,
                );
                tree.insert(&name, group, group, &defined_in, cmd)?;
            }

            // Build the "Task Groups:" section from the tree before converting.
            let group_names = tree.group_names();
            let task_groups_section = if group_names.is_empty() {
                String::new()
            } else {
                let max_len = group_names.iter().map(|n| n.len()).max().unwrap_or(0);
                let mut section = String::from("\n\n\x1b[1;4mTask Groups:\x1b[0m\n");
                for name in &group_names {
                    let padding = " ".repeat(max_len - name.len() + 2);
                    section.push_str(&format!(
                        "  \x1b[1m{}\x1b[0m{}\x1b[3m{}\x1b[0m task group\n",
                        name, padding, name
                    ));
                }
                section
            };

            let cmd = cmd.help_template(format!(
                "\
{{about-with-newline}}
{{usage-heading}} {{usage}}

\x1b[1;4mTasks:\x1b[0m
{{subcommands}}{task_groups_section}
\x1b[1;4mCommands:\x1b[0m
  \x1b[1mversion\x1b[0m  Print version
  \x1b[1mhelp\x1b[0m     Print this message or the help of the given subcommand(s)

\x1b[1;4mOptions:\x1b[0m
{{options}}"
            ));

            let cmd = tree.as_command(cmd, &[])?;
            let mut cmd_for_help = cmd.clone();

            // Parse command-line arguments.
            let matches = match cmd.try_get_matches() {
                Ok(m) => m,
                Err(err) => {
                    err.print().ok();
                    return Ok(ExitCode::from(err.exit_code() as u8));
                }
            };

            // Handle built-in commands.
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

            // Extract the deepest subcommand from the matches.
            let mut cmd = matches.subcommand().expect("failed to get command");
            while let Some(subcmd) = cmd.1.subcommand() {
                cmd = subcmd;
            }

            let (name, cmdargs) = cmd;
            let id: usize = tree.get_task_id(cmdargs);
            let task_val = all_task_values[id];
            let ct = task_val
                .downcast_ref::<ConfiguredTask>()
                .expect("all_task_values contains ConfiguredTask");
            let definition = ct.as_task().unwrap();

            // Resolve task key and task ID from CLI flags.
            let task_key = cmdargs
                .get_one::<String>("task-key")
                .filter(|s| !s.is_empty())
                .cloned()
                .unwrap_or_else(generate_task_key);
            let task_id = cmdargs.get_one::<String>("task-id").cloned();

            // Enter a tracing span for task execution.
            let span = info_span!("task", name = name);
            let _enter = span.enter();

            // Build per-feature arg specs (type_id, prefix, args) for the Phase 4 args builder.
            // Cloned once here so the closure below can capture by value.
            let feature_specs_for_args: Vec<(u64, String, SmallMap<String, Arg>)> = mpe
                .feature_types_with_paths()
                .iter()
                .filter_map(|(id, val, _, symbol)| {
                    let args = extract_feature_args(*val)?;
                    let prefix =
                        extract_feature_name(*val).unwrap_or_else(|| to_command_name(symbol));
                    Some((*id, prefix, args))
                })
                .collect();

            // Phase 4: run enabled feature implementations, supplying parsed CLI args per feature.
            mpe.eval_feature_impls(|type_id, heap| {
                let mut args = CliArgs::new();
                let mut explicit_args = CliArgs::new();
                let Some((_, prefix, spec)) = feature_specs_for_args
                    .iter()
                    .find(|(id, _, _)| *id == type_id)
                else {
                    return (args, explicit_args);
                };
                for (k, v) in spec.iter() {
                    // Look up the value by the Clap ID (respecting long override if set)
                    // but store it under the short key (e.g. "mode") for ctx.args access.
                    let clap_key = if let Some(lo) = v.long_override() {
                        lo.to_string()
                    } else if prefix.is_empty() {
                        k.replace('_', "-")
                    } else {
                        format!("{}:{}", prefix, k.replace('_', "-"))
                    };
                    let val = match v {
                        Arg::String { .. } => heap
                            .alloc_str(
                                cmdargs
                                    .get_one::<String>(&clap_key)
                                    .unwrap_or(&String::new()),
                            )
                            .to_value(),
                        Arg::Int { .. } => heap
                            .alloc(*cmdargs.get_one::<i32>(&clap_key).unwrap_or(&0))
                            .to_value(),
                        Arg::UInt { .. } => heap
                            .alloc(*cmdargs.get_one::<u32>(&clap_key).unwrap_or(&0))
                            .to_value(),
                        Arg::Boolean { .. } => {
                            heap.alloc(*cmdargs.get_one::<bool>(&clap_key).unwrap_or(&false))
                        }
                        Arg::Positional { .. } => heap.alloc(CliArgs::alloc_list(
                            cmdargs
                                .get_many::<String>(&clap_key)
                                .map_or(vec![], |f| f.map(|s| s.as_str()).collect()),
                        )),
                        Arg::TrailingVarArgs { .. } => heap.alloc(CliArgs::alloc_list(
                            cmdargs
                                .get_many::<String>(&clap_key)
                                .map_or(vec![], |f| f.map(|s| s.as_str()).collect()),
                        )),
                        Arg::StringList { .. } => heap.alloc(CliArgs::alloc_list(
                            cmdargs
                                .get_many::<String>(&clap_key)
                                .unwrap_or_default()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>(),
                        )),
                        Arg::BooleanList { .. } => heap.alloc(CliArgs::alloc_list(
                            cmdargs
                                .get_many::<bool>(&clap_key)
                                .unwrap_or_default()
                                .copied()
                                .collect::<Vec<_>>(),
                        )),
                        Arg::IntList { .. } => heap.alloc(CliArgs::alloc_list(
                            cmdargs
                                .get_many::<i32>(&clap_key)
                                .unwrap_or_default()
                                .copied()
                                .collect::<Vec<_>>(),
                        )),
                        Arg::UIntList { .. } => heap.alloc(CliArgs::alloc_list(
                            cmdargs
                                .get_many::<u32>(&clap_key)
                                .unwrap_or_default()
                                .copied()
                                .collect::<Vec<_>>(),
                        )),
                        Arg::Custom { .. } => {
                            // Custom args are not CLI-exposed; extract_feature_args filters them out.
                            continue;
                        }
                    };
                    args.insert(k.clone(), val);
                    if cmdargs.value_source(&clap_key) == Some(ValueSource::CommandLine) {
                        explicit_args.insert(k.clone(), val);
                    }
                }
                (args, explicit_args)
            })
            .map_err(anyhow::Error::from)?;

            // Phase 5: execute the selected task

            let exit_code = mpe
                .execute_with_args(task_val, task_key, task_id, |heap| {
                    let mut all_args = CliArgs::new();
                    let mut explicit_args = CliArgs::new();
                    for (k, v) in definition.cli_args() {
                        let val = match v {
                            Arg::String { .. } => heap
                                .alloc_str(cmdargs.get_one::<String>(k).unwrap_or(&String::new()))
                                .to_value(),
                            Arg::Int { .. } => heap
                                .alloc(*cmdargs.get_one::<i32>(k).unwrap_or(&0))
                                .to_value(),
                            Arg::UInt { .. } => heap
                                .alloc(*cmdargs.get_one::<u32>(k).unwrap_or(&0))
                                .to_value(),
                            Arg::Boolean { .. } => {
                                heap.alloc(*cmdargs.get_one::<bool>(k).unwrap_or(&false))
                            }
                            Arg::Positional { .. } => heap.alloc(CliArgs::alloc_list(
                                cmdargs
                                    .get_many::<String>(k)
                                    .map_or(vec![], |f| f.map(|s| s.as_str()).collect()),
                            )),
                            Arg::TrailingVarArgs { .. } => heap.alloc(CliArgs::alloc_list(
                                cmdargs
                                    .get_many::<String>(k)
                                    .map_or(vec![], |f| f.map(|s| s.as_str()).collect()),
                            )),
                            Arg::StringList { .. } => heap.alloc(CliArgs::alloc_list(
                                cmdargs
                                    .get_many::<String>(k)
                                    .unwrap_or_default()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>(),
                            )),
                            Arg::BooleanList { .. } => heap.alloc(CliArgs::alloc_list(
                                cmdargs
                                    .get_many::<bool>(k)
                                    .unwrap_or_default()
                                    .copied()
                                    .collect::<Vec<_>>(),
                            )),
                            Arg::IntList { .. } => heap.alloc(CliArgs::alloc_list(
                                cmdargs
                                    .get_many::<i32>(k)
                                    .unwrap_or_default()
                                    .copied()
                                    .collect::<Vec<_>>(),
                            )),
                            Arg::UIntList { .. } => heap.alloc(CliArgs::alloc_list(
                                cmdargs
                                    .get_many::<u32>(k)
                                    .unwrap_or_default()
                                    .copied()
                                    .collect::<Vec<_>>(),
                            )),
                            Arg::Custom { .. } => {
                                // Custom args are not CLI-exposed; cli_args() filters them out.
                                continue;
                            }
                        };
                        all_args.insert(k.to_owned(), val);
                        if cmdargs.value_source(k) == Some(ValueSource::CommandLine) {
                            explicit_args.insert(k.to_owned(), val);
                        }
                    }
                    (all_args, explicit_args)
                })
                .map_err(anyhow::Error::from)?;

            mpe.finish();
            Ok(ExitCode::from(exit_code.unwrap_or(0)))
        })
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

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:?}");
            ExitCode::FAILURE
        }
    }
}
