use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Arg, ArgAction, Command};
use serde_json::{Value, json};

struct Prompt {
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    preambles: &'static [&'static str],
    prompt: &'static str,
}

const BAZEL_FOUNDATIONS: &str = include_str!("prompts/shared/bazel-foundations.md");
const RULES_GO_RECIPE: &str = include_str!("prompts/shared/rules-go.md");
const RULES_RS_RECIPE: &str = include_str!("prompts/shared/rules-rs.md");
const RULES_PY_2_RECIPE: &str = include_str!("prompts/shared/rules-py-2.md");
const RULES_JS_RECIPE: &str = include_str!("prompts/shared/rules-js.md");
const RULES_SCALA_RECIPE: &str = include_str!("prompts/shared/rules-scala.md");
const BAZELIFY_PREAMBLES: &[&str] = &[BAZEL_FOUNDATIONS];
const GO_PREAMBLES: &[&str] = &[BAZEL_FOUNDATIONS, RULES_GO_RECIPE];
const RUST_PREAMBLES: &[&str] = &[BAZEL_FOUNDATIONS, RULES_RS_RECIPE];
const PYTHON_PREAMBLES: &[&str] = &[BAZEL_FOUNDATIONS, RULES_PY_2_RECIPE];
const JAVASCRIPT_PREAMBLES: &[&str] = &[BAZEL_FOUNDATIONS, RULES_JS_RECIPE];
const SCALA_PREAMBLES: &[&str] = &[BAZEL_FOUNDATIONS, RULES_SCALA_RECIPE];

const BAZEL_ROOT_MARKERS: &[&str] = &["MODULE.bazel", "WORKSPACE", "WORKSPACE.bazel"];
const JAVA_BUILD_MARKERS: &[&str] = &[
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
];

const PROMPTS: &[Prompt] = &[
    Prompt {
        id: "bazelify-rust",
        title: "Bazel-ify a Rust repository",
        summary: "Introduce Bzlmod and rules_rs while preserving the existing Cargo workflow.",
        preambles: RUST_PREAMBLES,
        prompt: include_str!("prompts/bazelify-rust.md"),
    },
    Prompt {
        id: "bazelify-go",
        title: "Bazel-ify a Go repository",
        summary: "Introduce Bzlmod, rules_go, and Gazelle while preserving the Go module workflow.",
        preambles: GO_PREAMBLES,
        prompt: include_str!("prompts/bazelify-go.md"),
    },
    Prompt {
        id: "bazelify-protos",
        title: "Bazel-ify Protocol Buffer sources",
        summary: "Make protobuf source ownership and generated-language targets explicit.",
        preambles: BAZELIFY_PREAMBLES,
        prompt: include_str!("prompts/bazelify-protos.md"),
    },
    Prompt {
        id: "bazelify-containers",
        title: "Bazel-ify container images",
        summary: "Migrate an application image from Dockerfile steps to declarative OCI layers.",
        preambles: BAZELIFY_PREAMBLES,
        prompt: include_str!("prompts/bazelify-containers.md"),
    },
    Prompt {
        id: "bazelify-python",
        title: "Bazel-ify a Python repository",
        summary: "Introduce Bzlmod and Aspect rules_py without disrupting Python packaging.",
        preambles: PYTHON_PREAMBLES,
        prompt: include_str!("prompts/bazelify-python.md"),
    },
    Prompt {
        id: "bazelify-javascript",
        title: "Bazel-ify a JavaScript or TypeScript repository",
        summary: "Introduce Aspect rules_js and rules_ts while preserving the package-manager workflow.",
        preambles: JAVASCRIPT_PREAMBLES,
        prompt: include_str!("prompts/bazelify-javascript.md"),
    },
    Prompt {
        id: "bazelify-scala",
        title: "Bazel-ify a Scala repository",
        summary: "Introduce Bzlmod and rules_scala while preserving the declared Scala and sbt workflow.",
        preambles: SCALA_PREAMBLES,
        prompt: include_str!("prompts/bazelify-scala.md"),
    },
    Prompt {
        id: "upgrade-python-rules",
        title: "Upgrade a Python Bazel repository to Aspect rules_py",
        summary: "Migrate rules_python and pip-based dependency setup to Aspect rules_py incrementally.",
        preambles: PYTHON_PREAMBLES,
        prompt: include_str!("prompts/upgrade-python-rules.md"),
    },
    Prompt {
        id: "upgrade-rust-rules",
        title: "Upgrade a Rust Bazel repository to rules_rs",
        summary: "Migrate legacy rules_rust and crate-universe setup to rules_rs incrementally.",
        preambles: RUST_PREAMBLES,
        prompt: include_str!("prompts/upgrade-rust-rules.md"),
    },
    Prompt {
        id: "upgrade-oci-rules",
        title: "Migrate rules_oci images to rules_img",
        summary: "Migrate an existing rules_oci image graph to rules_img incrementally.",
        preambles: BAZELIFY_PREAMBLES,
        prompt: include_str!("prompts/upgrade-oci-rules.md"),
    },
    Prompt {
        id: "configure-remote-bazel",
        title: "Configure Aspect Remote for a Bazel repository",
        summary: "Connect an existing Bazel workspace to Aspect's managed remote cache and BES.",
        preambles: &[],
        prompt: include_str!("prompts/configure-remote-bazel.md"),
    },
    Prompt {
        id: "configure-remote-sbt",
        title: "Configure Aspect Remote for an sbt 2.x build",
        summary: "Connect an sbt 2.x build to Aspect's managed remote cache without adopting Bazel.",
        preambles: &[],
        prompt: include_str!("prompts/configure-remote-sbt.md"),
    },
];

pub fn is_invocation(args: &[OsString]) -> bool {
    args.get(1).is_some_and(|arg| arg == "prompts")
}

pub fn invocation_root(current_dir: &Path) -> PathBuf {
    invocation_root_from(current_dir, std::env::var_os("BUILD_WORKSPACE_DIRECTORY"))
}

fn invocation_root_from(current_dir: &Path, workspace_dir: Option<OsString>) -> PathBuf {
    workspace_dir
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| current_dir.to_path_buf())
}

