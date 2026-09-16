//! YAML frontmatter prepended to every generated page.
//!
//! The block always carries the page `title`. Consumers add site-specific
//! lines with `--frontmatter` and an Open Graph image with
//! `--og-image-url-template`, whose `{title}` placeholder is replaced by the
//! URL-encoded title. Encoding matches Python's `urllib.parse.urlencode`
//! (`quote_plus`) so a site-side script that recomputes the same URL from the
//! frontmatter title sees no difference.

/// Render the frontmatter block, including the closing newline.
pub fn render(title: &str, extra_lines: &[String], og_image_url_template: Option<&str>) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("title: {}\n", yaml_quote(title)));
    for line in extra_lines {
        out.push_str(line.trim());
        out.push('\n');
    }
    if let Some(template) = og_image_url_template {
        let url = template.replace("{title}", &url_encode(title));
        out.push_str(&format!("og:image: {}\n", yaml_quote(&url)));
    }
    out.push_str("---\n\n");
    out
}

/// Double-quoted YAML scalar with the two characters that need escaping.
fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `application/x-www-form-urlencoded` encoding as Python's `quote_plus`
/// does it: alphanumerics and `_.-~` pass through, space becomes `+`, every
/// other byte becomes uppercase `%XX`.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_matches_python_quote_plus() {
        assert_eq!(
            url_encode("VideoAmp: CI/CD that just works, at monorepo scale"),
            "VideoAmp%3A+CI%2FCD+that+just+works%2C+at+monorepo+scale"
        );
        assert_eq!(
            url_encode("@rules_x//lib:foo.bzl"),
            "%40rules_x%2F%2Flib%3Afoo.bzl"
        );
        assert_eq!(url_encode("aspect.auth"), "aspect.auth");
        assert_eq!(url_encode("é"), "%C3%A9");
    }

    #[test]
    fn render_title_only() {
        assert_eq!(
            render("AuthSession", &[], None),
            "---\ntitle: \"AuthSession\"\n---\n\n"
        );
    }

    #[test]
    fn render_with_extra_lines_and_og_image() {
        let got = render(
            "aspect.auth",
            &["public: true".to_string()],
            Some("https://aspect.build/_og?title={title}&category=Docs"),
        );
        assert_eq!(
            got,
            "---\n\
             title: \"aspect.auth\"\n\
             public: true\n\
             og:image: \"https://aspect.build/_og?title=aspect.auth&category=Docs\"\n\
             ---\n\n"
        );
    }

    #[test]
    fn render_quotes_special_characters() {
        assert_eq!(
            render("say \"hi\"", &[], None),
            "---\ntitle: \"say \\\"hi\\\"\"\n---\n\n"
        );
    }
}
