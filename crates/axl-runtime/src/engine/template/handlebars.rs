use handlebars::Handlebars;

use serde_json::Value as JsonValue;

pub(super) fn handlebars_render(template: &str, data: &JsonValue) -> anyhow::Result<String> {
    let hb = Handlebars::new();
    let rendered = hb.render_template(template, data)?;
    Ok(rendered)
}