pub fn run(args: &[OsString], root: &Path) -> ExitCode {
    let mut argv = vec![OsString::from("aspect prompts")];
    argv.extend(args.iter().skip(2).cloned());
    let matches = match command().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(error) => {
            error.print().ok();
            return ExitCode::from(error.exit_code() as u8);
        }
    };

    match matches.subcommand() {
        Some(("list", list_matches)) => {
            if list_matches.get_flag("json") {
                print_json(&catalog_json())
            } else {
                print!("{}", render_catalog());
                ExitCode::SUCCESS
            }
        }
        Some(("show", show_matches)) => {
            let id = show_matches.get_one::<String>("prompt").unwrap();
            match prompt(id) {
                Some(prompt) if show_matches.get_flag("json") => {
                    print_json(&selected_json(prompt, root))
                }
                Some(prompt) => {
                    print!("{}", render_prompt(prompt));
                    ExitCode::SUCCESS
                }
                None => unknown_prompt(id),
            }
        }
        Some(("detect", detect_matches)) => {
            if detect_matches.get_flag("json") {
                print_json(&detected_json(root))
            } else {
                print!("{}", render_detection(root));
                ExitCode::SUCCESS
            }
        }
        _ => ExitCode::FAILURE,
    }
}

fn command() -> Command {
    Command::new("prompts")
        .bin_name("aspect prompts")
        .about("Discover and print bundled agent prompts")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .after_help(
            "Prompts are instruction documents for a coding agent. Use `detect` to infer prompts, then give the Markdown from `show` to the agent with the work you want it to do.\n\n\
             These prompts are a starting point for simple and moderately complex repositories. Review and validate every generated change in your repository. For large monorepos, custom toolchains, multi-language dependency graphs, or production-critical migrations, use them to establish a bounded first slice rather than as a complete migration plan. Aspect offers Bazel support and consulting: https://www.aspect.build/services\n\n\
             Examples:\n\
               aspect prompts list\n\
               aspect prompts detect\n\
               aspect prompts show bazelify-rust\n\
               aspect prompts detect --json",
        )
        .subcommand(
            Command::new("list")
                .about("List every available prompt")
                .arg(json_arg()),
        )
        .subcommand(
            Command::new("show")
                .about("Print one agent instruction prompt as Markdown")
                .arg(
                    Arg::new("prompt")
                        .value_name("PROMPT")
                        .required(true)
                        .help("Prompt ID from `aspect prompts list`."),
                )
                .arg(json_arg()),
        )
        .subcommand(
            Command::new("detect")
                .about("Detect prompts that apply to this repository")
                .arg(json_arg()),
        )
}

fn json_arg() -> Arg {
    Arg::new("json")
        .long("json")
        .action(ArgAction::SetTrue)
        .help("Emit a structured response instead of terminal text.")
}

fn unknown_prompt(id: &str) -> ExitCode {
    eprintln!("error: unknown prompt {id:?}");
    eprintln!("run `aspect prompts list` to see available prompts.");
    ExitCode::from(2)
}

fn print_json(doc: &Value) -> ExitCode {
    match serde_json::to_string_pretty(doc) {
        Ok(doc) => {
            println!("{doc}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: failed to serialize prompt response: {error}");
            ExitCode::FAILURE
        }
    }
}

fn render_catalog() -> String {
    let mut output = String::from("Available prompts:\n\n");
    for prompt in PROMPTS {
        output.push_str(&format!("  {:<24} {}\n", prompt.id, prompt.summary));
    }
    output.push_str(
        "\nRun `aspect prompts show <PROMPT>`, then give its Markdown output to your coding agent.\n",
    );
    output
}

fn render_detection(root: &Path) -> String {
    let evidence = evidence(root);
    let candidates = recommended_prompts(root);
    if candidates.is_empty() {
        return render_no_match(&evidence);
    }

    let languages = detected_languages(root);
    let build_systems = detected_build_systems(root);
    let mut output = String::new();
    if languages.is_empty() {
        output.push_str("Recognised languages: none\n");
    } else {
        output.push_str(&format!("Recognised languages: {}\n", languages.join(", ")));
    }
    if !build_systems.is_empty() {
        output.push_str(&format!(
            "Recognised build systems: {}\n",
            build_systems.join(", ")
        ));
    }
    output.push_str(
        "\nRecommended prompt order:\n\n\
         Start with language or rules-migration prompts, then model source generation or packaging, and configure remote services after the local path is understood.\n\n",
    );
    for (index, prompt) in candidates.iter().enumerate() {
        output.push_str(&format!(
            "  {}. {:<25} {}\n",
            index + 1,
            prompt.id,
            recommendation_reason(prompt.id, root),
        ));
    }
    output.push_str(&format!("\nDetected markers: {}\n", evidence.join(", ")));
    let totals = repo_coverage(root).totals();
    if has_any(root, BAZEL_ROOT_MARKERS) && totals.units > 0 && !totals.complete() {
        let (units, with_build_files) = (totals.units, totals.with_build_files);
        output.push_str(&format!(
            "\nBazel coverage: {with_build_files} of {units} build units have a BUILD file. Migration prompts remain listed for the languages that are not modelled yet.\n",
        ));
    }
    output.push_str(
        "\nRun `aspect prompts show <PROMPT>`, then give its Markdown output to your coding agent.\n",
    );
    output
}

fn render_no_match(evidence: &[String]) -> String {
    let markers = if evidence.is_empty() {
        "none".to_owned()
    } else {
        evidence.join(", ")
    };
    format!(
        "No supported prompt scenario was detected.\n\n\
         Detected markers: {markers}\n\n\
         Run `aspect prompts list` to see available prompts, then `aspect prompts show <PROMPT>` to choose one explicitly and give its Markdown output to your coding agent.\n"
    )
}

fn catalog_json() -> Value {
    json!({
        "prompts": PROMPTS.iter().map(|prompt| json!({
            "id": prompt.id,
            "title": prompt.title,
            "summary": prompt.summary,
        })).collect::<Vec<_>>(),
    })
}

fn selected_json(prompt: &Prompt, root: &Path) -> Value {
    json!({
        "prompt_id": prompt.id,
        "evidence": evidence(root),
        "prompt": render_prompt(prompt),
    })
}

