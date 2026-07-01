mod handlebars;
// mod jinja2;
// mod liquid;

use allocative::Allocative;
use derive_more::Display;
use serde_json::{Map, Value as JsonValue};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::StringValue;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::tuple::AllocTuple;
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value, starlark_value,
};

use crate::engine::template::handlebars::handlebars_render;
// use crate::engine::template::jinja2::jinja2_render;
// use crate::engine::template::liquid::liquid_render;

use liquid::ParserBuilder as LiquidParserBuilder;
use liquid_core::model::{KString, Object as LiquidObject, Value as LiquidValue};
use minijinja::{Environment as MinijinjaEnvironment, value::Value as MinijinjaValue};

/// Convert a `serde_json::Value` directly to a `minijinja::Value`, walking
/// the tree and building primitive minijinja values for each node.
///
/// We CANNOT use `MinijinjaValue::from_serialize(json_value)` here. The
/// `starlark-rust` we depend on enables `serde_json/arbitrary_precision`,
/// and cargo's feature unification activates that flag across the workspace.
/// With that feature on, `serde_json::Number::serialize` emits a tagged
/// newtype struct (`$serde_json::private::Number`) that only the serde_json
/// deserializer understands. Other serializers — minijinja included —
/// see an opaque struct, which surfaces in templates as a map (`{...}`)
/// instead of a number, breaking arithmetic like `{{ a + b }}` with
/// "tried to use + operator on unsupported types map and map".
///
/// Building the minijinja value directly sidesteps the tagged form.
fn json_to_minijinja(value: &JsonValue) -> MinijinjaValue {
    match value {
        JsonValue::Null => MinijinjaValue::from(()),
        JsonValue::Bool(b) => MinijinjaValue::from(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                MinijinjaValue::from(i)
            } else if let Some(u) = n.as_u64() {
                MinijinjaValue::from(u)
            } else if let Some(f) = n.as_f64() {
                MinijinjaValue::from(f)
            } else {
                // Number that doesn't fit i64/u64/f64 — surface the string form
                // so templates at least see something readable.
                MinijinjaValue::from(n.to_string())
            }
        }
        JsonValue::String(s) => MinijinjaValue::from(s.as_str()),
        JsonValue::Array(arr) => {
            MinijinjaValue::from(arr.iter().map(json_to_minijinja).collect::<Vec<_>>())
        }
        JsonValue::Object(obj) => {
            let entries: Vec<(String, MinijinjaValue)> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_minijinja(v)))
                .collect();
            MinijinjaValue::from_iter(entries)
        }
    }
}

pub(super) fn jinja2_render(template: &str, data: &JsonValue) -> anyhow::Result<String> {
    let mut env = MinijinjaEnvironment::new();
    env.add_template("template", template)
        .map_err(|e| anyhow::anyhow!(e))?;
    let tmpl = env
        .get_template("template")
        .map_err(|e| anyhow::anyhow!(e))?;
    let ctx = json_to_minijinja(data);
    tmpl.render(&ctx).map_err(|e| anyhow::anyhow!(e))
}

fn liquid_render(template: &str, data: &JsonValue) -> anyhow::Result<String> {
    fn json_to_liquid(json: &JsonValue) -> LiquidValue {
        match json {
            JsonValue::Null => LiquidValue::Nil,
            JsonValue::Bool(b) => LiquidValue::scalar(*b),
            JsonValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    LiquidValue::scalar(i)
                } else {
                    LiquidValue::scalar(n.as_f64().unwrap())
                }
            }
            JsonValue::String(s) => LiquidValue::scalar(s.to_string()),
            JsonValue::Array(arr) => {
                LiquidValue::array(arr.iter().map(json_to_liquid).collect::<Vec<_>>())
            }
            JsonValue::Object(obj) => {
                let mut liquid_obj = LiquidObject::new();
                for (k, v) in obj.iter() {
                    liquid_obj.insert(KString::from_ref(k.as_str()), json_to_liquid(v));
                }
                LiquidValue::Object(liquid_obj)
            }
        }
    }

    let parser = LiquidParserBuilder::with_stdlib()
        .build()
        .map_err(|e| anyhow::anyhow!(e))?;
    let template = parser.parse(template).map_err(|e| anyhow::anyhow!(e))?;
    let globals = if let LiquidValue::Object(obj) = json_to_liquid(data) {
        obj
    } else {
        return Err(anyhow::anyhow!("data is not an object"));
    };
    template.render(&globals).map_err(|e| anyhow::anyhow!(e))
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<Template>")]
pub struct Template {}

impl Template {
    pub fn new() -> Self {
        Self {}
    }
}

starlark_simple_value!(Template);

#[starlark_value(type = "Template")]
impl<'v> StarlarkValue<'v> for Template {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(template_methods)
    }
}

