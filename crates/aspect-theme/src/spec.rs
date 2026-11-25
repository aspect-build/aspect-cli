//! A role's value as a theme writes it, parsed but not resolved: references
//! stay names until [`crate::Theme`] follows them at a depth.

use serde_json::Value;

use crate::colour::{ansi_index, nearest16, parse_hex, rgb256};

/// One of the terminal's 16 colours, or `None` for its own (`"default"`).
pub(crate) type Ansi = Option<u8>;

/// A colour as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColourSpec {
    /// `"@role"`: that role's foreground, else its background.
    Ref(String),
    /// `"default"` or `"none"`: no colour of ours, the terminal's own.
    Default,
    /// An ANSI name: that colour at every depth.
    Ansi(u8),
    /// A 256-palette index.
    Index(u8),
    /// `#rrggbb`.
    Rgb([u8; 3]),
    /// `{"rgb", "256", "16", "ansi"}`: each depth as given, a missing one
    /// derived from the next richer one. `ansi` is the value when following
    /// the terminal, `16` when it is absent.
    Tiers {
        rgb: Option<[u8; 3]>,
        i256: Option<u8>,
        i16: Option<Ansi>,
        ansi: Option<Ansi>,
    },
}

/// An attribute: on, off, or on only where a capability holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Attr {
    #[default]
    Off,
    On,
    /// `"hyperlinks"`: on where the terminal is known to open OSC 8 links.
    Hyperlinks,
}

/// A role's value as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RoleSpec {
    /// `"@role"`: that role's whole style.
    Ref(String),
    /// `"none"`: no styling.
    None,
    Style(StyleSpec),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StyleSpec {
    pub fg: Option<ColourSpec>,
    pub bg: Option<ColourSpec>,
    pub bold: Attr,
    pub dim: Attr,
    pub italic: Attr,
    pub underline: Attr,
    pub reverse: Attr,
    /// Raw SGR parameters, used as they are.
    pub sgr: Option<String>,
    /// A whole style used instead at the 16-colour depth and when following
    /// the terminal.
    pub sixteen: Option<Box<StyleSpec>>,
}

const STYLE_KEYS: [&str; 9] = [
    "fg",
    "bg",
    "bold",
    "dim",
    "italic",
    "underline",
    "reverse",
    "sgr",
    "16",
];

/// Whether an object is a colour by depth rather than a style.
fn is_tiers(map: &serde_json::Map<String, Value>) -> bool {
    map.keys().any(|k| k == "rgb" || k == "256" || k == "ansi")
}

impl RoleSpec {
    pub(crate) fn parse(value: &Value) -> Result<RoleSpec, String> {
        match value {
            Value::String(s) if s.starts_with('@') => Ok(RoleSpec::Ref(reference(s)?)),
            Value::String(s) if s == "none" => Ok(RoleSpec::None),
            Value::Object(map) if !is_tiers(map) => Ok(RoleSpec::Style(StyleSpec::parse(value)?)),
            _ => Ok(RoleSpec::Style(StyleSpec {
                fg: Some(ColourSpec::parse(value)?),
                ..StyleSpec::default()
            })),
        }
    }

    /// This value as it is when the background is unknown: its `16` style if
    /// it has one, else its colours as the terminal's own.
    pub(crate) fn following_terminal(&self) -> RoleSpec {
        match self {
            RoleSpec::Style(style) => RoleSpec::Style(match &style.sixteen {
                Some(sixteen) => (**sixteen).clone(),
                None => StyleSpec {
                    fg: style.fg.as_ref().map(ColourSpec::following_terminal),
                    bg: style.bg.as_ref().map(ColourSpec::following_terminal),
                    ..style.clone()
                },
            }),
            other => other.clone(),
        }
    }
}