fn detected_json(root: &Path) -> Value {
    let evidence = evidence(root);
    let candidates = recommended_prompts(root)
        .into_iter()
        .enumerate()
        .map(|(index, prompt)| candidate(prompt.id, index + 1, root))
        .collect::<Vec<_>>();
    json!({
        "languages": detected_languages(root),
        "build_systems": detected_build_systems(root),
        "evidence": evidence,
        "bazel_coverage": repo_coverage(root).json(),
        "candidates": candidates,
    })
}

fn detected_prompts(root: &Path) -> Vec<&'static Prompt> {
    let has_bazel = has_any(root, BAZEL_ROOT_MARKERS);
    // A Bazel root proves the migration started, not that this language is
    // modelled: a Go module beside a Bazel workspace that has no Go rules still
    // needs bazelify-go.
    let models = |needles: &[&str]| {
        bazel_configuration(root)
            .any(|contents| needles.iter().any(|needle| contents.contains(needle)))
    };

    // A ruleset named in `MODULE.bazel` proves some of the language is
    // modelled, never that all of it is. Keep the migration prompt listed until
    // that language's packages all have a BUILD file, or the surface reports
    // the job finished when one package of eight is done.
    let coverage = repo_coverage(root);

    let mut candidates = Vec::new();
    if root.join("Cargo.toml").is_file() {
        if has_bazel && uses_legacy_rust_rules(root) {
            candidates.push(prompt("upgrade-rust-rules").unwrap());
        } else if !has_bazel || !models(&["rules_rs", "rules_rust"]) || !coverage.rust.complete() {
            candidates.push(prompt("bazelify-rust").unwrap());
        }
    }
    if has_any(root, &["go.mod", "go.work"]) {
        if has_bazel && uses_go_rules(root) {
            if !coverage.go.complete() {
                candidates.push(prompt("bazelify-go").unwrap());
            }
        } else if !has_bazel || !uses_go_rules(root) {
            candidates.push(prompt("bazelify-go").unwrap());
        }
    }
    if has_proto_sources(root) {
        candidates.push(prompt("bazelify-protos").unwrap());
    }
    if has_bazel && uses_rules_oci(root) {
        candidates.push(prompt("upgrade-oci-rules").unwrap());
    } else if has_dockerfiles(root) {
        candidates.push(prompt("bazelify-containers").unwrap());
    }
    if has_any(root, &["pyproject.toml", "setup.py", "requirements.txt"]) {
        if has_bazel && uses_legacy_python_rules(root) {
            candidates.push(prompt("upgrade-python-rules").unwrap());
        } else if !has_bazel
            || !models(&["rules_py", "rules_python"])
            || !coverage.python.complete()
        {
            candidates.push(prompt("bazelify-python").unwrap());
        }
    }
    if has_node_package_manifest(root) {
        if !has_bazel
            || uses_legacy_javascript_rules(root)
            || !models(&["rules_js", "rules_nodejs"])
            || !coverage.javascript.complete()
        {
            candidates.push(prompt("bazelify-javascript").unwrap());
        }
    }
    if has_scala_project(root)
        && (!has_bazel || !models(&["rules_scala"]) || !coverage.jvm.complete())
    {
        candidates.push(prompt("bazelify-scala").unwrap());
    }
    if has_bazel {
        candidates.push(prompt("configure-remote-bazel").unwrap());
    }
    if uses_sbt_2(root) {
        candidates.push(prompt("configure-remote-sbt").unwrap());
    }
    candidates
}

fn recommended_prompts(root: &Path) -> Vec<&'static Prompt> {
    let mut prompts = detected_prompts(root);
    prompts.sort_by_key(|prompt| match prompt.id {
        "bazelify-rust" | "upgrade-rust-rules" => 10,
        "bazelify-go" => 20,
        "bazelify-python" | "upgrade-python-rules" => 30,
        "bazelify-javascript" => 40,
        "bazelify-scala" => 50,
        "bazelify-protos" => 70,
        "bazelify-containers" | "upgrade-oci-rules" => 80,
        "configure-remote-bazel" | "configure-remote-sbt" => 90,
        _ => 100,
    });
    prompts
}

/// Why this prompt is being recommended. A partly migrated language gets a
/// reason that says so, rather than the no-Bazel-root wording that would be
/// plainly false in a repository that already has one.
fn recommendation_reason(id: &str, root: &Path) -> String {
    let coverage = repo_coverage(root);
    let language = match id {
        "bazelify-rust" => Some(("Cargo packages", coverage.rust)),
        "bazelify-go" => Some(("Go package directories", coverage.go)),
        "bazelify-python" => Some(("Python package directories", coverage.python)),
        "bazelify-javascript" => {
            Some(("JavaScript or TypeScript directories", coverage.javascript))
        }
        "bazelify-scala" => Some(("JVM source roots", coverage.jvm)),
        _ => None,
    };
    if let Some((unit, coverage)) = language
        && has_any(root, BAZEL_ROOT_MARKERS)
        && !coverage.complete()
        && coverage.with_build_files > 0
    {
        return format!(
            "Bazel models {} of {} {unit}; finish the remaining ones before treating the migration as done.",
            coverage.with_build_files, coverage.units,
        );
    }
    static_recommendation_reason(id).to_owned()
}

fn static_recommendation_reason(id: &str) -> &'static str {
    match id {
        "bazelify-rust" => {
            "No Bazel root was found for the Rust sources; establish a representative Cargo package first."
        }
        "bazelify-go" => {
            "No Bazel root was found for the Go module; establish one representative rules_go target first."
        }
        "bazelify-python" => {
            "No Bazel root was found for the Python package; establish a source-only representative target first."
        }
        "bazelify-javascript" => {
            "No Bazel root was found for the JavaScript or TypeScript package; preserve the package-manager workflow first."
        }
        "bazelify-scala" => {
            "No Bazel root was found for the Scala project; establish the declared Scala version and one representative target first."
        }
        "upgrade-rust-rules" => {
            "The existing Bazel configuration uses legacy Rust rules; migrate them in place before expanding scope."
        }
        "upgrade-python-rules" => {
            "The existing Bazel configuration uses legacy Python rules; migrate its interpreter and dependency model incrementally."
        }
        "bazelify-protos" => {
            "Protobuf sources need explicit source ownership and generated-language targets after their consuming language path is clear."
        }
        "upgrade-oci-rules" => {
            "The existing Bazel configuration uses rules_oci; migrate its image graph in place before introducing new rules_img images."
        }
        "bazelify-containers" => {
            "Dockerfiles are present; model images after the application targets and their runtime inputs are explicit."
        }
        "configure-remote-sbt" => {
            "sbt 2.x can use a Bazel-compatible gRPC remote cache, allowing CI and developers to share cacheable sbt task outputs without adopting Bazel first."
        }
        "configure-remote-bazel" => {
            "The existing Bazel repository can add remote caching and build-event streaming after the local target path is understood."
        }
        _ => "This prompt matches the repository markers.",
    }
}