#[starlark_module]
pub(crate) fn template_methods(registry: &mut MethodsBuilder) {
    /// Renders a Handlebars template with the provided data.
    ///
    /// **Parameters**
    /// - `template`: The Handlebars template string.
    /// - `data`: A dictionary of data to render the template with.
    ///
    /// **Returns**
    /// The rendered template as a string.
    ///
    /// **Example**
    /// ```starlark
    /// result = ctx.template.handlebars("Hello, {{name}}!", {"name": "World"})
    /// ```
    fn handlebars<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] template: StringValue<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        data: UnpackDictEntries<String, Value<'v>>,
    ) -> anyhow::Result<String> {
        let mut json_map: Map<String, JsonValue> = Map::new();
        for (k, v) in data.entries {
            json_map.insert(k, v.to_json_value()?);
        }
        let json_data = JsonValue::Object(json_map);
        handlebars_render(template.as_str(), &json_data)
    }

    /// Renders a Jinja2 template with the provided data.
    ///
    /// **Parameters**
    /// - `template`: The Jinja2 template string.
    /// - `data`: A dictionary of data to render the template with.
    ///
    /// **Returns**
    /// The rendered template as a string.
    ///
    /// **Example**
    /// ```starlark
    /// result = ctx.template.jinja2("Hello, {{ name }}!", {"name": "World"})
    /// ```
    fn jinja2<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] template: StringValue<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        data: UnpackDictEntries<String, Value<'v>>,
    ) -> anyhow::Result<String> {
        let mut json_map: Map<String, JsonValue> = Map::new();
        for (k, v) in data.entries {
            json_map.insert(k, v.to_json_value()?);
        }
        let json_data = JsonValue::Object(json_map);
        jinja2_render(template.as_str(), &json_data)
    }

    /// Renders a Jinja2 template like `jinja2`, but never fails the evaluation:
    /// a render error is returned to the caller as a value instead of
    /// propagating and aborting the whole invocation.
    ///
    /// **Returns** a 2-tuple `(ok, text)`:
    /// - `(True, rendered)` on success.
    /// - `(False, error_message)` on any parse/render error.
    ///
    /// Callers use this where a template bug in one surface must degrade to a
    /// fallback body rather than crash the process (e.g. status-check
    /// renderers). Prefer `jinja2` for internal templates whose failure is a
    /// programming error that should surface loudly.
    ///
    /// **Example**
    /// ```starlark
    /// ok, text = ctx.template.try_jinja2(tmpl, data = d)
    /// if not ok:
    ///     _LOG.warn(ctx.std, "render failed: %s" % text)
    ///     text = fallback_body(text)
    /// ```
    fn try_jinja2<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] template: StringValue<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        data: UnpackDictEntries<String, Value<'v>>,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let mut json_map: Map<String, JsonValue> = Map::new();
        for (k, v) in data.entries {
            json_map.insert(k, v.to_json_value()?);
        }
        let json_data = JsonValue::Object(json_map);
        let (ok, text) = match jinja2_render(template.as_str(), &json_data) {
            Ok(rendered) => (true, rendered),
            Err(e) => (false, e.to_string()),
        };
        let items = [heap.alloc(ok), heap.alloc_str(&text).to_value()];
        Ok(heap.alloc(AllocTuple(items)))
    }

    /// Renders a Liquid template with the provided data.
    ///
    /// **Parameters**
    /// - `template`: The Liquid template string.
    /// - `data`: A dictionary of data to render the template with.
    ///
    /// **Returns**
    /// The rendered template as a string.
    ///
    /// **Example**
    /// ```starlark
    /// result = ctx.template.liquid("Hello, {{ name }}!", {"name": "World"})
    /// ```
    fn liquid<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] template: StringValue<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        data: UnpackDictEntries<String, Value<'v>>,
    ) -> anyhow::Result<String> {
        let mut json_map: Map<String, JsonValue> = Map::new();
        for (k, v) in data.entries {
            json_map.insert(k, v.to_json_value()?);
        }
        let json_data = JsonValue::Object(json_map);
        liquid_render(template.as_str(), &json_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jinja2_render_ok() {
        let data = JsonValue::Object(Map::from_iter([(
            "name".to_string(),
            JsonValue::String("World".to_string()),
        )]));
        assert_eq!(
            jinja2_render("Hello, {{ name }}!", &data).unwrap(),
            "Hello, World!"
        );
    }

    #[test]
    fn jinja2_render_undefined_call_errors() {
        // The failure `try_jinja2` is built to catch: calling a macro/function
        // that was never defined. Confirm it surfaces as Err (not a panic, not
        // a silent empty string) so the tuple mapping returns (False, msg).
        let err = jinja2_render("{{ not_a_real_macro(x) }}", &JsonValue::Object(Map::new()))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not_a_real_macro") || err.contains("unknown"),
            "error should name the undefined call: {err}"
        );
    }
}
