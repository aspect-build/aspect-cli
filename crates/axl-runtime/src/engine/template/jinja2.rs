use minijinja::{value::Value as MinijinjaValue, Environment as MinijinjaEnvironment};

use serde_json::Value as JsonValue;

pub(super) fn jinja2_render(template: &str, data: &JsonValue) -> anyhow::Result<String> {
    let mut env = MinijinjaEnvironment::new();
    env.add_template("template", template)
        .map_err(|e| anyhow::anyhow!(e))?;
    let tmpl = env
        .get_template("template")
        .map_err(|e| anyhow::anyhow!(e))?;
    let ctx = MinijinjaValue::from_serialize(data).map_err(|e| anyhow::anyhow!(e))?;
    tmpl.render(&ctx).map_err(|e| anyhow::anyhow!(e))
}