fn candidate(id: &str, order: usize, root: &Path) -> Value {
    json!({
        "id": id,
        "order": order,
        "reason": recommendation_reason(id, root),
        "because": candidate_markers(id, root),
    })
}

/// The markers that selected this prompt, as opposed to every marker in the
/// repository. A caller reading `because` is asking why *this* candidate
/// appeared; the full set is already on the response as `evidence`.
fn candidate_markers(id: &str, root: &Path) -> Vec<String> {
    let mut markers: Vec<String> = Vec::new();
    let mut push = |marker: &str| markers.push(marker.to_owned());
    match id {
        "bazelify-rust" | "upgrade-rust-rules" => {
            if root.join("Cargo.toml").is_file() {
                push("Cargo.toml");
            }
            if id == "upgrade-rust-rules" {
                push("rules_rust");
            }
        }
        "bazelify-go" => {
            for marker in ["go.mod", "go.work"] {
                if root.join(marker).is_file() {
                    push(marker);
                }
            }
        }
        "bazelify-python" | "upgrade-python-rules" => {
            for marker in ["pyproject.toml", "setup.py", "requirements.txt"] {
                if root.join(marker).is_file() {
                    push(marker);
                }
            }
            if id == "upgrade-python-rules" {
                push("rules_python");
            }
        }
        "bazelify-javascript" => {
            push("package.json");
        }
        "bazelify-scala" => {
            if root.join("build.sbt").is_file() {
                push("build.sbt");
            }
            if has_source_with_extension(root, "scala") {
                push("*.scala");
            }
        }
        "bazelify-protos" => push("*.proto"),
        "upgrade-oci-rules" => push("rules_oci"),
        "bazelify-containers" => push("Dockerfile"),
        "configure-remote-bazel" => {
            for marker in BAZEL_ROOT_MARKERS {
                if root.join(marker).is_file() {
                    push(marker);
                }
            }
        }
        "configure-remote-sbt" => push("sbt.version=2.x"),
        _ => {}
    }
    markers
}

fn detected_languages(root: &Path) -> Vec<&'static str> {
    let mut languages = Vec::new();
    if root.join("Cargo.toml").is_file() {
        languages.push("Rust");
    }
    if has_any(root, &["go.mod", "go.work"]) {
        languages.push("Go");
    }
    if has_any(root, &["pyproject.toml", "setup.py", "requirements.txt"]) {
        languages.push("Python");
    }
    if has_node_package_manifest(root) {
        languages.push("JavaScript or TypeScript");
    }
    if has_scala_project(root) {
        languages.push("Scala");
    }
    if has_java_project(root) {
        languages.push("Java");
    }
    if has_proto_sources(root) {
        languages.push("Protocol Buffers");
    }
    languages
}

fn detected_build_systems(root: &Path) -> Vec<&'static str> {
    let mut build_systems = Vec::new();
    if has_any(root, BAZEL_ROOT_MARKERS) {
        build_systems.push("Bazel");
    }
    if uses_sbt_2(root) {
        build_systems.push("sbt 2.x");
    } else if root.join("build.sbt").is_file() {
        build_systems.push("sbt");
    }
    // Root-level only. A `pom.xml` or `build.gradle` inside an examples or
    // fixtures tree is that sample's build system, not the repository's, and
    // the Java recipe branches on this answer.
    if has_any(
        root,
        &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            "gradlew",
        ],
    ) {
        build_systems.push("Gradle");
    }
    if root.join("pom.xml").is_file() {
        build_systems.push("Maven");
    }
    build_systems
}

fn evidence(root: &Path) -> Vec<String> {
    let mut found = [
        "MODULE.bazel",
        "WORKSPACE",
        "WORKSPACE.bazel",
        "Cargo.toml",
        "go.mod",
        "go.work",
        "pyproject.toml",
        "setup.py",
        "requirements.txt",
        "package.json",
    ]
    .into_iter()
    .filter(|marker| root.join(marker).is_file())
    .map(str::to_owned)
    .collect::<Vec<_>>();
    if uses_legacy_python_rules(root) {
        found.push("rules_python".to_owned());
    }
    if uses_go_rules(root) {
        found.push("rules_go/Gazelle".to_owned());
    }
    if uses_legacy_rust_rules(root) {
        found.push("rules_rust".to_owned());
    }
    if has_node_package_manifest(root) && !found.iter().any(|marker| marker == "package.json") {
        found.push("package.json".to_owned());
    }
    if root.join("build.sbt").is_file() {
        found.push("build.sbt".to_owned());
    }
    if has_source_with_extension(root, "scala") {
        found.push("*.scala".to_owned());
    }
    if has_source_with_extension(root, "java") {
        found.push("*.java".to_owned());
    }
    for marker in JAVA_BUILD_MARKERS.iter().chain(["gradlew"].iter()) {
        if root.join(marker).is_file() {
            found.push((*marker).to_owned());
        }
    }
    if uses_sbt_2(root) {
        found.push("sbt.version=2.x".to_owned());
    }
    if has_proto_sources(root) {
        found.push("*.proto".to_owned());
    }
    if has_dockerfiles(root) {
        found.push("Dockerfile".to_owned());
    }
    found
}

fn has_any(root: &Path, markers: &[&str]) -> bool {
    markers.iter().any(|marker| root.join(marker).is_file())
}

fn has_node_package_manifest(root: &Path) -> bool {
    has_file_named(root, "package.json")
}

