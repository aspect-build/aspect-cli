//! The Aspect CLI's colour themes: every colour the CLI draws is a role
//! (`error`, `muted`, `label.warning`, `bazel.progress`, …), and a theme
//! says what each role looks like.
//!
//! * [`Theme`]: a theme file with its inheritance flattened and every role
//!   checked. Found by name through a [`Library`]: `config.json`'s `themes`,
//!   the user's `~/.aspect/themes/<name>.json`, the themes a `config.axl`
//!   defines, then the built-ins ([`BUILTIN`], embedded from `themes/*.json`).
//! * [`Palette`]: a theme rendered at one [`Depth`], each role's SGR
//!   parameters computed once. What the console and the runtime write with.
//! * [`select`]: which theme a process uses, from [`Layer`]s of settings
//!   (the user's `config.json`, the user's and the repository's
//!   `config.axl`), each able to pick a theme, override its roles and define
//!   themes of its own.
//!
//! # Families and the background
//!
//! A name is a theme file when one has that name (`ansi`, `mono`,
//! `aspect-dark`), and otherwise a family, meaning `<name>-dark` and
//! `<name>-light`. A family's variant is the forced appearance when there is
//! one, else what the appearance probe sees ([`Seen`]). Only a known
//! background gives the fixed colours: with a system appearance or nothing
//! to go on, the theme's `follow_terminal` roles take the terminal's own
//! colours at every depth ([`Theme::following_terminal`]). Naming a variant,
//! or forcing an appearance, counts as knowing it.
//!
//! # Theme format
//!
//! A theme is `{name, family, description, appearance, extends,
//! syntax_theme, bazel, follow_terminal, roles}`. A theme without `extends`
//! extends the hidden `_base`. A role in a child replaces the parent's whole
//! role; `bazel`, `follow_terminal` and `syntax_theme` inherit unless the
//! child sets them. A role value is a colour (the foreground), a style
//! object (`fg`, `bg`, `bold`, `dim`, `italic`, `underline`, `reverse`, each
//! true, false or `"hyperlinks"`; `sgr`; and `16`, the style used at 16
//! colours and when following the terminal), `"@role"` for another role's
//! style, or `"none"`. A colour is `#rrggbb`, a 256 index, an ANSI name,
//! `"default"`, `"@role"` (that role's colour), or `{"rgb", "256", "16",
//! "ansi"}` by depth, `ansi` being the value when following the terminal.
//!
//! Nothing here fails over a theme: a value that does not parse, refers to a
//! role that does not exist or ends in a reference cycle is reported in
//! [`Theme::warnings`] and replaced by the parent's value. Reading files and
//! the environment is the caller's, except for the theme files a [`Library`]
//! is pointed at.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

mod colour;
mod spec;

pub use colour::{ANSI_NAMES, Colour, Depth, nearest16, nearest256, rgb256};
use spec::{Attr, ColourSpec, RoleSpec, StyleSpec};

/// The built-in themes, `_base` first, as `(name, JSON)`.
pub const BUILTIN: [(&str, &str); 9] = [
    ("_base", include_str!("../themes/_base.json")),
    ("aspect-dark", include_str!("../themes/aspect-dark.json")),
    ("aspect-light", include_str!("../themes/aspect-light.json")),
    (
        "high-contrast-dark",
        include_str!("../themes/high-contrast-dark.json"),
    ),
    (
        "high-contrast-light",
        include_str!("../themes/high-contrast-light.json"),
    ),
    (
        "colorblind-dark",
        include_str!("../themes/colorblind-dark.json"),
    ),
    (
        "colorblind-light",
        include_str!("../themes/colorblind-light.json"),
    ),
    ("ansi", include_str!("../themes/ansi.json")),
    ("mono", include_str!("../themes/mono.json")),
];

/// The built-in families, the default first.
pub const FAMILIES: [&str; 5] = ["aspect", "high-contrast", "colorblind", "ansi", "mono"];

/// The default family.
pub const DEFAULT_FAMILY: &str = FAMILIES[0];

/// The hidden root every theme extends.
const BASE: &str = "_base";

/// The selectable built-in theme files.
pub fn builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTIN.iter().map(|(n, _)| *n).filter(|n| *n != BASE)
}

/// The role families a theme may set roles in. A name outside them
/// (`go.command`) is a plugin's own role and is accepted as is.
const ROLE_FAMILIES: [&str; 6] = ["label", "tip", "bazel", "syntax", "link", "search"];

/// Every role the CLI defines, the ones `_base` sets.
pub fn role_names() -> Vec<String> {
    let base = parse_json(BASE, BUILTIN[0].1).expect("_base parses");
    match base.get("roles") {
        Some(Value::Object(roles)) => roles.keys().cloned().collect(),
        _ => unreachable!("_base has roles"),
    }
}

/// Whether a theme reads best on a dark or a light background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
    Any,
}

impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
            Appearance::Any => "any",
        }
    }

    /// `config.json`'s `appearance`: `"auto"` (None), `"light"` or `"dark"`.
    pub fn parse_setting(value: &Value) -> Result<Option<Appearance>, String> {
        match value.as_str() {
            Some("auto") => Ok(None),
            Some("dark") => Ok(Some(Appearance::Dark)),
            Some("light") => Ok(Some(Appearance::Light)),
            _ => Err(format!(
                "\"appearance\" is \"auto\", \"light\" or \"dark\", got {value}"
            )),
        }
    }
}

/// What the appearance probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// The background itself: a CI log viewer known to be dark, `COLORFGBG`,
    /// or the terminal's answer. Fixed colours apply.
    Known(Appearance),
    /// The system's light or dark preference: it picks the variant, but the
    /// terminal may differ, so the `follow_terminal` roles stay ANSI.
    System(Appearance),
    /// Nothing: the dark variant, the `follow_terminal` roles ANSI.
    Unknown,
}

/// Whether Bazel's own colours (its labels, status line, test headers and
/// test summary words) take the theme's `bazel.*` roles. A theme sets it with
/// `"bazel"`, and `config.json`'s top-level `"bazel"` overrides the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bazel {
    /// `"shared"`: the CLI's vocabulary, the `bazel.*` roles.
    Shared,
    /// `"tool"`: Bazel's bytes, untouched.
    Tool,
}

/// What a theme that does not say gets.
pub const DEFAULT_BAZEL: Bazel = Bazel::Shared;

impl Bazel {
    pub fn parse(value: &Value) -> Result<Bazel, String> {
        match value.as_str() {
            Some("shared") => Ok(Bazel::Shared),
            Some("tool") => Ok(Bazel::Tool),
            _ => Err(format!("\"bazel\" is \"shared\" or \"tool\", got {value}")),
        }
    }
}

/// A role's style at one depth, references followed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
    /// Raw SGR parameters, written as they are instead of the rest.
    pub raw: Option<String>,
}

impl Style {
    /// The SGR parameters, attributes then foreground then background
    /// (`1;4;38;5;75`), or empty for no styling.
    pub fn params(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let attrs = [
            (self.bold, "1"),
            (self.dim, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
            (self.reverse, "7"),
        ];
        let mut parts: Vec<String> = attrs
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, p)| p.to_string())
            .collect();
        parts.extend(self.fg.map(|c| c.params(false)));
        parts.extend(self.bg.map(|c| c.params(true)));
        parts.join(";")
    }
}

/// What a resolution depends on besides the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    /// None: no colour; every role resolves, unstyled.
    depth: Option<Depth>,
    /// Whether the terminal opens OSC 8 links ([`hyperlinks`]).
    hyperlinks: bool,
}

/// A theme, its inheritance flattened.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    /// The family it is a variant of, as its file says.
    pub family: Option<String>,
    pub description: String,
    pub appearance: Appearance,
    /// A syntect built-in the peek uses instead of the `syntax.*` roles.
    pub syntax_theme: Option<String>,
    /// None: [`DEFAULT_BAZEL`].
    pub bazel: Option<Bazel>,
    /// The roles that take the terminal's own colours when the background is
    /// unknown.
    pub follow_terminal: Vec<String>,
    /// Whether they do here ([`Theme::following_terminal`]).
    pub follows_terminal: bool,
    /// The terminal's background, when the probe knew it ([`Seen::Known`]);
    /// None when it is unknown, a guess, or never asked.
    pub background: Option<Appearance>,
    roles: BTreeMap<String, RoleSpec>,
    /// What was wrong with the theme and its parents, each fixed by falling
    /// back to the parent's value.
    pub warnings: Vec<String>,
}

/// Where themes are found by name, first match: `config`, then the theme
/// files in `dir`, then each of `extra` in turn, then the built-ins. One
/// namespace: a name in an earlier source hides it in a later one.
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// `config.json`'s `themes`.
    pub config: BTreeMap<String, Value>,
    /// The directory of the user's theme files.
    pub dir: Option<PathBuf>,
    /// Themes defined further down, by precedence: the user's `config.axl`,
    /// then the repository's.
    pub extra: Vec<BTreeMap<String, Value>>,
}

