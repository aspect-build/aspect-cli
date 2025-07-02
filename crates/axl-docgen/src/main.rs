mod highlight;
use anyhow::{Ok, Result};
use axl_runtime::eval;
use starlark::docs::multipage::{DocModuleInfo, render_markdown_multipage};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn snake(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_ascii_uppercase() {
            // Add an underscore before a new word, unless it's the very first character
            if !result.is_empty() && result.chars().last().unwrap() != '_' {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("Generating docs");

    let task = eval::get_globals().build().documentation();

    let modules_info = DocModuleInfo {
        module: &task,
        name: "lib".to_owned(),
        page_path: "lib".to_owned(),
    };

    // Normalize index modules to be at the root.
    // Eg if a path is /lib/std/std then it should be just /lib/std
    // since we don't want to generate the index pages.
    fn normalize_path(path: &str) -> String {
        let paths = path.split("/").map(snake).collect::<Vec<String>>();
        if path != "lib" && paths.len() > 2 && paths.last() == paths.get(paths.len() - 2) {
            paths[0..paths.len() - 1].join("/")
        } else {
            paths.join("/")
        }
    }

    fn deduplicate_keeping_with_more_docs<'v>(
        tuples: Vec<(&'v String, &'v String)>,
    ) -> Vec<(String, &'v String)> {
        let mut map: HashMap<String, &String> = HashMap::new();

        // Insert tuples, keeping the last value for duplicate keys
        for (key, value) in tuples {
            let k = normalize_path(key.as_str());
            if let Some(oldval) = map.get(k.as_str()) {
                if oldval.len() < value.len() {
                    map.insert(k, value);
                }
            } else {
                map.insert(k, value);
            }
        }

        // Convert HashMap back to Vec
        map.into_iter().collect()
    }

    fn linked_ty_mapper(path: &str, type_name: &str) -> String {
        let path = normalize_path(path);
        format!(r#"'@link@ /{path} @@ {type_name} @link@'"#)
    }
    let res = render_markdown_multipage(vec![modules_info], Some(linked_ty_mapper));

    let res = res
        .iter()
        .map(|(k, v)| (k, v))
        .collect::<Vec<(&String, &String)>>();
    let mut res = deduplicate_keeping_with_more_docs(res);
    res.sort_by(|(ka, _), (kb, _)| ka.cmp(kb));

    fs::remove_dir_all("../../docs/lib")?;

    for (k, v) in res {
        let p = PathBuf::from(format!("../../docs/{k}.md"));
        if !p.parent().unwrap().exists() {
            fs::create_dir_all(p.parent().unwrap())?;
        }
        // Remove the first # in the markdown.
        let mut value = highlight::highlight(v)?;

        let value = value.split_off(value.find("\n").unwrap());
        eprintln!("docs/{k}.md");
        std::fs::write(format!("../../docs/{k}.md"), value).unwrap()
    }
    Ok(())
}