fn has_file_named(root: &Path, name: &str) -> bool {
    fn visit_named(dir: &Path, name: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name() == name && path.is_file() {
                return true;
            }

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir()
                && !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "node_modules" | "vendor")
                )
                && !entry.file_name().to_string_lossy().starts_with("bazel-")
                && visit_named(&path, name)
            {
                return true;
            }
        }
        false
    }

    visit_named(root, name)
}

fn has_source_with_extension(root: &Path, extension: &str) -> bool {
    fn visit(dir: &Path, extension: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|candidate| candidate == extension)
            {
                return true;
            }

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir()
                && !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "node_modules" | "vendor")
                )
                && !entry.file_name().to_string_lossy().starts_with("bazel-")
                && visit(&path, extension)
            {
                return true;
            }
        }
        false
    }

    visit(root, extension)
}

fn has_scala_project(root: &Path) -> bool {
    has_file_named(root, "build.sbt") || has_source_with_extension(root, "scala")
}

fn has_java_project(root: &Path) -> bool {
    has_source_with_extension(root, "java")
        || JAVA_BUILD_MARKERS
            .iter()
            .any(|marker| has_file_named(root, marker))
}

fn uses_sbt_2(root: &Path) -> bool {
    std::fs::read_to_string(root.join("project/build.properties"))
        .ok()
        .is_some_and(|contents| {
            contents.lines().any(|line| {
                line.trim()
                    .strip_prefix("sbt.version=")
                    .is_some_and(|version| version.trim().starts_with("2."))
            })
        })
}

fn has_proto_sources(root: &Path) -> bool {
    fn visit(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "proto")
            {
                return true;
            }

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir()
                && !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "node_modules" | "vendor")
                )
                && !entry.file_name().to_string_lossy().starts_with("bazel-")
                && visit(&path)
            {
                return true;
            }
        }
        false
    }

    visit(root)
}

fn has_dockerfiles(root: &Path) -> bool {
    fn visit(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name() == "Dockerfile" && path.is_file() {
                return true;
            }

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir()
                && !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "node_modules" | "vendor")
                )
                && !entry.file_name().to_string_lossy().starts_with("bazel-")
                && visit(&path)
            {
                return true;
            }
        }
        false
    }

    visit(root)
}

fn uses_legacy_python_rules(root: &Path) -> bool {
    bazel_configuration(root).any(|contents| {
        contents.contains("rules_python")
            || contents.contains("pip.parse")
            || contents.contains("pip_install")
    })
}

fn uses_rules_oci(root: &Path) -> bool {
    bazel_configuration(root)
        .any(|contents| contents.contains("rules_oci") || contents.contains("io_bazel_rules_oci"))
}

fn uses_go_rules(root: &Path) -> bool {
    bazel_configuration(root).any(|contents| {
        contents.contains("rules_go")
            || contents.contains("io_bazel_rules_go")
            || contents.contains("bazel_gazelle")
            || contents.contains("@gazelle//")
    })
}

/// BUILD files found against BUILD files expected, for one language.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Coverage {
    units: usize,
    with_build_files: usize,
}

impl Coverage {
    fn count(&mut self, has_build_file: bool) {
        self.units += 1;
        if has_build_file {
            self.with_build_files += 1;
        }
    }

    fn complete(&self) -> bool {
        self.units == 0 || self.with_build_files >= self.units
    }

    fn json(&self) -> Value {
        json!({
            "units": self.units,
            "with_build_files": self.with_build_files,
            "complete": self.complete(),
        })
    }
}

/// How much of the repository is actually modelled in Bazel. A `MODULE.bazel`
/// alone says the migration started, not that it finished, and the migration
/// prompts must not disappear the moment the first module file lands.
///
/// A unit is one directory that is expected to own a BUILD file, which is the
/// unit the language itself compiles — not every directory holding a source
/// file. Getting this wrong makes the number unreachable rather than merely
/// imprecise: a Cargo package owns its whole `src/` module tree from the
/// package root, so counting `src/` and `src/net/` as units of their own
/// leaves a fully migrated Rust repository scoring zero forever.
#[derive(Clone, Copy, Default)]
struct RepoCoverage {
    /// One per Cargo package; the package root owns `src/`.
    rust: Coverage,
    /// One per directory of `.go` files, which is what Gazelle generates.
    go: Coverage,
    /// One per directory of `.py` files.
    python: Coverage,
    /// One per directory of `.ts` or `.js` sources.
    javascript: Coverage,
    /// One per JVM source root. Java and Scala packages are routinely
    /// mutually recursive, so the root owns the tree below it.
    jvm: Coverage,
}

impl RepoCoverage {
    fn totals(&self) -> Coverage {
        [self.rust, self.go, self.python, self.javascript, self.jvm]
            .into_iter()
            .fold(Coverage::default(), |mut total, language| {
                total.units += language.units;
                total.with_build_files += language.with_build_files;
                total
            })
    }

    fn json(&self) -> Value {
        let totals = self.totals();
        let mut by_language = serde_json::Map::new();
        for (name, coverage) in [
            ("Rust", self.rust),
            ("Go", self.go),
            ("Python", self.python),
            ("JavaScript or TypeScript", self.javascript),
            ("Java or Scala", self.jvm),
        ] {
            if coverage.units > 0 {
                by_language.insert(name.to_owned(), coverage.json());
            }
        }
        json!({
            "build_units": totals.units,
            "with_build_files": totals.with_build_files,
            "complete": totals.complete(),
            "by_language": Value::Object(by_language),
        })
    }
}

const JVM_EXTENSIONS: &[&str] = &["java", "scala"];
const JAVASCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mts", "cts"];