impl Library {
    /// The theme file `name` as written, `None` when there is none.
    fn find(&self, name: &str) -> Option<Result<Value, String>> {
        if name == BASE {
            return Some(parse_json(BASE, BUILTIN[0].1));
        }
        if let Some(v) = self.config.get(name) {
            return Some(Ok(v.clone()));
        }
        if let Some(dir) = &self.dir {
            let path = dir.join(format!("{name}.json"));
            match std::fs::read_to_string(&path) {
                Ok(text) => return Some(parse_json(&path.display().to_string(), &text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Some(Err(format!("failed to read {}: {e}", path.display()))),
            }
        }
        if let Some(v) = self.extra.iter().find_map(|m| m.get(name)) {
            return Some(Ok(v.clone()));
        }
        BUILTIN
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(n, text)| parse_json(n, text))
    }

    /// Every theme and family name this library knows, for a did-you-mean.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = builtin_names().map(String::from).collect();
        names.extend(self.config.keys().cloned());
        names.extend(self.extra.iter().flat_map(|m| m.keys().cloned()));
        if let Some(entries) = self.dir.as_ref().and_then(|d| std::fs::read_dir(d).ok()) {
            names.extend(entries.flatten().filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                name.strip_suffix(".json").map(String::from)
            }));
        }
        let families: Vec<String> = names
            .iter()
            .filter_map(|n| n.strip_suffix("-dark").or_else(|| n.strip_suffix("-light")))
            .map(String::from)
            .collect();
        names.extend(families);
        names
    }

    /// The theme file `name`, its parents followed.
    pub fn load(&self, name: &str) -> Result<Theme, String> {
        if name == BASE {
            return Err(format!("{BASE:?} is not a theme of its own"));
        }
        let Some(found) = self.find(name) else {
            return Err(self.no_theme(name));
        };
        let def = found?;
        let mut warnings = Vec::new();
        let theme = self.build(name, &def, &mut vec![], &mut warnings);
        Ok(Theme { warnings, ..theme })
    }

    fn no_theme(&self, name: &str) -> String {
        format!(
            "no theme or family named {name:?}{}",
            did_you_mean(name, self.names().iter().map(String::as_str))
        )
    }

    /// A theme written in place (`config.json`'s `theme` as an object).
    pub fn inline(&self, def: &Value) -> Theme {
        let mut warnings = Vec::new();
        let theme = self.build("(config.json)", def, &mut vec![], &mut warnings);
        Theme { warnings, ..theme }
    }

    /// What `name` names: the theme file of that name, else the family's
    /// variant for `appearance` when it is forced, else for what `probe`
    /// sees. `probe` is called only for a family with no forced appearance.
    pub fn choose(
        &self,
        name: &str,
        appearance: Option<Appearance>,
        probe: &mut dyn FnMut() -> Seen,
    ) -> Result<Theme, String> {
        let (theme, follow) = self.choose_variant(name, appearance, probe)?;
        Ok(if follow {
            theme.following_terminal()
        } else {
            theme
        })
    }

    /// [`Library::choose`]'s theme before [`Theme::following_terminal`], and
    /// whether the background is unknown so it should follow the terminal.
    fn choose_variant(
        &self,
        name: &str,
        appearance: Option<Appearance>,
        probe: &mut dyn FnMut() -> Seen,
    ) -> Result<(Theme, bool), String> {
        if self.find(name).is_some() {
            return Ok((self.load(name)?, false));
        }
        let mut known = None;
        let (variant, follow) = match appearance {
            Some(a) => (a, false),
            None => match probe() {
                Seen::Known(a) => {
                    known = Some(a);
                    (a, false)
                }
                Seen::System(a) => (a, true),
                Seen::Unknown => (Appearance::Dark, true),
            },
        };
        let suffix = if variant == Appearance::Light {
            "light"
        } else {
            "dark"
        };
        let file = format!("{name}-{suffix}");
        if self.find(&file).is_none() {
            return Err(self.no_theme(name));
        }
        let mut theme = self.load(&file)?;
        theme.background = known;
        Ok((theme, follow))
    }

    fn build(
        &self,
        name: &str,
        def: &Value,
        chain: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Theme {
        let Value::Object(def) = def else {
            warnings.push(format!(
                "theme {name:?} is not an object; using {:?}",
                "aspect-dark"
            ));
            return self.builtin("aspect-dark", chain, warnings);
        };
        for key in def.keys() {
            if !matches!(
                key.as_str(),
                "name"
                    | "family"
                    | "description"
                    | "appearance"
                    | "extends"
                    | "syntax_theme"
                    | "bazel"
                    | "follow_terminal"
                    | "roles"
            ) {
                warnings.push(format!("theme {name:?}: unknown key {key:?}"));
            }
        }
        chain.push(name.to_string());
        let parent = if name == BASE {
            None
        } else {
            let wanted = match def.get("extends") {
                None => BASE.to_string(),
                Some(Value::String(p)) => p.clone(),
                Some(other) => {
                    warnings.push(format!(
                        "theme {name:?}: \"extends\" must be a theme name, got {other}"
                    ));
                    BASE.to_string()
                }
            };
            Some(self.parent(name, &wanted, chain, warnings))
        };
        chain.pop();
        let text = |key: &str| def.get(key).and_then(Value::as_str).map(String::from);
        let appearance = match text("appearance").as_deref() {
            Some("dark") => Appearance::Dark,
            Some("light") => Appearance::Light,
            Some("any") => Appearance::Any,
            None => parent.as_ref().map_or(Appearance::Any, |p| p.appearance),
            Some(other) => {
                warnings.push(format!(
                    "theme {name:?}: \"appearance\" is dark, light or any, got {other:?}"
                ));
                Appearance::Any
            }
        };
        let bazel = match def.get("bazel").map(Bazel::parse) {
            None => parent.as_ref().and_then(|p| p.bazel),
            Some(Ok(b)) => Some(b),
            Some(Err(e)) => {
                warnings.push(format!("theme {name:?}: {e}"));
                parent.as_ref().and_then(|p| p.bazel)
            }
        };
        let follow_terminal = match def.get("follow_terminal") {
            None => parent
                .as_ref()
                .map(|p| p.follow_terminal.clone())
                .unwrap_or_default(),
            Some(Value::Array(roles)) if roles.iter().all(Value::is_string) => roles
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect(),
            Some(other) => {
                warnings.push(format!(
                    "theme {name:?}: \"follow_terminal\" must be a list of role names, got {other}"
                ));
                parent
                    .as_ref()
                    .map(|p| p.follow_terminal.clone())
                    .unwrap_or_default()
            }
        };
        let inherited = parent.as_ref().map(|p| p.roles.clone()).unwrap_or_default();
        let roles = match def.get("roles") {
            None => inherited,
            Some(Value::Object(roles)) => {
                overlay(&format!("theme {name:?}"), inherited, roles, warnings)
            }
            Some(other) => {
                warnings.push(format!(
                    "theme {name:?}: \"roles\" must be an object, got {other}"
                ));
                inherited
            }
        };
        for role in &follow_terminal {
            if !roles.contains_key(role) {
                warnings.push(format!(
                    "theme {name:?}: \"follow_terminal\" names {role:?}, which is not a role"
                ));
            }
        }
        Theme {
            name: text("name").unwrap_or_else(|| name.to_string()),
            family: text("family"),
            description: text("description").unwrap_or_default(),
            appearance,
            syntax_theme: text("syntax_theme")
                .or_else(|| parent.as_ref().and_then(|p| p.syntax_theme.clone())),
            bazel,
            follow_terminal,
            follows_terminal: false,
            background: None,
            roles,
            warnings: Vec::new(),
        }
    }

    /// The parent `wanted` of `child`, `_base` when it cannot be had.
    fn parent(
        &self,
        child: &str,
        wanted: &str,
        chain: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Theme {
        if chain.iter().any(|n| n == wanted) {
            let cycle: Vec<&str> = chain.iter().map(String::as_str).chain([wanted]).collect();
            warnings.push(format!(
                "theme {child:?}: \"extends\" makes a cycle ({}); it extends {BASE:?} instead",
                cycle.join(" -> ")
            ));
            return self.builtin(BASE, chain, warnings);
        }
        match self.find(wanted) {
            Some(Ok(def)) => self.build(wanted, &def, chain, warnings),
            found => {
                let why = match found {
                    Some(Err(e)) => e,
                    _ => format!(
                        "no theme named {wanted:?}{}",
                        did_you_mean(wanted, self.names().iter().map(String::as_str))
                    ),
                };
                warnings.push(format!(
                    "theme {child:?} extends {wanted:?}: {why}; it extends {BASE:?} instead"
                ));
                self.builtin(BASE, chain, warnings)
            }
        }
    }

    fn builtin(&self, name: &str, chain: &mut Vec<String>, warnings: &mut Vec<String>) -> Theme {
        let (_, text) = BUILTIN
            .iter()
            .find(|(n, _)| *n == name)
            .expect("a built-in");
        let def = parse_json(name, text).expect("built-in themes parse");
        Library::default().build(name, &def, chain, warnings)
    }
}

/// `roles` laid over `inherited`. A value that does not parse, or that does
/// not resolve once the whole level is laid, gives way to the inherited one,
/// with a warning that starts with `label` (the theme, or the source of an
/// override).
fn overlay(
    label: &str,
    inherited: BTreeMap<String, RoleSpec>,
    roles: &serde_json::Map<String, Value>,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, RoleSpec> {
    let known = role_names();
    let mut out = inherited.clone();
    let mut laid = Vec::new();
    let fallback = |role: &str| match inherited.contains_key(role) {
        true => "the inherited value",
        false => "no styling",
    };
    for (role, value) in roles {
        if !known.contains(role) && is_reserved(role) {
            warnings.push(format!(
                "{label}: {role:?} is not a role{}",
                did_you_mean(role, known.iter().map(String::as_str))
            ));
            continue;
        }
        match RoleSpec::parse(value) {
            Ok(spec) => {
                out.insert(role.clone(), spec);
                laid.push(role.clone());
            }
            Err(e) => warnings.push(format!(
                "{label}: role {role:?}: {e}; using {}",
                fallback(role)
            )),
        }
    }
    // Giving one role back its inherited value can mend another that refers
    // to it, so settle until nothing changes; each role gives way once.
    loop {
        let broken = laid.iter().position(|role| check(&out, role).is_err());
        let Some(i) = broken else { break };
        let role = laid.remove(i);
        let err = check(&out, &role).expect_err("broken");
        warnings.push(format!(
            "{label}: role {role:?}: {err}; using {}",
            fallback(&role)
        ));
        match inherited.get(&role) {
            Some(spec) => out.insert(role, spec.clone()),
            None => out.insert(role, RoleSpec::None),
        };
    }
    out
}

/// Whether `role` resolves at every depth.
fn check(roles: &BTreeMap<String, RoleSpec>, role: &str) -> Result<(), String> {
    for depth in Depth::ALL {
        let target = Target {
            depth: Some(depth),
            hyperlinks: true,
        };
        resolve(roles, role, target, &mut vec![])?;
    }
    Ok(())
}

/// Whether a role name not among the CLI's is in a family the CLI owns, or
/// has no family at all, and so is a typo rather than a plugin's role.
fn is_reserved(role: &str) -> bool {
    match role.split_once('.') {
        None => true,
        Some((family, _)) => ROLE_FAMILIES.contains(&family),
    }
}

fn resolve(
    roles: &BTreeMap<String, RoleSpec>,
    role: &str,
    target: Target,
    stack: &mut Vec<String>,
) -> Result<Style, String> {
    let spec = roles
        .get(role)
        .ok_or_else(|| format!("refers to {role:?}, which is not a role"))?;
    resolve_spec(roles, role, spec, target, stack)
}

fn resolve_spec(
    roles: &BTreeMap<String, RoleSpec>,
    role: &str,
    spec: &RoleSpec,
    target: Target,
    stack: &mut Vec<String>,
) -> Result<Style, String> {
    if stack.iter().any(|r| r == role) {
        let cycle: Vec<&str> = stack.iter().map(String::as_str).chain([role]).collect();
        return Err(format!(
            "its references make a cycle ({})",
            cycle.join(" -> ")
        ));
    }
    stack.push(role.to_string());
    let out = match spec {
        RoleSpec::Ref(target_role) => resolve(roles, target_role, target, stack),
        RoleSpec::None => Ok(Style::default()),
        RoleSpec::Style(style) => resolve_style(roles, style, target, stack),
    };
    stack.pop();
    out
}

fn resolve_style(
    roles: &BTreeMap<String, RoleSpec>,
    spec: &StyleSpec,
    target: Target,
    stack: &mut Vec<String>,
) -> Result<Style, String> {
    let spec = match (&spec.sixteen, target.depth) {
        (Some(sixteen), Some(Depth::Ansi16)) => sixteen,
        _ => spec,
    };
    let mut colour = |c: &Option<ColourSpec>| match c {
        Some(c) => resolve_colour(roles, c, target, stack),
        None => Ok(None),
    };
    let fg = colour(&spec.fg)?;
    let bg = colour(&spec.bg)?;
    let on = |a: Attr| match a {
        Attr::Off => false,
        Attr::On => true,
        Attr::Hyperlinks => target.hyperlinks,
    };
    Ok(Style {
        fg,
        bg,
        bold: on(spec.bold),
        dim: on(spec.dim),
        italic: on(spec.italic),
        underline: on(spec.underline),
        reverse: on(spec.reverse),
        raw: spec.sgr.clone(),
    })
}

fn resolve_colour(
    roles: &BTreeMap<String, RoleSpec>,
    spec: &ColourSpec,
    target: Target,
    stack: &mut Vec<String>,
) -> Result<Option<Colour>, String> {
    let depth = target.depth;
    let at = |rgb: [u8; 3]| match depth {
        Some(Depth::TrueColor) | None => Colour::Rgb(rgb[0], rgb[1], rgb[2]),
        Some(Depth::Ansi256) => Colour::Index(nearest256(rgb)),
        Some(Depth::Ansi16) => Colour::Ansi(nearest16(rgb)),
    };
    let index = |i: u8| match depth {
        Some(Depth::Ansi16) => Colour::Ansi(nearest16(rgb256(i))),
        _ => Colour::Index(i),
    };
    let sixteen = Some(Depth::Ansi16);
    Ok(match spec {
        ColourSpec::Ref(role) => {
            let s = resolve(roles, role, target, stack)?;
            s.fg.or(s.bg)
        }
        ColourSpec::Default => None,
        ColourSpec::Ansi(n) => Some(Colour::Ansi(*n)),
        ColourSpec::Index(i) => Some(index(*i)),
        ColourSpec::Rgb(rgb) => Some(at(*rgb)),
        ColourSpec::Tiers { i16: Some(n), .. } if depth == sixteen => n.map(Colour::Ansi),
        ColourSpec::Tiers { rgb: Some(rgb), .. }
            if depth != Some(Depth::Ansi256) && depth != sixteen =>
        {
            Some(at(*rgb))
        }
        ColourSpec::Tiers { i256: Some(i), .. } if depth != sixteen => Some(Colour::Index(*i)),
        ColourSpec::Tiers { rgb: Some(rgb), .. } => Some(at(*rgb)),
        ColourSpec::Tiers { i256: Some(i), .. } => Some(index(*i)),
        ColourSpec::Tiers {
            i16: Some(n), ansi, ..
        } => n.or(ansi.flatten()).map(Colour::Ansi),
        ColourSpec::Tiers { ansi, .. } => ansi.flatten().map(Colour::Ansi),
    })
}

impl Theme {
    /// The built-in theme file `name`.
    pub fn builtin(name: &str) -> Option<Theme> {
        builtin_names().any(|n| n == name).then(|| {
            Library::default()
                .load(name)
                .expect("a built-in theme loads")
        })
    }

    /// The default family's dark variant with its background known.
    pub fn aspect_dark() -> Theme {
        Theme::builtin("aspect-dark").expect("aspect-dark is built in")
    }

    /// This theme for a background nobody could tell: every role in
    /// [`Theme::follow_terminal`] takes its `ansi` colour (else its `16` one)
    /// or its `16` style, at every depth.
    pub fn following_terminal(mut self) -> Theme {
        for role in &self.follow_terminal {
            if let Some(spec) = self.roles.get(role) {
                let spec = spec.following_terminal();
                self.roles.insert(role.clone(), spec);
            }
        }
        self.follows_terminal = true;
        self
    }

    /// The names of the roles this theme sets, the CLI's and any plugin's.
    pub fn roles(&self) -> impl Iterator<Item = &str> {
        self.roles.keys().map(String::as_str)
    }
}

/// A role at a palette's depth: its style and SGR parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub style: Style,
    /// The SGR parameters (`1;38;5;203`), empty for no styling.
    pub params: String,
}

impl Entry {
    fn of(style: Style, depth: Option<Depth>) -> Entry {
        let params = match depth {
            Some(_) => style.params(),
            None => String::new(),
        };
        Entry { style, params }
    }

    /// The opening escape, `ESC[…m`, or empty.
    pub fn sgr(&self) -> String {
        sgr(&self.params)
    }

    /// Only the foreground, or with `background` the background, as an
    /// escape; empty when it has none.
    pub fn colour_sgr(&self, background: bool) -> String {
        if self.params.is_empty() {
            return String::new();
        }
        let c = if background {
            self.style.bg
        } else {
            self.style.fg
        };
        c.map(|c| sgr(&c.params(background))).unwrap_or_default()
    }
}

/// `ESC[<params>m`, or empty for no parameters.
pub fn sgr(params: &str) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{params}m")
    }
}

