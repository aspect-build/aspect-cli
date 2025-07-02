use liquid::ParserBuilder as LiquidParserBuilder;
use liquid_core::model::{Value as LiquidValue, KString, Object as LiquidObject};

use serde_json::Value as JsonValue;

pub(super) fn liquid_render(template: &str, data: &JsonValue) -> anyhow::Result<String> {
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
            JsonValue::String(s) => LiquidValue::scalar(s.as_str()),
            JsonValue::Array(arr) => {
                LiquidValue::array(arr.iter().map(json_to_liquid).collect::<Vec<_>>())
            },
            JsonValue::Object(obj) => {
                let mut liquid_obj = LiquidObject::new();
                for (k, v) in obj.iter() {
                    liquid_obj.insert(KString::from_ref(k), json_to_liquid(v));
                }
                LiquidValue::object(liquid_obj)
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