fn repo_coverage(root: &Path) -> RepoCoverage {
    struct Scope {
        in_cargo_package: bool,
        in_jvm_root: bool,
    }

    fn visit(dir: &Path, coverage: &mut RepoCoverage, scope: Scope, depth: usize) {
        if depth > 12 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        let mut has_build_file = false;
        let mut has_cargo_manifest = false;
        let mut extensions: Vec<String> = Vec::new();
        let mut children = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            match name.to_str() {
                Some("BUILD" | "BUILD.bazel") => has_build_file = true,
                Some("Cargo.toml") => has_cargo_manifest = true,
                _ => {}
            }
            if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
                extensions.push(extension.to_owned());
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir())
                && !matches!(
                    name.to_str(),
                    Some(".git" | "node_modules" | "vendor" | "target" | "third_party")
                )
                && !name.to_string_lossy().starts_with("bazel-")
                && !name.to_string_lossy().starts_with('.')
            {
                children.push(path);
            }
        }
        let holds = |wanted: &[&str]| {
            extensions
                .iter()
                .any(|extension| wanted.contains(&extension.as_str()))
        };

        // A virtual workspace manifest declares no package and owns no sources.
        let cargo_package = has_cargo_manifest && declares_cargo_package(dir);
        if cargo_package {
            coverage.rust.count(has_build_file);
        } else if !scope.in_cargo_package && holds(&["rs"]) {
            coverage.rust.count(has_build_file);
        }
        if holds(&["go"]) {
            coverage.go.count(has_build_file);
        }
        if holds(&["py"]) {
            coverage.python.count(has_build_file);
        }
        if holds(JAVASCRIPT_EXTENSIONS) {
            coverage.javascript.count(has_build_file);
        }
        let jvm_root = !scope.in_jvm_root && holds(JVM_EXTENSIONS);
        if jvm_root {
            coverage.jvm.count(has_build_file);
        }

        for child in children {
            visit(
                &child,
                coverage,
                Scope {
                    in_cargo_package: scope.in_cargo_package || cargo_package,
                    in_jvm_root: scope.in_jvm_root || jvm_root,
                },
                depth + 1,
            );
        }
    }

    let mut coverage = RepoCoverage::default();
    visit(
        root,
        &mut coverage,
        Scope {
            in_cargo_package: false,
            in_jvm_root: false,
        },
        0,
    );
    coverage
}

fn declares_cargo_package(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|contents| {
        contents
            .lines()
            .any(|line| line.trim_start().starts_with("[package]"))
    })
}

fn uses_legacy_rust_rules(root: &Path) -> bool {
    bazel_configuration(root).any(|contents| {
        contents.contains("bazel_dep(name = \"rules_rust\"")
            || contents.contains("name = \"rules_rust\"")
            || contents.contains("io_bazel_rules_rust")
            || contents.contains("crate_universe(")
    })
}

fn uses_legacy_javascript_rules(root: &Path) -> bool {
    bazel_configuration(root).any(|contents| {
        contents.contains("build_bazel_rules_nodejs")
            || contents.contains("@build_bazel_rules_nodejs")
            || (contents.contains("@npm//") && !contents.contains("aspect_rules_js"))
            || contents.contains("@bazel/typescript")
    })
}

fn bazel_configuration(root: &Path) -> impl Iterator<Item = String> + '_ {
    ["MODULE.bazel", "WORKSPACE", "WORKSPACE.bazel"]
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(root.join(path)).ok())
}

fn prompt(id: &str) -> Option<&'static Prompt> {
    PROMPTS.iter().find(|prompt| prompt.id == id)
}