impl StyleSpec {
    fn parse(value: &Value) -> Result<StyleSpec, String> {
        let Value::Object(map) = value else {
            return Err(format!("a style is an object, got {value}"));
        };
        let mut style = StyleSpec::default();
        for (key, v) in map {
            let attr = || match v {
                Value::Bool(true) => Ok(Attr::On),
                Value::Bool(false) => Ok(Attr::Off),
                Value::String(s) if s == "hyperlinks" => Ok(Attr::Hyperlinks),
                _ => Err(format!(
                    "\"{key}\" must be true, false or \"hyperlinks\", got {v}"
                )),
            };
            match key.as_str() {
                "fg" => style.fg = Some(ColourSpec::parse(v)?),
                "bg" => style.bg = Some(ColourSpec::parse(v)?),
                "bold" => style.bold = attr()?,
                "dim" => style.dim = attr()?,
                "italic" => style.italic = attr()?,
                "underline" => style.underline = attr()?,
                "reverse" => style.reverse = attr()?,
                "sgr" => {
                    let s = v
                        .as_str()
                        .ok_or_else(|| format!("\"sgr\" must be a string, got {v}"))?;
                    if !s.bytes().all(|b| b.is_ascii_digit() || b == b';') {
                        return Err(format!(
                            "\"sgr\" takes SGR parameters such as \"1;38;5;75\", got {s:?}"
                        ));
                    }
                    style.sgr = Some(s.to_string());
                }
                "16" => style.sixteen = Some(Box::new(StyleSpec::parse(v)?)),
                other => {
                    return Err(format!(
                        "unknown style key {other:?}; a style takes {}",
                        STYLE_KEYS.join(", ")
                    ));
                }
            }
        }
        Ok(style)
    }
}

impl ColourSpec {
    pub(crate) fn parse(value: &Value) -> Result<ColourSpec, String> {
        match value {
            Value::String(s) if s.starts_with('@') => Ok(ColourSpec::Ref(reference(s)?)),
            Value::String(s) if s == "default" || s == "none" => Ok(ColourSpec::Default),
            Value::String(s) if s.starts_with('#') => parse_hex(s)
                .map(ColourSpec::Rgb)
                .ok_or_else(|| format!("{s:?} is not a #rrggbb or #rgb colour")),
            Value::String(s) => ansi_index(s).map(ColourSpec::Ansi).ok_or_else(|| {
                format!(
                    "{s:?} is not a colour: a #rrggbb hex, a 256-colour index, an ANSI \
                     name such as \"red\" or \"bright-black\", \"default\" or \"@role\""
                )
            }),
            Value::Number(_) => index(value).map(ColourSpec::Index),
            Value::Object(map) => {
                let (mut rgb, mut i256, mut i16, mut ansi) = (None, None, None, None);
                for (key, v) in map {
                    match key.as_str() {
                        "rgb" => {
                            rgb = Some(v.as_str().and_then(parse_hex).ok_or_else(|| {
                                format!("\"rgb\" must be a #rrggbb colour, got {v}")
                            })?)
                        }
                        "256" => i256 = Some(index(v)?),
                        "16" => i16 = Some(ansi_name(key, v)?),
                        "ansi" => ansi = Some(ansi_name(key, v)?),
                        other => {
                            return Err(format!(
                                "unknown colour key {other:?}; a colour by depth takes rgb, \
                                 256, 16 and ansi"
                            ));
                        }
                    }
                }
                Ok(ColourSpec::Tiers {
                    rgb,
                    i256,
                    i16,
                    ansi,
                })
            }
            other => Err(format!("{other} is not a colour")),
        }
    }

    /// The terminal's own colour this stands for: the `ansi` tier, else the
    /// `16` one, else the nearest of the 16.
    fn following_terminal(&self) -> ColourSpec {
        let of = |a: Ansi| a.map_or(ColourSpec::Default, ColourSpec::Ansi);
        match self {
            ColourSpec::Tiers { ansi: Some(a), .. }
            | ColourSpec::Tiers {
                ansi: None,
                i16: Some(a),
                ..
            } => of(*a),
            ColourSpec::Tiers { rgb: Some(rgb), .. } | ColourSpec::Rgb(rgb) => {
                ColourSpec::Ansi(nearest16(*rgb))
            }
            ColourSpec::Tiers { i256: Some(i), .. } | ColourSpec::Index(i) => {
                ColourSpec::Ansi(nearest16(rgb256(*i)))
            }
            ColourSpec::Tiers { .. } => ColourSpec::Default,
            other => other.clone(),
        }
    }
}

fn ansi_name(key: &str, v: &Value) -> Result<Ansi, String> {
    match v.as_str() {
        Some("default") => Ok(None),
        Some(name) if ansi_index(name).is_some() => Ok(ansi_index(name)),
        _ => Err(format!(
            "\"{key}\" must be an ANSI colour name or \"default\", got {v}"
        )),
    }
}

