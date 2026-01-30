use anyhow::Result;
use markdown::mdast;
use mdast_util_to_markdown::to_markdown;
use regex::Regex;
use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

const PREDULE: &'static str = r#"<pre class="language-python"><code>"#;
const POSTDULE: &'static str = r#"</code></pre>"#;

pub fn highlight(md: &String) -> Result<String> {
    let mut md = markdown::to_mdast(md.as_str(), &markdown::ParseOptions::default()).unwrap();

    fn traverse(nodes: &mut Vec<mdast::Node>) {
        // Match link markers: @link@ /path @@ Name @link@
        // The markers are wrapped in quotes in the source so Python syntax highlighting
        // treats them as string literals. The regex matches just the marker content,
        // since the quotes get separated by HTML span tags during highlighting.
        let link_regex = Regex::new(r"@link@ ([\w/]+) @@ ([\w\.]+) @link@").unwrap();

        // After replacing link markers, we need to remove the quote spans that surround
        // the anchor tags. The pattern is:
        // <span class="...string..."><span class="...string..."><span class="...begin...">&#39;</span></span></span>
        // ...content (now an <a> tag)...
        // <span class="...end...">&#39;</span></span></span>
        let opening_quote_regex = Regex::new(
            r#"<span class="[^"]*string[^"]*"><span class="[^"]*string[^"]*"><span class="[^"]*begin[^"]*">&#39;</span></span></span><span class="[^"]*string[^"]*"><span class="[^"]*string[^"]*">"#
        ).unwrap();
        let closing_quote_regex =
            Regex::new(r#"<span class="[^"]*end[^"]*">&#39;</span></span></span>"#).unwrap();

        nodes.iter_mut().for_each(|node| match node {
            mdast::Node::Html(html) => {
                let html_raw = html.value.clone();
                if html_raw.starts_with(PREDULE) && html_raw.ends_with(POSTDULE) {
                    let strip_down = html_raw
                        .strip_prefix(PREDULE)
                        .unwrap()
                        .strip_suffix(POSTDULE)
                        .unwrap();

                    let syntax_set = SyntaxSet::load_defaults_newlines();
                    let syntax_starlark = syntax_set.find_syntax_by_extension("py").unwrap();
                    let mut html_generator = ClassedHTMLGenerator::new_with_class_style(
                        syntax_starlark,
                        &syntax_set,
                        ClassStyle::Spaced,
                    );
                    for line in LinesWithEndings::from(strip_down) {
                        html_generator
                            .parse_html_for_line_which_includes_newline(line)
                            .unwrap();
                    }
                    let out = html_generator.finalize();

                    // Replace link markers with anchor tags
                    let out = link_regex.replace_all(out.as_str(), r#"<a href="$1">$2</a>"#);

                    // Remove surrounding quote spans from anchor tags
                    let out = opening_quote_regex.replace_all(&out, "");
                    let out = closing_quote_regex.replace_all(&out, "");

                    html.value = format!("{}{}{}", PREDULE, out, POSTDULE);
                }
            }
            mdast::Node::Heading(head) => traverse(&mut head.children),
            _ => {}
        });
    }

    traverse(md.children_mut().unwrap());

    Ok(md
        .children()
        .unwrap()
        .iter()
        .map(|node| to_markdown(node).unwrap())
        .collect::<Vec<String>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::highlight;

    #[test]
    fn test_syntax_highlight_with_links() -> anyhow::Result<()> {
        let out = highlight(
            &r#"
## task

<pre class="language-python"><code>def task(
    *,
    name: '@link@ /lib/str @@ str @link@' = ...,
    implementation: typing.Callable[['@link@ /lib/task_context @@ TaskContext @link@'], None],
    args: dict['@link@ /lib/str @@ str @link@', '@link@ /lib/task_arg @@ task_arg @link@'],
    description: '@link@ /lib/str @@ str @link@' = ...,
) -> Task</code></pre>

Task type representing a Task.

```python
def _task_impl(ctx):
    pass

build =
    name = "build",
    impl = _task_impl,
    task_args = {
        "target": args.string(),
    }
)
```

---
            "#
            .to_string(),
        )?;

        // Verify the output contains expected highlighted content with links
        assert!(out.contains(r#"<a href="/lib/str">str</a>"#));
        assert!(out.contains(r#"<a href="/lib/task_context">TaskContext</a>"#));
        assert!(out.contains(r#"<a href="/lib/task_arg">task_arg</a>"#));

        Ok(())
    }
}
