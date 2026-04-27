mod highlight;
mod renderer;
mod traversal;
mod type_linker;
mod type_registry;

use anyhow::Result;
use axl_runtime::docs;
use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "axl-docgen", about = "Generate AXL API documentation")]
struct Args {
    /// Output directory. Pages are written under <output>/types/... and
    /// <output>/builtins/... .
    #[arg(long, default_value = "docs")]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let documentation = docs::documentation()?;
    let result = traversal::traverse_all(&documentation.types, &documentation.builtins);

    let linker = type_linker::TypeLinker::new(&result.registry);
    let renderer = renderer::Renderer::new(&linker);

    let pages: Vec<(String, String)> = result
        .pages
        .iter()
        .map(|(path, page)| (path.clone(), renderer.render_page(page)))
        .collect();

    for sub in ["types", "builtins"] {
        let _ = fs::remove_dir_all(args.output.join(sub));
    }

    // Pre-create parent directories sequentially so the parallel write phase is race-free.
    for (path, _) in &pages {
        let p = args.output.join(format!("{path}.md"));
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
    }

    // Highlight + write each page on the blocking thread pool. `highlight` is pure
    // CPU work (markdown parse + syntect tokenization + regex passes) and dominates
    // total runtime for large doc trees.
    let mut set = tokio::task::JoinSet::new();
    for (path, content) in pages {
        let output = args.output.clone();
        set.spawn_blocking(move || -> Result<()> {
            let p = output.join(format!("{path}.md"));
            let value = highlight::highlight(&content)?;
            eprintln!("{}", p.display());
            fs::write(p, value)?;
            Ok(())
        });
    }
    while let Some(res) = set.join_next().await {
        res.map_err(|e| anyhow::anyhow!("join error: {e}"))??;
    }
    Ok(())
}