fn index(value: &Value) -> Result<u8, String> {
    value
        .as_u64()
        .filter(|n| *n <= 255)
        .map(|n| n as u8)
        .ok_or_else(|| format!("{value} is not a 256-colour index (0-255)"))
}

fn reference(s: &str) -> Result<String, String> {
    let name = &s[1..];
    if name.is_empty() {
        return Err("\"@\" names no role".to_string());
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn role(v: Value) -> Result<RoleSpec, String> {
        RoleSpec::parse(&v)
    }

    fn fg(c: ColourSpec) -> RoleSpec {
        RoleSpec::Style(StyleSpec {
            fg: Some(c),
            ..StyleSpec::default()
        })
    }

    #[test]
    fn every_form_of_a_role_value() {
        assert_eq!(role(json!("@accent")), Ok(RoleSpec::Ref("accent".into())));
        assert_eq!(role(json!("none")), Ok(RoleSpec::None));
        assert_eq!(
            role(json!("#6aa6ff")),
            Ok(fg(ColourSpec::Rgb([0x6a, 0xa6, 0xff])))
        );
        assert_eq!(role(json!(75)), Ok(fg(ColourSpec::Index(75))));
        assert_eq!(role(json!("bright-black")), Ok(fg(ColourSpec::Ansi(8))));
        assert_eq!(role(json!("default")), Ok(fg(ColourSpec::Default)));
        assert_eq!(
            role(json!({"rgb": "#979ea8", "256": 247, "16": "default", "ansi": "bright-black"})),
            Ok(fg(ColourSpec::Tiers {
                rgb: Some([0x97, 0x9e, 0xa8]),
                i256: Some(247),
                i16: Some(None),
                ansi: Some(Some(8)),
            }))
        );
        assert_eq!(
            role(
                json!({"fg": "@accent", "bold": true, "underline": "hyperlinks", "16": {"reverse": true}})
            ),
            Ok(RoleSpec::Style(StyleSpec {
                fg: Some(ColourSpec::Ref("accent".into())),
                bold: Attr::On,
                underline: Attr::Hyperlinks,
                sixteen: Some(Box::new(StyleSpec {
                    reverse: Attr::On,
                    ..StyleSpec::default()
                })),
                ..StyleSpec::default()
            }))
        );
    }

    #[test]
    fn invalid_values_say_why() {
        for (value, says) in [
            (json!("purple"), "is not a colour"),
            (json!(300), "0-255"),
            (json!("#12"), "#rrggbb"),
            (json!("tool"), "is not a colour"),
            (
                json!({"fg": "red", "blink": true}),
                "unknown style key \"blink\"",
            ),
            (json!({"bold": "yes"}), "true, false or \"hyperlinks\""),
            (json!({"sgr": "1;31m"}), "SGR parameters"),
            (json!({"rgb": "#fff", "256": 999}), "0-255"),
            (
                json!({"rgb": "#ffffff", "ansi": "grey"}),
                "ANSI colour name or \"default\"",
            ),
            (json!("@"), "names no role"),
            (json!(true), "is not a colour"),
        ] {
            let err = role(value.clone()).expect_err(&value.to_string());
            assert!(err.contains(says), "{value}: {err}");
        }
    }

    #[test]
    fn following_the_terminal_takes_the_ansi_tier_then_16_then_nearest() {
        let follow = |v: Value| role(v).unwrap().following_terminal();
        assert_eq!(
            follow(json!({"rgb": "#979ea8", "256": 247, "16": "default", "ansi": "bright-black"})),
            fg(ColourSpec::Ansi(8))
        );
        assert_eq!(
            follow(json!({"rgb": "#52d0ae", "16": "green"})),
            fg(ColourSpec::Ansi(2))
        );
        assert_eq!(
            follow(json!({"rgb": "#c3c9d1", "ansi": "default"})),
            fg(ColourSpec::Default)
        );
        assert_eq!(follow(json!("#ff0000")), fg(ColourSpec::Ansi(9)));
        assert_eq!(
            follow(json!("@muted")),
            RoleSpec::Ref("muted".into()),
            "refs follow through"
        );
        assert_eq!(
            follow(json!({"bg": {"rgb": "#2c4a6e", "256": 24}, "16": {"reverse": true}})),
            RoleSpec::Style(StyleSpec {
                reverse: Attr::On,
                ..StyleSpec::default()
            }),
            "a 16 style wins"
        );
    }
}