fn render_prompt(prompt: &Prompt) -> String {
    prompt
        .preambles
        .iter()
        .copied()
        .chain(std::iter::once(prompt.prompt))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_without_bazel() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();

        assert_eq!(detected_prompts(root.path())[0].id, "bazelify-rust");
    }

    /// A Cargo package owns its whole `src/` tree from the package root, so a
    /// migrated package is one covered unit — not one covered directory and
    /// two uncovered ones, which no correct migration could ever satisfy.
    #[test]
    fn a_cargo_package_is_one_coverage_unit() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("MODULE.bazel"), "bazel_dep()\n").unwrap();
        std::fs::create_dir_all(root.path().join("crates/matcher/src/net")).unwrap();
        std::fs::write(
            root.path().join("crates/matcher/Cargo.toml"),
            "[package]\nname = \"matcher\"\n",
        )
        .unwrap();
        std::fs::write(root.path().join("crates/matcher/BUILD.bazel"), "").unwrap();
        std::fs::write(root.path().join("crates/matcher/src/lib.rs"), "").unwrap();
        std::fs::write(root.path().join("crates/matcher/src/net/tcp.rs"), "").unwrap();

        let coverage = repo_coverage(root.path()).rust;
        assert_eq!((coverage.units, coverage.with_build_files), (1, 1));
        assert!(coverage.complete());
    }

    /// A virtual workspace manifest declares no package and compiles nothing.
    #[test]
    fn a_virtual_workspace_root_is_not_a_coverage_unit() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"api\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.path().join("api/src")).unwrap();
        std::fs::write(
            root.path().join("api/Cargo.toml"),
            "[package]\nname = \"api\"\n",
        )
        .unwrap();
        std::fs::write(root.path().join("api/src/lib.rs"), "").unwrap();

        assert_eq!(repo_coverage(root.path()).rust.units, 1);
    }

    /// Naming `rules_rs` in `MODULE.bazel` proves one package was migrated, not
    /// the workspace. The prompt has to survive a partial migration.
    #[test]
    fn a_partly_migrated_workspace_keeps_its_migration_prompt() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"api\", \"cli\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "bazel_dep(name = \"rules_rs\", version = \"0.0.102\")\n",
        )
        .unwrap();
        for (package, build_file) in [("api", true), ("cli", false)] {
            std::fs::create_dir_all(root.path().join(package).join("src")).unwrap();
            std::fs::write(
                root.path().join(package).join("Cargo.toml"),
                format!("[package]\nname = \"{package}\"\n"),
            )
            .unwrap();
            std::fs::write(root.path().join(package).join("src/lib.rs"), "").unwrap();
            if build_file {
                std::fs::write(root.path().join(package).join("BUILD.bazel"), "").unwrap();
            }
        }

        assert!(
            detected_prompts(root.path())
                .iter()
                .any(|prompt| prompt.id == "bazelify-rust")
        );
        assert_eq!(
            recommendation_reason("bazelify-rust", root.path()),
            "Bazel models 1 of 2 Cargo packages; finish the remaining ones before treating the migration as done."
        );
    }

    #[test]
    fn a_fully_migrated_workspace_drops_its_migration_prompt() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"api\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "bazel_dep(name = \"rules_rs\", version = \"0.0.102\")\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.path().join("api/src")).unwrap();
        std::fs::write(
            root.path().join("api/Cargo.toml"),
            "[package]\nname = \"api\"\n",
        )
        .unwrap();
        std::fs::write(root.path().join("api/src/lib.rs"), "").unwrap();
        std::fs::write(root.path().join("api/BUILD.bazel"), "").unwrap();

        assert!(
            !detected_prompts(root.path())
                .iter()
                .any(|prompt| prompt.id == "bazelify-rust")
        );
    }

    /// Go is the opposite case: Gazelle writes one BUILD file per directory of
    /// `.go` files, so each of those directories is its own unit.
    #[test]
    fn go_counts_one_coverage_unit_per_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("go.mod"), "module example.com/demo\n").unwrap();
        std::fs::write(root.path().join("main.go"), "package main\n").unwrap();
        std::fs::write(root.path().join("BUILD.bazel"), "").unwrap();
        std::fs::create_dir_all(root.path().join("internal/store")).unwrap();
        std::fs::write(
            root.path().join("internal/store/store.go"),
            "package store\n",
        )
        .unwrap();

        let coverage = repo_coverage(root.path()).go;
        assert_eq!((coverage.units, coverage.with_build_files), (2, 1));
    }

    #[test]
    fn detects_go_without_bazel() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("go.mod"), "module example.com/demo\n").unwrap();

        assert_eq!(detected_prompts(root.path())[0].id, "bazelify-go");
    }

    #[test]
    fn detects_nested_proto_sources_alongside_go() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("go.mod"), "module example.com/demo\n").unwrap();
        std::fs::create_dir_all(root.path().join("api/v1")).unwrap();
        std::fs::write(
            root.path().join("api/v1/demo.proto"),
            "syntax = \"proto3\";\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["bazelify-go", "bazelify-protos"]
        );
        assert!(evidence(root.path()).contains(&"*.proto".to_owned()));
    }

    #[test]
    fn detects_nested_dockerfiles() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("deploy/service")).unwrap();
        std::fs::write(
            root.path().join("deploy/service/Dockerfile"),
            "FROM scratch\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["bazelify-containers"]
        );
        assert!(evidence(root.path()).contains(&"Dockerfile".to_owned()));
    }

    #[test]
    fn detects_rules_oci_without_a_dockerfile() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "module(name = \"demo\")\nbazel_dep(name = \"rules_oci\", version = \"2.0.0\")\n",
        )
        .unwrap();

        assert_eq!(
            recommended_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["upgrade-oci-rules", "configure-remote-bazel"]
        );
        assert_eq!(
            candidate_markers("upgrade-oci-rules", root.path()),
            ["rules_oci"]
        );
    }

    #[test]
    fn detects_javascript_without_bazel() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("web")).unwrap();
        std::fs::write(
            root.path().join("web/package.json"),
            "{\"private\": true}\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["bazelify-javascript"]
        );
    }

    #[test]
    fn does_not_recommend_a_generic_java_prompt() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("app/src/main/java/example")).unwrap();
        std::fs::write(root.path().join("pom.xml"), "<project />\n").unwrap();
        std::fs::write(
            root.path().join("app/src/main/java/example/App.java"),
            "package example;\n",
        )
        .unwrap();

        assert!(detected_prompts(root.path()).is_empty());
    }

    #[test]
    fn detects_scala_without_a_java_prompt() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/main/scala/example")).unwrap();
        std::fs::write(
            root.path().join("build.sbt"),
            "scalaVersion := \"2.13.18\"\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("src/main/scala/example/App.scala"),
            "package example\n",
        )
        .unwrap();
        std::fs::write(root.path().join("build.gradle"), "plugins {}\n").unwrap();

        assert_eq!(
            recommended_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["bazelify-scala"]
        );
        assert_eq!(detected_build_systems(root.path()), ["sbt", "Gradle"]);
    }

    #[test]
    fn gradle_and_maven_are_recognised_build_systems() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/main/java")).unwrap();
        std::fs::write(root.path().join("src/main/java/App.java"), "class App {}\n").unwrap();
        std::fs::write(root.path().join("build.gradle"), "plugins {}\n").unwrap();
        std::fs::write(root.path().join("settings.gradle"), "\n").unwrap();
        std::fs::write(root.path().join("gradlew"), "#!/bin/sh\n").unwrap();

        assert_eq!(detected_build_systems(root.path()), ["Gradle"]);
        let markers = evidence(root.path());
        assert!(markers.contains(&"build.gradle".to_owned()));
        assert!(markers.contains(&"gradlew".to_owned()));
        assert!(!markers.contains(&"Java source or Maven/Gradle build".to_owned()));
    }

    #[test]
    fn because_names_only_the_markers_that_selected_the_candidate() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.path().join("pyproject.toml"), "[project]\n").unwrap();
        std::fs::write(root.path().join("Dockerfile"), "FROM scratch\n").unwrap();

        let doc = detected_json(root.path());
        let candidates = doc["candidates"].as_array().unwrap();
        let by_id = |id: &str| {
            candidates
                .iter()
                .find(|candidate| candidate["id"] == id)
                .unwrap()["because"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(by_id("bazelify-rust"), ["Cargo.toml"]);
        assert_eq!(by_id("bazelify-python"), ["pyproject.toml"]);
        assert_eq!(by_id("bazelify-containers"), ["Dockerfile"]);
    }

    #[test]
    fn a_bazel_root_does_not_hide_an_unmodelled_language() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.path().join("go.mod"), "module example.com/demo\n").unwrap();
        // Bazel models Rust only; the Go module is still unmigrated. The BUILD
        // file is what makes the Cargo package modelled — naming `rules_rs` in
        // `MODULE.bazel` on its own no longer counts.
        std::fs::write(root.path().join("BUILD.bazel"), "").unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "module(name = \"demo\")\nbazel_dep(name = \"rules_rs\", version = \"0.0.102\")\n",
        )
        .unwrap();

        assert_eq!(
            recommended_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["bazelify-go", "configure-remote-bazel"]
        );
    }

    #[test]
    fn sbt_2_detection_includes_remote_configuration_with_a_reason() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("project")).unwrap();
        std::fs::write(root.path().join("build.sbt"), "scalaVersion := \"3.8.4\"\n").unwrap();
        std::fs::write(
            root.path().join("project/build.properties"),
            "sbt.version=2.0.1\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["bazelify-scala", "configure-remote-sbt"]
        );
        assert!(render_detection(root.path()).contains("configure-remote-sbt"));
        assert!(evidence(root.path()).contains(&"sbt.version=2.x".to_owned()));
    }

    #[test]
    fn bazel_detection_recommends_javascript_and_remote_setup() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("package.json"), "{\"private\": true}\n").unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "module(name = \"demo\")\nbazel_dep(name = \"build_bazel_rules_nodejs\", version = \"6.0.0\")\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["bazelify-javascript", "configure-remote-bazel"]
        );
    }

    #[test]
    fn invocation_root_prefers_the_bazel_workspace_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let root = invocation_root_from(
            Path::new("/a/current/directory"),
            Some(workspace.path().as_os_str().to_os_string()),
        );

        assert_eq!(root, workspace.path());
    }

    #[test]
    fn bazel_detection_composes_python_upgrade_and_remote_setup() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "module(name = \"demo\")\nbazel_dep(name = \"rules_python\", version = \"1.0.0\")\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("pyproject.toml"),
            "[project]\nname = \"demo\"\n",
        )
        .unwrap();

        // Bazel models Python only, so the Cargo package here is still
        // unmigrated and keeps its bazelify prompt.
        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            [
                "bazelify-rust",
                "upgrade-python-rules",
                "configure-remote-bazel"
            ]
        );
    }

    #[test]
    fn established_go_rules_need_no_default_prompt() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("go.mod"), "module example.com/demo\n").unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "module(name = \"demo\")\nbazel_dep(name = \"rules_go\", version = \"0.50.0\")\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["configure-remote-bazel"]
        );
    }

    #[test]
    fn bazel_detection_composes_rust_upgrade_and_remote_setup() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "module(name = \"demo\")\nbazel_dep(name = \"rules_rust\", version = \"0.51.0\")\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["upgrade-rust-rules", "configure-remote-bazel"]
        );
    }

    #[test]
    fn rules_rs_does_not_trigger_the_legacy_rust_migration() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        // The package is modelled, so `bazelify-rust` is not expected either.
        std::fs::write(root.path().join("BUILD.bazel"), "").unwrap();
        std::fs::write(
            root.path().join("MODULE.bazel"),
            "module(name = \"demo\")\nbazel_dep(name = \"rules_rs\", version = \"0.0.102\")\n",
        )
        .unwrap();

        assert_eq!(
            detected_prompts(root.path())
                .into_iter()
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>(),
            ["configure-remote-bazel"]
        );
    }

    #[test]
    fn a_shown_prompt_does_not_depend_on_detection() {
        let root = tempfile::tempdir().unwrap();

        let doc = selected_json(prompt("bazelify-rust").unwrap(), root.path());
        assert_eq!(doc["prompt_id"], "bazelify-rust");
        assert!(doc["prompt"].as_str().unwrap().contains("rules_rs"));
        assert!(
            doc["prompt"]
                .as_str()
                .unwrap()
                .contains("# Bazel foundations")
        );
        assert!(
            doc["prompt"]
                .as_str()
                .unwrap()
                .contains("# rules_rs Paved Path")
        );
    }

    #[test]
    fn python_prompts_include_the_foundations_and_rules_py_recipe() {
        let rendered = render_prompt(prompt("bazelify-python").unwrap());

        assert!(rendered.contains("# Bazel foundations"));
        assert!(rendered.contains("aspect_rules_py\", version = \"2.0.0-alpha.4\""));
        assert!(rendered.contains("# Bazel-ify this Python repository"));
    }

    #[test]
    fn javascript_prompts_include_the_foundations_and_rules_js_recipe() {
        let rendered = render_prompt(prompt("bazelify-javascript").unwrap());

        assert!(rendered.contains("# Bazel foundations"));
        assert!(rendered.contains("aspect_rules_js\", version = \"3.0.1\""));
        assert!(rendered.contains("# Bazel-ify this JavaScript or TypeScript repository"));
    }

    #[test]
    fn scala_prompt_includes_its_rules_recipe() {
        let scala = render_prompt(prompt("bazelify-scala").unwrap());
        assert!(scala.contains("# Bazel foundations"));
        assert!(scala.contains("rules_scala\", version = \"7.2.6\""));
        assert!(scala.contains("# Bazel-ify this Scala repository"));
    }

    #[test]
    fn upgrade_prompts_include_their_ruleset_recipe() {
        assert!(
            render_prompt(prompt("upgrade-rust-rules").unwrap()).contains("# rules_rs Paved Path")
        );
        let oci = render_prompt(prompt("upgrade-oci-rules").unwrap());
        assert!(oci.contains("# Bazel foundations"));
        assert!(oci.contains("# Migrate these Bazel images from rules_oci to rules_img"));
    }

    #[test]
    fn no_match_explains_how_to_discover_and_select_prompts() {
        let rendered = render_no_match(&[]);
        assert!(rendered.contains("aspect prompts list"));
        assert!(rendered.contains("aspect prompts show <PROMPT>"));
        assert!(rendered.contains("coding agent"));
    }

    #[test]
    fn catalog_is_human_readable_and_discoverable() {
        let rendered = render_catalog();
        assert!(rendered.contains("bazelify-rust"));
        assert!(rendered.contains("upgrade-oci-rules"));
        assert!(rendered.contains("aspect prompts show <PROMPT>"));
    }

    #[test]
    fn detection_reports_candidates_without_printing_prompt_bodies() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();

        let rendered = render_detection(root.path());
        assert!(rendered.contains("bazelify-rust"));
        assert!(rendered.contains("aspect prompts show <PROMPT>"));
        assert!(!rendered.contains("# Bazel-ify this Rust repository"));
    }

    #[test]
    fn prompts_requires_a_subcommand_and_show_requires_a_prompt() {
        assert!(command().try_get_matches_from(["aspect prompts"]).is_err());
        assert!(
            command()
                .try_get_matches_from(["aspect prompts", "show"])
                .is_err()
        );
    }
}
