mod highlight;
mod renderer;
mod traversal;
mod type_linker;
mod type_registry;

use anyhow::{Ok, Result};
use axl_runtime::eval;
use std::fs;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    println!("generating docs");

    let doc_module = eval::get_globals().build().documentation();

    // Phase 1: Traverse and build registry (handles path normalization and deduplication)
    let result = traversal::traverse(&doc_module, "lib");

    // Phase 2: Create linker and renderer
    let linker = type_linker::TypeLinker::new(&result.registry);
    let renderer = renderer::Renderer::new(&linker);

    // Phase 3: Render all pages and sort by path
    let mut pages: Vec<(String, String)> = result
        .pages
        .iter()
        .map(|(path, page)| (path.clone(), renderer.render_page(page)))
        .collect();
    pages.sort_by(|(ka, _), (kb, _)| ka.cmp(kb));

    // Phase 4: Write files with highlighting
    let _ = fs::remove_dir_all("../../docs/lib"); // Ignore error if directory doesn't exist

    for (path, content) in pages {
        let p = PathBuf::from(format!("../../docs/{path}.md"));
        if !p.parent().unwrap().exists() {
            fs::create_dir_all(p.parent().unwrap())?;
        }
        // Remove the first # in the markdown.
        let mut value = highlight::highlight(&content)?;

        let value = value.split_off(value.find('\n').unwrap());
        eprintln!("docs/{path}.md");
        std::fs::write(format!("../../docs/{path}.md"), value).unwrap()
    }
    Ok(())
}