/// The SGR reset.
pub const RESET: &str = "\x1b[0m";

/// A theme rendered at one depth, or at none (no SGR at all). Ungated: a
/// caller still asks whether its stream takes colour.
#[derive(Debug, Clone)]
pub struct Palette {
    theme: Theme,
    target: Target,
    entries: BTreeMap<String, Entry>,
}

impl Palette {
    /// `theme` at `depth`, a `"hyperlinks"` attribute on when `hyperlinks`.
    pub fn new(theme: Theme, depth: Option<Depth>, hyperlinks: bool) -> Palette {
        let target = Target { depth, hyperlinks };
        let entries = theme
            .roles()
            .map(|role| {
                let style = resolve(&theme.roles, role, target, &mut vec![]).unwrap_or_default();
                (role.to_string(), Entry::of(style, depth))
            })
            .collect();
        Palette {
            theme,
            target,
            entries,
        }
    }

    /// `aspect-dark` at `depth`, background known: for a caller with no
    /// process theme, such as a test.
    pub fn aspect_dark(depth: Depth) -> Palette {
        Palette::new(Theme::aspect_dark(), Some(depth), false)
    }

    /// No SGR at all.
    pub fn plain() -> Palette {
        Palette::new(Theme::aspect_dark(), None, false)
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The depth, None when the palette writes no SGR.
    pub fn depth(&self) -> Option<Depth> {
        self.target.depth
    }

    /// Whether a `"hyperlinks"` attribute is on.
    pub fn hyperlinks(&self) -> bool {
        self.target.hyperlinks
    }

    /// Whether Bazel's own colours take the `bazel.*` roles.
    pub fn bazel(&self) -> Bazel {
        self.theme.bazel.unwrap_or(DEFAULT_BAZEL)
    }

    /// `role`, None for a role the theme does not have.
    pub fn entry(&self, role: &str) -> Option<&Entry> {
        self.entries.get(role)
    }

    /// `role`, or for one the theme does not have, `default` (a role value
    /// such as `"@highlight"`) resolved against the theme.
    pub fn entry_or(&self, role: &str, default: Option<&Value>) -> Result<Entry, String> {
        if let Some(e) = self.entries.get(role) {
            return Ok(e.clone());
        }
        match default {
            Some(v) => {
                let spec = RoleSpec::parse(v)?;
                let style = resolve_spec(&self.theme.roles, role, &spec, self.target, &mut vec![])?;
                Ok(Entry::of(style, self.target.depth))
            }
            None => Err(format!(
                "{role:?} is not a role of theme {:?}{}",
                self.theme.name,
                did_you_mean(role, self.entries.keys().map(String::as_str))
            )),
        }
    }

    /// `role`'s SGR parameters, empty for no styling or a role the theme does
    /// not have.
    pub fn params(&self, role: &str) -> &str {
        self.entries.get(role).map_or("", |e| &e.params)
    }

    /// `role`'s opening escape, or empty.
    pub fn sgr(&self, role: &str) -> String {
        sgr(self.params(role))
    }

    /// `text` in `role`, then a reset; `text` alone when the role has no
    /// styling.
    pub fn paint(&self, role: &str, text: &str) -> String {
        match self.params(role) {
            "" => text.to_string(),
            p => format!("\x1b[{p}m{text}{RESET}"),
        }
    }

    /// `role`'s style, None for a role the theme does not have.
    pub fn style(&self, role: &str) -> Option<&Style> {
        self.entries.get(role).map(|e| &e.style)
    }
}

/// One source of theme settings: the user's `config.json`, the user's
/// `config.axl`, or the repository's. Every field is optional.
#[derive(Debug, Clone, Default)]
pub struct Layer {
    /// Named in warnings: a file, and the key or call.
    pub source: String,
    /// A theme or family name, or a theme object written in place.
    pub theme: Option<Value>,
    /// The family's variant: `Some(None)` is an explicit `auto`.
    pub appearance: Option<Option<Appearance>>,
    pub bazel: Option<Bazel>,
    /// Role overrides for whichever theme is selected: role names to values,
    /// and `dark` and `light` to overrides for that appearance only.
    pub roles: serde_json::Map<String, Value>,
    /// Themes this source defines, by name.
    pub themes: BTreeMap<String, Value>,
}

impl Layer {
    /// The keys of `config.json` that make a layer. `theme` is a name, a
    /// selection (`{"name", "appearance", "bazel", "roles"}`, any of them),
    /// or a theme written in place (an object with `extends` or `family`);
    /// `appearance` and `bazel` may also sit beside it, the selection's own
    /// winning. A value of the wrong shape is a warning in `warnings` and
    /// left out.
    pub fn from_config_json(
        source: &str,
        theme: Option<&Value>,
        appearance: Option<&Value>,
        bazel: Option<&Value>,
        themes: BTreeMap<String, Value>,
        warnings: &mut Vec<String>,
    ) -> Layer {
        let mut layer = Layer {
            source: source.to_string(),
            themes,
            ..Layer::default()
        };
        fn setting(
            key: &str,
            value: Option<&Value>,
            layer: &mut Layer,
            warnings: &mut Vec<String>,
        ) {
            let Some(v) = value else { return };
            let warned = |e: String| format!("{} \"{key}\": {e}; ignoring it", layer.source);
            if key.ends_with("appearance") {
                match Appearance::parse_setting(v) {
                    Ok(a) => layer.appearance = Some(a),
                    Err(e) => warnings.push(warned(e)),
                }
            } else {
                match Bazel::parse(v) {
                    Ok(b) => layer.bazel = Some(b),
                    Err(e) => warnings.push(warned(e)),
                }
            }
        }
        setting("appearance", appearance, &mut layer, warnings);
        setting("bazel", bazel, &mut layer, warnings);
        match theme {
            None => {}
            Some(Value::String(name)) => layer.theme = Some(Value::String(name.clone())),
            Some(def @ Value::Object(map))
                if map.contains_key("extends") || map.contains_key("family") =>
            {
                layer.theme = Some(def.clone())
            }
            Some(Value::Object(map)) => {
                for (key, v) in map {
                    match key.as_str() {
                        "name" => match v {
                            Value::String(_) => layer.theme = Some(v.clone()),
                            _ => warnings.push(format!(
                                "{source} \"theme.name\" must be a theme or family name, got {v}; ignoring it"
                            )),
                        },
                        "appearance" => setting("theme.appearance", Some(v), &mut layer, warnings),
                        "bazel" => setting("theme.bazel", Some(v), &mut layer, warnings),
                        "roles" => match v {
                            Value::Object(roles) => layer.roles = roles.clone(),
                            _ => warnings.push(format!(
                                "{source} \"theme.roles\" must be an object, got {v}; ignoring it"
                            )),
                        },
                        other => warnings.push(format!(
                            "{source} \"theme\": unknown key {other:?}; a selection takes name, \
                             appearance, bazel and roles, and a theme written in place needs \
                             \"extends\""
                        )),
                    }
                }
            }
            Some(other) => warnings.push(format!(
                "{source} \"theme\" must be a theme or family name or an object, got {other}; ignoring it"
            )),
        }
        layer
    }
}

/// The theme a process uses, from `layers` in precedence order (the user's
/// `config.json`, the user's `config.axl`, the repository's):
///
/// * The theme: `aspect_theme` (`ASPECT_THEME`), else the first layer that
///   names one, else the `aspect` family. A family's variant is the
///   `appearance` of the first layer that sets one, an explicit `auto`
///   included; with `auto`, or none set, it is what `probe` sees
///   ([`Library::choose`]). `probe` is called at most once, and only then.
/// * `bazel`: the first layer's that sets it, else the theme's.
/// * Role overrides: every layer's, the last first, so a higher layer's
///   override wins and a lower one's applies to whatever theme is selected.
///   Each layer's overrides for every appearance come before those for the
///   theme's own (`dark` or `light`). A bad override is a warning naming its
///   source, and the role keeps the value below it.
///
/// With the background unknown, the theme's `follow_terminal` roles then
/// take the terminal's own colours, overrides included. A name that cannot
/// be had is reported in the theme's warnings and the default family is used
/// instead. Whether a stream takes colour at all is the caller's to decide
/// first.
pub fn select(
    aspect_theme: Option<&str>,
    layers: &[Layer],
    library: &Library,
    probe: &mut dyn FnMut() -> Seen,
) -> Theme {
    let mut once = None;
    let mut probe = move || *once.get_or_insert_with(&mut *probe);
    let probe: &mut dyn FnMut() -> Seen = &mut probe;
    let appearance = layers.iter().find_map(|l| l.appearance).flatten();
    let fallback = |probe: &mut dyn FnMut() -> Seen, warning: String| {
        let (mut theme, follow) = library
            .choose_variant(DEFAULT_FAMILY, appearance, probe)
            .expect("the default family is built in");
        theme
            .warnings
            .insert(0, format!("{warning}; using {DEFAULT_FAMILY:?}"));
        (theme, follow)
    };
    let named = match aspect_theme {
        Some(name) => Some(("ASPECT_THEME".to_string(), Value::String(name.to_string()))),
        None => layers.iter().find_map(|l| {
            l.theme
                .clone()
                .map(|t| (format!("{} \"theme\"", l.source), t))
        }),
    };
    let (mut theme, follow) = match named {
        None => library
            .choose_variant(DEFAULT_FAMILY, appearance, probe)
            .expect("the default family is built in"),
        Some((_, def @ Value::Object(_))) => (library.inline(&def), false),
        Some((source, Value::String(name))) => {
            match library.choose_variant(&name, appearance, probe) {
                Ok(chosen) => chosen,
                Err(e) => fallback(probe, format!("{source}: {e}")),
            }
        }
        Some((source, other)) => fallback(probe, format!("{source}: {other} is not a theme")),
    };
    if let Some(bazel) = layers.iter().find_map(|l| l.bazel) {
        theme.bazel = Some(bazel);
    }
    for layer in layers.iter().rev() {
        let appearance_key = |k: &str| k == "dark" || k == "light";
        let every: serde_json::Map<String, Value> = layer
            .roles
            .iter()
            .filter(|(k, _)| !appearance_key(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let own = match layer.roles.get(theme.appearance.name()) {
            Some(Value::Object(roles)) => roles.clone(),
            Some(other) => {
                theme.warnings.push(format!(
                    "{} \"roles.{}\" must be an object of roles, got {other}; ignoring it",
                    layer.source,
                    theme.appearance.name()
                ));
                Default::default()
            }
            None => Default::default(),
        };
        for (label, roles) in [
            (format!("{} \"roles\"", layer.source), every),
            (
                format!("{} \"roles.{}\"", layer.source, theme.appearance.name()),
                own,
            ),
        ] {
            if !roles.is_empty() {
                let mut warnings = Vec::new();
                theme.roles = overlay(&label, theme.roles.clone(), &roles, &mut warnings);
                theme.warnings.append(&mut warnings);
            }
        }
    }
    if follow {
        theme.following_terminal()
    } else {
        theme
    }
}

/// The background `COLORFGBG` reports (`fg;bg` or `fg;default;bg`): of the
/// 16, light for 7 and 9 to 15; of the 256-colour palette beyond them, light
/// when its relative luminance is above a mid grey's; None when unknown.
pub fn background(colorfgbg: Option<&str>) -> Option<Appearance> {
    let bg: u8 = colorfgbg?.rsplit(';').next()?.trim().parse().ok()?;
    let light = if bg < 16 {
        bg >= 7 && bg != 8
    } else {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = rgb256(bg);
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) > 0.18
    };
    Some(if light {
        Appearance::Light
    } else {
        Appearance::Dark
    })
}

/// Whether the terminal is known to open OSC 8 links, from its environment:
/// iTerm2, WezTerm, VS Code, Ghostty, kitty, Windows Terminal, and VTE 0.50
/// and later; not Terminal.app, tmux, a CI log, or anything unknown.
pub fn hyperlinks(var: impl Fn(&str) -> Option<String>, ci: bool) -> bool {
    if ci || var("TMUX").is_some() {
        return false;
    }
    let program = var("TERM_PROGRAM");
    if matches!(
        program.as_deref(),
        Some("iTerm.app" | "WezTerm" | "vscode" | "ghostty")
    ) {
        return true;
    }
    if program.as_deref() == Some("Apple_Terminal") {
        return false;
    }
    var("KITTY_WINDOW_ID").is_some()
        || var("WT_SESSION").is_some()
        || var("VTE_VERSION")
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|v| v >= 5000)
}

/// `config.json`'s `color_depth`: `"auto"` (None), 24, 256, 16 or 0
/// (Some(None): no colour).
pub fn parse_depth(value: &Value) -> Result<Option<Option<Depth>>, String> {
    let err = || format!("\"color_depth\" is \"auto\", 24, 256, 16 or 0, got {value}");
    match value {
        Value::String(s) if s == "auto" => Ok(None),
        Value::Number(n) => match n.as_u64() {
            Some(24) => Ok(Some(Some(Depth::TrueColor))),
            Some(256) => Ok(Some(Some(Depth::Ansi256))),
            Some(16) => Ok(Some(Some(Depth::Ansi16))),
            Some(0) => Ok(Some(None)),
            _ => Err(err()),
        },
        _ => Err(err()),
    }
}

fn parse_json(source: &str, text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("failed to parse {source}: {e}"))
}

/// `"; did you mean \"x\"?"` for the closest of `candidates`, or empty.
fn did_you_mean<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> String {
    candidates
        .map(|c| (edit_distance(name, c), c))
        .filter(|(d, c)| *d <= (name.len().max(c.len()) / 3).max(2))
        .min()
        .map(|(_, c)| format!("; did you mean {c:?}?"))
        .unwrap_or_default()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (prev + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests;
