use anyhow::{Result, bail};
use blazon::{Version, generate, jar_for_bazel};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "blazon",
    about = "Generate typed Rust bindings to Bazel's server classes, per Bazel version"
)]
struct Cli {
    /// Binding spec (TOML).
    #[arg(long, default_value = "bindings.toml")]
    spec: PathBuf,
    /// Directory for the generated modules.
    #[arg(long, default_value = "src/bindings")]
    out: PathBuf,
    /// Bazel version to bind, fetched into the cache if absent. Repeatable;
    /// defaults to the spec's `versions`.
    #[arg(long = "bazel")]
    bazel: Vec<String>,
    /// A server jar to bind instead of a release; needs --version.
    #[arg(long)]
    jar: Option<PathBuf>,
    /// Version label for --jar.
    #[arg(long)]
    version: Option<String>,
    /// Print every member of these classes (dotted names) from the first version and exit.
    #[arg(long = "dump")]
    dump: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut labels = cli.bazel.clone();
    if labels.is_empty() && cli.jar.is_none() {
        labels = blazon::spec::load(&cli.spec)?.versions;
    }
    let mut versions = Vec::new();
    for v in &labels {
        versions.push(Version {
            label: v.clone(),
            jar: jar_for_bazel(v)?,
        });
    }
    match (cli.jar, cli.version) {
        (Some(jar), Some(label)) => versions.push(Version { label, jar }),
        (Some(_), None) => bail!("--jar needs --version"),
        (None, Some(_)) => bail!("--version needs --jar"),
        (None, None) => {}
    }
    if versions.is_empty() {
        bail!(
            "nothing to bind: pass --bazel <version>, --jar <path> --version <label>, or `versions` in the spec"
        );
    }
    if !cli.dump.is_empty() {
        print!("{}", blazon::dump(&versions[0].jar, &cli.dump)?);
        return Ok(());
    }
    let report = generate(&cli.spec, &versions, &cli.out)?;
    if let Some(shape) = &report.shape {
        eprint!("{shape}");
    }
    let members: usize = report.classes.iter().map(|c| c.members.len()).sum();
    eprintln!(
        "blazon: {} classes, {members} members, {} version(s) into {}",
        report.classes.len(),
        report.versions.len(),
        cli.out.display()
    );
    Ok(())
}
