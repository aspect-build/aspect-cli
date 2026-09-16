mod api_surface;
mod frontmatter;
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

    /// URL prefix prepended to every generated link (e.g. `/docs` for a site
    /// hosted at https://example.com/docs/). Trailing slashes are stripped.
    /// Defaults to empty, producing absolute paths like `/types/str`.
    #[arg(long, default_value = "")]
    base_url: String,

    /// Extra frontmatter line to emit on every page, verbatim, after `title`
    /// (e.g. `--frontmatter 'public: true'`). Repeatable.
    #[arg(long = "frontmatter", value_name = "LINE")]
    frontmatter: Vec<String>,

    /// Emit an `og:image` frontmatter line on every page from this template,
    /// with `{title}` replaced by the URL-encoded page title
    /// (e.g. `https://aspect.build/_og?title={title}&category=Docs`).
    #[arg(long, value_name = "URL")]
    og_image_url_template: Option<String>,

    /// Print the flat, signature-only public API surface to stdout and exit,
    /// instead of generating documentation pages. Consumed by the
    /// `axl-api-watch` CI job to snapshot the surface and alert on drift.
    #[arg(long)]
    api_surface: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let base_url = args.base_url.trim_end_matches('/').to_string();

    let documentation = docs::documentation()?;

    if args.api_surface {
        print!(
            "{}",
            api_surface::render_api_surface(&documentation.types, &documentation.builtins)
        );
        return Ok(());
    }

    let result = traversal::traverse_all(&documentation.types, &documentation.builtins);

    let linker = type_linker::TypeLinker::new(&result.registry, &base_url);
    let renderer = renderer::Renderer::new(&linker, &base_url);

    let pages: Vec<(String, String, String)> = result
        .pages
        .iter()
        .map(|(path, page)| (path.clone(), page.title.clone(), renderer.render_page(page)))
        .collect();

    for sub in ["types", "builtins"] {
        let _ = fs::remove_dir_all(args.output.join(sub));
    }

    // Pre-create parent directories sequentially so the parallel write phase is race-free.
    for (path, _, _) in &pages {
        let p = args.output.join(format!("{path}.md"));
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
    }

    // Highlight + write each page on the blocking thread pool. `highlight` is pure
    // CPU work (markdown parse + syntect tokenization + regex passes) and dominates
    // total runtime for large doc trees.
    // Frontmatter is prepended after highlighting so the markdown pass never
    // sees the `---` fences and cannot reformat them.
    let extra_frontmatter = std::sync::Arc::new(args.frontmatter.clone());
    let og_template = std::sync::Arc::new(args.og_image_url_template.clone());
    let mut set = tokio::task::JoinSet::new();
    for (path, title, content) in pages {
        let output = args.output.clone();
        let extra_frontmatter = extra_frontmatter.clone();
        let og_template = og_template.clone();
        set.spawn_blocking(move || -> Result<()> {
            let p = output.join(format!("{path}.md"));
            let mut value = frontmatter::render(&title, &extra_frontmatter, og_template.as_deref());
            value.push_str(&highlight::highlight(&content)?);
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
