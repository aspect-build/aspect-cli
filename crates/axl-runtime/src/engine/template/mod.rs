mod handlebars;
// mod jinja2;
// mod liquid;

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::{NoSerialize, ProvidesStaticType, StarlarkValue, Value, starlark_value};
use starlark::values::StringValue;
use serde_json::{Map, Value as JsonValue};

use crate::engine::template::handlebars::handlebars_render;
// use crate::engine::template::jinja2::jinja2_render;
// use crate::engine::template::liquid::liquid_render;

use liquid::ParserBuilder as LiquidParserBuilder;
use liquid_core::model::{Value as LiquidValue, KString, Object as LiquidObject};
use minijinja::{Environment as MinijinjaEnvironment, value::Value as MinijinjaValue};

pub(super) fn jinja2_render(template: &str, data: &JsonValue) -> anyhow::Result<String> {
    let mut env = MinijinjaEnvironment::new();
    env.add_template("template", template).map_err(|e| anyhow::anyhow!(e))?;
    let tmpl = env.get_template("template").map_err(|e| anyhow::anyhow!(e))?;
    let ctx = MinijinjaValue::from_serialize(data);
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
            },
            JsonValue::String(s) => LiquidValue::scalar(s.to_string()),
            JsonValue::Array(arr) => {
                LiquidValue::array(arr.iter().map(json_to_liquid).collect::<Vec<_>>())
            },
            JsonValue::Object(obj) => {
                let mut liquid_obj = LiquidObject::new();
                for (k, v) in obj.iter() {
                    liquid_obj.insert(KString::from_ref(k.as_str()), json_to_liquid(v));
                }
                LiquidValue::Object(liquid_obj)
            }
        }
    }

    let parser = LiquidParserBuilder::with_stdlib().build().map_err(|e| anyhow::anyhow!(e))?;
    let template = parser.parse(template).map_err(|e| anyhow::anyhow!(e))?;
    let globals = if let LiquidValue::Object(obj) = json_to_liquid(data) {
        obj
    } else {
        return Err(anyhow::anyhow!("Data is not an object"));
    };
    template.render(&globals).map_err(|e| anyhow::anyhow!(e))
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<template>")]
pub struct Template {}

impl Template {
    pub fn new() -> Self {
        Self {}
    }
}

starlark_simple_value!(Template);

#[starlark_value(type = "template")]
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
    /// # Parameters
    /// - `template`: The Handlebars template string.
    /// - `data`: A dictionary of data to render the template with.
    ///
    /// # Returns
    /// The rendered template as a string.
    ///
    /// # Example
    /// ```starlark
    /// result = ctx.template.handlebars("Hello, {{name}}!", {"name": "World"})
    /// ```
    fn handlebars<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] template: StringValue<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())] data: UnpackDictEntries<String, Value<'v>>,
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
    /// # Parameters
    /// - `template`: The Jinja2 template string.
    /// - `data`: A dictionary of data to render the template with.
    ///
    /// # Returns
    /// The rendered template as a string.
    ///
    /// # Example
    /// ```starlark
    /// result = ctx.template.jinja2("Hello, {{ name }}!", {"name": "World"})
    /// ```
    fn jinja2<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] template: StringValue<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())] data: UnpackDictEntries<String, Value<'v>>,
    ) -> anyhow::Result<String> {
        let mut json_map: Map<String, JsonValue> = Map::new();
        for (k, v) in data.entries {
            json_map.insert(k, v.to_json_value()?);
        }
        let json_data = JsonValue::Object(json_map);
        jinja2_render(template.as_str(), &json_data)
    }

    /// Renders a Liquid template with the provided data.
    ///
    /// # Parameters
    /// - `template`: The Liquid template string.
    /// - `data`: A dictionary of data to render the template with.
    ///
    /// # Returns
    /// The rendered template as a string.
    ///
    /// # Example
    /// ```starlark
    /// result = ctx.template.liquid("Hello, {{ name }}!", {"name": "World"})
    /// ```
    fn liquid<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] template: StringValue<'v>,
        #[starlark(require = named, default = UnpackDictEntries::default())] data: UnpackDictEntries<String, Value<'v>>,
    ) -> anyhow::Result<String> {
        let mut json_map: Map<String, JsonValue> = Map::new();
        for (k, v) in data.entries {
            json_map.insert(k, v.to_json_value()?);
        }
        let json_data = JsonValue::Object(json_map);
        liquid_render(template.as_str(), &json_data)
    }
}