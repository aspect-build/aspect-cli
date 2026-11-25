//! The process's colour theme: which theme ([`aspect_theme::select`]) at
//! which depth, carried by a run's `Env` as `theme`, behind `ctx.std.theme`
//! in AXL.
//!
//! Its settings come in layers, first match winning, role overrides stacked
//! with the higher on top ([`aspect_theme::Layer`]):
//!
//! 1. `ASPECT_THEME`, a theme or family name for this run.
//! 2. The user's `~/.aspect/config.json`: `theme` (a name; a selection
//!    `{"name", "appearance", "bazel", "roles"}`; or a theme written in
//!    place, with `extends`), `appearance` (`auto`, `light`, `dark`), `bazel`
//!    (`shared`, `tool`), `color_depth` (`auto`, 24, 256, 16, 0) and `themes`.
//! 3. The user's `~/.aspect/config.axl`, through `ctx.theme`
//!    ([`crate::engine::config_theme`]).
//! 4. The repository's `.aspect/config.axl`, the same way.
//! 5. The `aspect` family.
//!
//! Themes by name are one namespace: `config.json`'s `themes`, then
//! `~/.aspect/themes/<name>.json`, then the user's `config.axl`, then the
//! repository's, then the built-ins. A run's theme is first resolved without
//! the `config.axl` layers ([`of_process`]), and again once its configs have
//! run ([`refresh`]).
//!
//! With `appearance` `auto`, a family's variant comes from [`seen`], asked
//! only then: a CI log viewer known to be dark (GitHub Actions, Buildkite)
//! or `COLORFGBG` is a known background and gets the fixed colours; the
//! system's light or dark preference ([`system_appearance`]) picks the
//! variant but keeps the theme's `follow_terminal` roles on the terminal's
//! own colours; with neither, the dark variant does the same.
//!
//! A stream that takes no colour ([`crate::color`], which a non-empty
//! `NO_COLOR` turns off) gets plain text. When neither does, nothing is read:
//! the palette is `mono` under `NO_COLOR`, for the full-screen console, which
//! needs reverse video and asks for it ungated, and otherwise writes no SGR.
//! Problems with any of it are warnings naming their source, each printed
//! once; none fails a run.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use aspect_theme::{Appearance, Depth, Layer, Library, Palette, Seen, Theme};

use crate::color::StreamColor;

/// The environment variables a theme is resolved from.
const VARS: [&str; 17] = [
    "SSH_CONNECTION",
    "SSH_TTY",
    "ASPECT_THEME",
    "NO_COLOR",
    "COLORFGBG",
    "COLORTERM",
    "TERM",
    "GITHUB_ACTIONS",
    "BUILDKITE",
    "CIRCLECI",
    "GITLAB_CI",
    "CI",
    "TMUX",
    "TERM_PROGRAM",
    "KITTY_WINDOW_ID",
    "WT_SESSION",
    "VTE_VERSION",
];

/// What a theme is resolved from besides the `config.axl` layers: whether
/// each stream takes colour, the environment, the home holding `.aspect/`,
/// and the system appearance probe.
#[derive(Debug, Clone)]
pub struct Inputs {
    pub color: StreamColor,
    vars: BTreeMap<String, String>,
    pub home: Option<PathBuf>,
    system: fn() -> Option<Appearance>,
    /// Whether these are the process's own, so a resolution is the
    /// process's too ([`refresh`]).
    process: bool,
    /// The background the terminal reported ([`learn_background`]), heeded
    /// where nothing surer says.
    learnt: Option<Appearance>,
}

impl Inputs {
    /// A snapshot: `var` reads one environment variable.
    pub fn snapshot(
        color: StreamColor,
        var: impl Fn(&str) -> Option<OsString>,
        home: Option<&Path>,
        system: fn() -> Option<Appearance>,
    ) -> Inputs {
        let vars = VARS
            .iter()
            .filter_map(|name| {
                let value = var(name)?.into_string().ok()?;
                Some((name.to_string(), value))
            })
            .collect();
        Inputs {
            color,
            vars,
            home: home.map(Path::to_path_buf),
            system,
            process: false,
            learnt: None,
        }
    }

    /// These inputs with the terminal's background known to be `bg`.
    pub fn with_background(&self, bg: Appearance) -> Inputs {
        Inputs {
            learnt: Some(bg),
            ..self.clone()
        }
    }

    /// These inputs as if both streams took colour: for output the user
    /// asked to colour (`--color=yes`) though it goes to a pipe.
    pub fn coloured(&self) -> Inputs {
        Inputs {
            color: StreamColor {
                stdout: true,
                stderr: true,
            },
            ..self.clone()
        }
    }

    /// The process's own. A unit test's process has none: no environment, no
    /// home, no colour.
    pub fn of_process() -> Inputs {
        if cfg!(test) {
            let off = StreamColor {
                stdout: false,
                stderr: false,
            };
            return Inputs::snapshot(off, |_| None, None, || None);
        }
        Inputs {
            process: true,
            ..Inputs::snapshot(
                StreamColor::of_process(),
                |name| std::env::var_os(name),
                dirs::home_dir().as_deref(),
                system_appearance,
            )
        }
    }

    fn text(&self, name: &str) -> Option<String> {
        self.vars.get(name).filter(|v| !v.is_empty()).cloned()
    }

    /// Whether `path`, a `config.axl`, is the user's own (under
    /// `~/.aspect/`) rather than a repository's.
    pub fn is_user_config(&self, path: &Path) -> bool {
        self.home
            .as_ref()
            .is_some_and(|h| path.starts_with(h.join(".aspect")))
    }
}

/// What the `config.axl` files set through `ctx.theme`: the user's and the
/// repository's layers.
#[derive(Debug, Clone, Default)]
pub struct AxlLayers {
    pub user: Layer,
    pub repo: Layer,
}

impl AxlLayers {
    fn is_empty(&self) -> bool {
        [&self.user, &self.repo].iter().all(|l| {
            l.theme.is_none()
                && l.appearance.is_none()
                && l.bazel.is_none()
                && l.roles.is_empty()
                && l.themes.is_empty()
        })
    }
}

/// A run's theme, which its configs may change once they have run: shared by
/// every clone of the run's `Env`.
#[derive(Debug, Clone)]
pub struct ThemeSlot(Arc<RwLock<Arc<Palette>>>);

impl ThemeSlot {
    pub fn new(palette: Arc<Palette>) -> ThemeSlot {
        ThemeSlot(Arc::new(RwLock::new(palette)))
    }

    pub fn get(&self) -> Arc<Palette> {
        self.0
            .read()
            .map(|p| p.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    pub fn set(&self, palette: Arc<Palette>) {
        match self.0.write() {
            Ok(mut slot) => *slot = palette,
            Err(e) => *e.into_inner() = palette,
        }
    }
}

/// The process's theme for code with no `Env` to ask (the CLI's help, its
/// `WARNING:` lines): resolved on the first call from [`Inputs::of_process`],
/// then replaced by [`refresh`] once the configs have run, and by
/// [`learn_background`]. Its warnings wait for [`refresh`]: a name it cannot
/// find may be one a `config.axl` defines.
pub fn of_process() -> Arc<Palette> {
    static PROCESS: OnceLock<Arc<Palette>> = OnceLock::new();
    if let Some(p) = REFRESHED.read().ok().and_then(|p| p.clone()) {
        return p;
    }
    PROCESS
        .get_or_init(|| {
            let inputs = Inputs::of_process();
            let (palette, warnings) =
                crate::startup::time("theme", || resolve_with(&inputs, &AxlLayers::default()));
            if let Ok(mut state) = STATE.lock() {
                let state = state.get_or_insert_with(State::default);
                state.inputs = Some(inputs);
                state.pending = warnings;
            }
            Arc::new(palette)
        })
        .clone()
}

static REFRESHED: RwLock<Option<Arc<Palette>>> = RwLock::new(None);

/// What a later resolution of the process's theme starts from: its inputs,
/// what its configs set, the runs' slots to update (held weakly, so an
/// ended run's goes), and the first resolution's warnings, not yet printed.
#[derive(Default)]
struct State {
    inputs: Option<Inputs>,
    axl: AxlLayers,
    slots: Vec<Weak<RwLock<Arc<Palette>>>>,
    pending: Vec<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Keep `slot` up to date with the process's theme when `inputs` are the
/// process's: for a run's `Env`.
pub fn register(inputs: &Inputs, slot: &ThemeSlot) {
    if !inputs.process {
        return;
    }
    if let Ok(mut state) = STATE.lock() {
        let state = state.get_or_insert_with(State::default);
        state.slots.retain(|s| s.strong_count() > 0);
        state.slots.push(Arc::downgrade(&slot.0));
    }
}

/// Put `palette` behind [`of_process`] and in every registered slot still
/// held.
fn publish(state: &mut State, palette: &Arc<Palette>) {
    if let Ok(mut p) = REFRESHED.write() {
        *p = Some(palette.clone());
    }
    update_slots(state, palette);
}

/// Put `palette` in every registered slot still held, forgetting the rest.
fn update_slots(state: &mut State, palette: &Arc<Palette>) {
    state.slots.retain(|s| match s.upgrade() {
        Some(slot) => {
            ThemeSlot(slot).set(palette.clone());
            true
        }
        None => false,
    });
}

/// The run's theme again, now its configs have set `ctx.theme`: put in
/// `slot`, and when the inputs are the process's, behind [`of_process`] too.
/// Nothing changes when no config set anything. The warnings of the
/// process's first resolution are printed now, or this one's instead.
///
/// For the process, the theme is resolved and published under the lock
/// [`learn_background`] takes, from the inputs as they stand then, so a
/// background learnt meanwhile is never lost.
pub fn refresh(inputs: &Inputs, axl: &AxlLayers, slot: &ThemeSlot) {
    if !inputs.process {
        if !axl.is_empty() {
            let (palette, warnings) = resolve_with(inputs, axl);
            let palette = Arc::new(palette);
            warn_once(&palette, warnings);
            slot.set(palette);
        }
        return;
    }
    let (palette, warnings) = {
        let Ok(mut state) = STATE.lock() else {
            return;
        };
        let state = state.get_or_insert_with(State::default);
        state.axl = axl.clone();
        let pending = std::mem::take(&mut state.pending);
        if axl.is_empty() {
            (slot.get(), pending)
        } else {
            let inputs = state.inputs.clone().unwrap_or_else(|| inputs.clone());
            let (palette, warnings) = resolve_with(&inputs, axl);
            let palette = Arc::new(palette);
            publish(state, &palette);
            slot.set(palette.clone());
            (palette, warnings)
        }
    };
    warn_once(&palette, warnings);
}

/// Print the warnings of the process's first resolution that nothing has
/// printed yet: for a run that ended before its configs ran, resolving the
/// theme first if nothing had asked for it.
pub fn flush_warnings() {
    of_process();
    let pending = match STATE.lock() {
        Ok(mut state) => state.as_mut().map(|s| std::mem::take(&mut s.pending)),
        Err(_) => None,
    };
    if let Some(pending) = pending.filter(|p| !p.is_empty()) {
        warn_once(&of_process(), pending);
    }
}

/// The terminal's background, now the terminal has said it (the full-screen
/// console's OSC 11 query): the process's theme is resolved again as if it
/// had been known from the start, so a family's `follow_terminal` roles take
/// their fixed values, and every run's theme (`ctx.std.theme`) follows. It
/// changes nothing where something surer decided (an explicit `appearance`
/// or variant, `COLORFGBG`, a CI host), or before the process has a theme.
/// Idempotent, and safe from any thread.
///
/// Palettes already captured are not updated: a console that is running
/// (its `Config::palette`) and output coloured on request ([`shown`]) keep
/// the colours they started with.
pub fn learn_background(bg: Appearance) {
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    let Some(state) = state.as_mut() else {
        return;
    };
    let Some(inputs) = state.inputs.as_mut() else {
        return;
    };
    if inputs.learnt == Some(bg) {
        return;
    }
    *inputs = inputs.with_background(bg);
    let inputs = inputs.clone();
    let (palette, _) = resolve_with(&inputs, &state.axl);
    publish(state, &Arc::new(palette));
}

/// The palette for output the user asked to colour (`--color=yes`, so
/// `show_colour`) though its stream takes none, so the run's theme writes no
/// SGR: resolved as if the stream took colour. Otherwise the run's theme.
pub fn shown(env: &crate::engine::store::Env, show_colour: Option<bool>) -> Arc<Palette> {
    let palette = env.theme.get();
    if show_colour != Some(true) || palette.depth().is_some() {
        return palette;
    }
    let axl = env
        .theme_config
        .lock()
        .map(|a| a.clone())
        .unwrap_or_default();
    Arc::new(resolve_with(&env.theme_inputs.coloured(), &axl).0)
}

/// Marks a warning about the user's `config.json` as a whole, worded as
/// every reader of the file words it (`crate::config_json::object`).
const WHOLE_FILE: &str = "\u{1}config.json\u{1}";

/// Print each of `warnings` that has not been printed yet, styled with
/// `palette`, not the process's: this runs while that is being resolved. One
/// about the whole `config.json` ([`WHOLE_FILE`]) is as the file's other
/// readers print it, once among them (`crate::config_json::first_time`).
fn warn_once(palette: &Palette, warnings: Vec<String>) {
    static PRINTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Ok(mut printed) = PRINTED.lock() else {
        return;
    };
    let printed = printed.get_or_insert_with(HashSet::new);
    let palette = crate::color::console_supports_color().then_some(palette);
    for w in warnings {
        if !printed.insert(w.clone()) {
            continue;
        }
        let text = match w.strip_prefix(WHOLE_FILE) {
            Some(w) if crate::config_json::first_time(w) => w.to_owned(),
            Some(_) => continue,
            None => format!("theme: {w}"),
        };
        crate::diag::emit_with(crate::diag::Severity::Warning, palette, &text);
    }
}

/// [`resolve_with`] over a snapshot and no `config.axl` layers.
pub fn resolve(
    color: StreamColor,
    var: impl Fn(&str) -> Option<OsString>,
    home: Option<&Path>,
    system: fn() -> Option<Appearance>,
) -> (Palette, Vec<String>) {
    resolve_with(
        &Inputs::snapshot(color, var, home, system),
        &AxlLayers::default(),
    )
}

/// The palette `inputs` and `axl` make, and the warnings.
pub fn resolve_with(inputs: &Inputs, axl: &AxlLayers) -> (Palette, Vec<String>) {
    let text = |name: &str| inputs.text(name);
    let depth = || {
        Some(Depth::detect(
            text("COLORTERM").as_deref(),
            text("TERM").as_deref(),
        ))
    };
    let ci = crate::ci::renders_ansi(|name| text(name).is_some()) || inputs.vars.contains_key("CI");
    let hyperlinks = aspect_theme::hyperlinks(|name| text(name), ci);
    if !inputs.color.stdout && !inputs.color.stderr {
        let palette = match text("NO_COLOR") {
            Some(_) => {
                let mono = Theme::builtin("mono").expect("mono is built in");
                Palette::new(mono, depth(), hyperlinks)
            }
            None => Palette::plain(),
        };
        return (palette, Vec::new());
    }
    let mut warnings = Vec::new();
    const SOURCE: &str = "~/.aspect/config.json";
    let keys = match &inputs.home {
        Some(home) => crate::config_json::theme(&home.join(".aspect").join("config.json"))
            .unwrap_or_else(|e| {
                warnings.push(format!("{WHOLE_FILE}{e}; ignoring it"));
                Default::default()
            }),
        None => Default::default(),
    };
    warnings.extend(keys.warnings.iter().cloned());
    let color_depth = keys.color_depth.as_ref().and_then(|v| {
        aspect_theme::parse_depth(v)
            .map_err(|e| warnings.push(format!("{SOURCE} {e}; ignoring it")))
            .ok()
    });
    let user_json = Layer::from_config_json(
        SOURCE,
        keys.theme.as_ref(),
        keys.appearance.as_ref(),
        keys.bazel.as_ref(),
        keys.themes,
        &mut warnings,
    );
    let dir = inputs
        .home
        .as_ref()
        .map(|h| h.join(".aspect").join("themes"));
    log_clashes(&user_json, dir.as_deref(), axl);
    let library = Library {
        config: user_json.themes.clone(),
        dir,
        extra: vec![axl.user.themes.clone(), axl.repo.themes.clone()],
    };
    let layers = [user_json, axl.user.clone(), axl.repo.clone()];
    let mut theme = aspect_theme::select(
        text("ASPECT_THEME").as_deref(),
        &layers,
        &library,
        &mut || seen(&|name| text(name), inputs.learnt, inputs.system),
    );
    warnings.append(&mut theme.warnings);
    let depth = match color_depth.flatten() {
        Some(explicit) => explicit,
        None => depth(),
    };
    (Palette::new(theme, depth, hyperlinks), warnings)
}

/// A debug line for each theme name more than one source defines: the first
/// in [`Library`]'s order is the one used.
fn log_clashes(user_json: &Layer, dir: Option<&Path>, axl: &AxlLayers) {
    let mut sources: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let maps = [
        (user_json.source.as_str(), &user_json.themes),
        (axl.user.source.as_str(), &axl.user.themes),
        (axl.repo.source.as_str(), &axl.repo.themes),
    ];
    for (source, themes) in maps {
        for name in themes.keys() {
            sources.entry(name).or_default().push(source);
        }
    }
    for (name, defined) in sources {
        let file = dir
            .map(|d| d.join(format!("{name}.json")))
            .filter(|f| f.exists());
        let count = defined.len() + usize::from(file.is_some());
        if count > 1 {
            tracing::debug!(
                "theme {name:?} is defined in {}{}; the first is used",
                defined.join(", "),
                file.map(|f| format!(" and {}", f.display()))
                    .unwrap_or_default()
            );
        }
    }
}

/// What can be told of the terminal's background, surest first: a CI log
/// viewer known to be dark, `COLORFGBG`, what the terminal reported
/// (`learnt`), then the system's appearance, which says nothing certain about
/// the terminal ([`Seen::System`]) and is not asked on another CI host or
/// over SSH, where the system is not the viewer's.
pub fn seen(
    var: &dyn Fn(&str) -> Option<String>,
    learnt: Option<Appearance>,
    system: fn() -> Option<Appearance>,
) -> Seen {
    if var("GITHUB_ACTIONS").is_some() || var("BUILDKITE").is_some() {
        return Seen::Known(Appearance::Dark);
    }
    if let Some(bg) = aspect_theme::background(var("COLORFGBG").as_deref()) {
        return Seen::Known(bg);
    }
    if let Some(bg) = learnt {
        return Seen::Known(bg);
    }
    let remote = ["CI", "CIRCLECI", "GITLAB_CI", "SSH_CONNECTION", "SSH_TTY"]
        .iter()
        .any(|name| var(name).is_some());
    if remote {
        return Seen::Unknown;
    }
    match system() {
        Some(a) => Seen::System(a),
        None => Seen::Unknown,
    }
}

/// The system's light or dark preference. On macOS,
/// `AppleInterfaceStyle` read in process (`Dark`, or absent for light).
/// TODO: Linux's freedesktop `color-scheme` and Windows'
/// `AppsUseLightTheme` are not read yet; there the answer is unknown.
#[cfg(target_os = "macos")]
pub fn system_appearance() -> Option<Appearance> {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFPreferencesAnyApplication: CFStringRef;
        fn CFPreferencesCopyAppValue(key: CFStringRef, application: CFStringRef) -> CFTypeRef;
    }
    let key = CFString::from_static_string("AppleInterfaceStyle");
    // SAFETY: both are valid CFStrings; the result follows the create rule,
    // so wrapping it releases it.
    let value = unsafe {
        CFPreferencesCopyAppValue(key.as_concrete_TypeRef(), kCFPreferencesAnyApplication)
    };
    if value.is_null() {
        return Some(Appearance::Light);
    }
    let value = unsafe { CFType::wrap_under_create_rule(value) };
    let style = value.downcast::<CFString>()?.to_string();
    Some(if style == "Dark" {
        Appearance::Dark
    } else {
        Appearance::Light
    })
}

/// See the macOS version.
#[cfg(not(target_os = "macos"))]
pub fn system_appearance() -> Option<Appearance> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: StreamColor = StreamColor {
        stdout: true,
        stderr: true,
    };

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    fn home(config: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".aspect/themes")).unwrap();
        std::fs::write(dir.path().join(".aspect/config.json"), config).unwrap();
        dir
    }

    fn no_system() -> Option<Appearance> {
        None
    }

    fn system_light() -> Option<Appearance> {
        Some(Appearance::Light)
    }

    fn never() -> Option<Appearance> {
        panic!("the system probe must not run")
    }

    #[test]
    fn plain_streams_read_nothing_and_no_color_keeps_mono_for_the_full_screen() {
        let dir = home("{");
        let off = StreamColor {
            stdout: false,
            stderr: false,
        };
        let (plain, w) = resolve(
            off,
            env(&[("ASPECT_THEME", "ansi")]),
            Some(dir.path()),
            never,
        );
        assert_eq!(plain.depth(), None);
        assert_eq!(plain.sgr("error"), "");
        assert!(w.is_empty(), "the broken config was not read: {w:?}");
        let (mono, w) = resolve(off, env(&[("NO_COLOR", "1")]), Some(dir.path()), never);
        assert_eq!(mono.theme().name, "mono");
        assert_eq!(mono.params("active"), "7");
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn a_known_background_gets_fixed_colours() {
        for vars in [
            &[("GITHUB_ACTIONS", "true")][..],
            &[("BUILDKITE", "true")][..],
            &[("COLORFGBG", "15;0")][..],
        ] {
            let (p, w) = resolve(ON, env(vars), None, never);
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(p.theme().name, "aspect-dark", "{vars:?}");
            assert!(!p.theme().follows_terminal, "{vars:?}");
            assert_eq!(p.params("label.error"), "1;38;5;203", "{vars:?}");
        }
        let (p, _) = resolve(ON, env(&[("COLORFGBG", "0;15")]), None, never);
        assert_eq!(p.theme().name, "aspect-light");
        assert!(!p.theme().follows_terminal);
    }

    #[test]
    fn the_system_appearance_picks_the_variant_only() {
        let (p, _) = resolve(ON, env(&[]), None, system_light);
        assert_eq!(p.theme().name, "aspect-light");
        assert!(p.theme().follows_terminal);
        assert_eq!(p.params("label.error"), "1;31");
        let (p, _) = resolve(ON, env(&[]), None, no_system);
        assert_eq!(p.theme().name, "aspect-dark");
        assert!(p.theme().follows_terminal, "unknown");
        assert_eq!(p.params("muted"), "90");
    }

    #[test]
    fn config_json_chooses_family_appearance_depth_and_bazel() {
        let dir = home(
            r#"{"theme": "colorblind", "appearance": "light", "color_depth": 16, "bazel": "tool"}"#,
        );
        let (p, w) = resolve(ON, env(&[]), Some(dir.path()), never);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(p.theme().name, "colorblind-light");
        assert!(!p.theme().follows_terminal, "a forced appearance is known");
        assert_eq!(p.depth(), Some(Depth::Ansi16));
        assert_eq!(p.bazel(), aspect_theme::Bazel::Tool);
        let (p, _) = resolve(
            ON,
            env(&[("ASPECT_THEME", "high-contrast-dark")]),
            Some(dir.path()),
            never,
        );
        assert_eq!(p.theme().name, "high-contrast-dark");
        let (p, _) = resolve(ON, env(&[("COLORTERM", "truecolor")]), None, no_system);
        assert_eq!(p.depth(), Some(Depth::TrueColor));
        let (p, _) = resolve(ON, env(&[]), None, no_system);
        assert_eq!(p.depth(), Some(Depth::Ansi256));
    }

    #[test]
    fn a_user_family_from_theme_files() {
        let dir = home(r#"{"theme": "night"}"#);
        for (variant, accent) in [("dark", "#ff0000"), ("light", "#0000ff")] {
            std::fs::write(
                dir.path().join(format!(".aspect/themes/night-{variant}.json")),
                format!(
                    r#"{{"family": "night", "extends": "aspect-{variant}", "roles": {{"accent": "{accent}"}}}}"#
                ),
            )
            .unwrap();
        }
        let (p, w) = resolve(ON, env(&[("COLORFGBG", "0;15")]), Some(dir.path()), never);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(p.theme().name, "night-light");
        assert_eq!(p.params("accent"), "38;5;21");
    }

    #[test]
    fn bad_config_values_warn_and_fall_back() {
        let dir =
            home(r#"{"theme": "nope", "appearance": "dim", "bazel": "off", "color_depth": 8}"#);
        let (p, w) = resolve(ON, env(&[("COLORFGBG", "15;0")]), Some(dir.path()), never);
        assert_eq!(p.theme().name, "aspect-dark");
        assert_eq!(p.depth(), Some(Depth::Ansi256));
        assert_eq!(p.bazel(), aspect_theme::Bazel::Shared);
        assert_eq!(w.len(), 4, "{w:?}");
        let dir = home("[]");
        let (p, w) = resolve(ON, env(&[]), Some(dir.path()), no_system);
        assert_eq!(p.theme().name, "aspect-dark");
        assert!(w[0].contains("is not a JSON object"), "{w:?}");
    }

    #[test]
    fn hyperlinks_follow_the_terminal() {
        let (p, _) = resolve(
            ON,
            env(&[("TERM_PROGRAM", "iTerm.app"), ("COLORFGBG", "15;0")]),
            None,
            never,
        );
        assert_eq!(p.params("link.file"), "4");
        let (p, _) = resolve(
            ON,
            env(&[("TERM_PROGRAM", "Apple_Terminal"), ("COLORFGBG", "15;0")]),
            None,
            never,
        );
        assert_eq!(p.params("link.file"), "");
        let (p, _) = resolve(
            ON,
            env(&[("TERM_PROGRAM", "iTerm.app"), ("BUILDKITE", "true")]),
            None,
            never,
        );
        assert_eq!(p.params("link.file"), "", "CI");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_probe_answers() {
        assert!(system_appearance().is_some());
    }
}

#[cfg(test)]
mod layer_tests {
    use super::*;
    use serde_json::json;

    const KNOWN_DARK: &[(&str, &str)] = &[("COLORFGBG", "15;0"), ("TERM", "xterm-256color")];

    fn inputs(home: &Path) -> Inputs {
        let on = StreamColor {
            stdout: true,
            stderr: true,
        };
        let var = |name: &str| {
            KNOWN_DARK
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        };
        Inputs::snapshot(on, var, Some(home), || None)
    }

    fn axl_layer(source: &str, theme: Option<&str>, roles: serde_json::Value) -> Layer {
        Layer {
            source: source.into(),
            theme: theme.map(|t| json!(t)),
            roles: roles.as_object().cloned().unwrap_or_default(),
            ..Layer::default()
        }
    }

    /// Personal beats team: `config.json`, then the user's `config.axl`, then
    /// the repository's, each role override stacked on the one below.
    #[test]
    fn config_json_then_the_users_config_axl_then_the_repos() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".aspect")).unwrap();
        std::fs::write(
            home.path().join(".aspect/config.json"),
            r#"{"theme": {"roles": {"accent": 201}}}"#,
        )
        .unwrap();
        let axl = AxlLayers {
            user: axl_layer(
                "~/.aspect/config.axl ctx.theme",
                Some("colorblind"),
                json!({"error": 196}),
            ),
            repo: axl_layer(
                ".aspect/config.axl ctx.theme",
                Some("high-contrast"),
                json!({"accent": 46, "error": 160, "success": 21, "warning": "purple"}),
            ),
        };
        let (p, w) = resolve_with(&inputs(home.path()), &axl);
        assert_eq!(
            p.theme().name,
            "colorblind-dark",
            "the user's config.axl picks"
        );
        assert_eq!(p.params("accent"), "38;5;201", "config.json over both");
        assert_eq!(
            p.params("error"),
            "38;5;196",
            "the user's config.axl over the repo's"
        );
        assert_eq!(
            p.params("success"),
            "38;5;21",
            "the repo's applies to the user's pick"
        );
        assert_eq!(
            p.params("warning"),
            "38;5;221",
            "a bad value keeps the theme's"
        );
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].starts_with(".aspect/config.axl ctx.theme \"roles\": role \"warning\""),
            "{w:?}"
        );
    }

    #[test]
    fn a_config_axl_family_and_a_clash_with_config_json() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".aspect")).unwrap();
        std::fs::write(
            home.path().join(".aspect/config.json"),
            r#"{"themes": {"team-dark": {"extends": "ansi"}}}"#,
        )
        .unwrap();
        let mut repo = axl_layer(".aspect/config.axl ctx.theme", Some("team"), json!({}));
        repo.themes.insert(
            "team-dark".into(),
            json!({"extends": "aspect-dark", "roles": {"accent": 208}}),
        );
        repo.themes
            .insert("team-light".into(), json!({"extends": "aspect-light"}));
        let axl = AxlLayers {
            user: Layer::default(),
            repo,
        };
        let (p, w) = resolve_with(&inputs(home.path()), &axl);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(p.theme().name, "team-dark");
        assert_eq!(
            p.params("accent"),
            "36",
            "config.json's team-dark (ansi) wins the name"
        );
    }

    #[test]
    fn no_layers_set_leaves_the_slot_alone() {
        let home = tempfile::tempdir().unwrap();
        let slot = ThemeSlot::new(Arc::new(Palette::plain()));
        refresh(&inputs(home.path()), &AxlLayers::default(), &slot);
        assert_eq!(slot.get().depth(), None);
        let axl = AxlLayers {
            user: Layer::default(),
            repo: axl_layer("r", Some("ansi"), json!({})),
        };
        refresh(&inputs(home.path()), &axl, &slot);
        assert_eq!(slot.get().theme().name, "ansi");
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;

    const ON: StreamColor = StreamColor {
        stdout: true,
        stderr: true,
    };

    fn inputs(vars: &[(&str, &str)], home: Option<&Path>) -> Inputs {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        let var = move |name: &str| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        Inputs::snapshot(ON, var, home, || None)
    }

    fn never() -> Option<Appearance> {
        panic!("the system probe must not run")
    }

    const TERM: (&str, &str) = ("TERM", "xterm-256color");

    /// The terminal's answer makes an unknown background known: the
    /// `follow_terminal` roles take their fixed values.
    #[test]
    fn a_learnt_background_gives_the_fixed_colours() {
        let unknown = inputs(&[TERM], None);
        let (p, _) = resolve_with(&unknown, &AxlLayers::default());
        assert_eq!(p.params("search.match"), "1;4");
        assert_eq!(p.theme().background, None);
        let (p, _) = resolve_with(
            &unknown.with_background(Appearance::Dark),
            &AxlLayers::default(),
        );
        assert_eq!(p.params("search.match"), "38;5;230;48;5;58");
        assert_eq!(p.params("label.error"), "1;38;5;203");
        assert_eq!(p.theme().background, Some(Appearance::Dark));
        let (p, _) = resolve_with(
            &unknown.with_background(Appearance::Light),
            &AxlLayers::default(),
        );
        assert_eq!(p.theme().name, "aspect-light");
    }

    #[test]
    fn colorfgbg_and_an_explicit_appearance_beat_a_learnt_background() {
        let light = inputs(&[TERM, ("COLORFGBG", "0;15")], None);
        let (p, _) = resolve_with(
            &light.with_background(Appearance::Dark),
            &AxlLayers::default(),
        );
        assert_eq!(p.theme().name, "aspect-light");
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".aspect")).unwrap();
        std::fs::write(
            home.path().join(".aspect/config.json"),
            r#"{"appearance": "light"}"#,
        )
        .unwrap();
        let forced = inputs(&[TERM], Some(home.path()));
        let (p, _) = resolve_with(
            &forced.with_background(Appearance::Dark),
            &AxlLayers::default(),
        );
        assert_eq!(p.theme().name, "aspect-light");
        assert_eq!(p.theme().background, None, "chosen, not seen");
    }

    /// No process theme in a unit test: nothing to learn into.
    #[test]
    fn learning_before_the_process_has_a_theme_does_nothing() {
        learn_background(Appearance::Light);
        learn_background(Appearance::Light);
    }

    #[test]
    fn the_system_appearance_is_not_asked_on_ci_or_over_ssh() {
        for var in ["CI", "CIRCLECI", "GITLAB_CI", "SSH_CONNECTION", "SSH_TTY"] {
            let text = |name: &str| (name == var).then(|| "1".to_string());
            assert_eq!(seen(&text, None, never), Seen::Unknown, "{var}");
        }
    }

    /// `--color=yes` into a pipe: the run's theme writes nothing, so the
    /// console asks for one as if the stream took colour.
    #[test]
    fn coloured_inputs_resolve_a_theme_for_a_piped_stream() {
        let off = StreamColor {
            stdout: false,
            stderr: false,
        };
        let piped = Inputs::snapshot(
            off,
            |n| (n == "TERM").then(|| "xterm-256color".into()),
            None,
            || None,
        );
        assert_eq!(resolve_with(&piped, &AxlLayers::default()).0.depth(), None);
        let (p, _) = resolve_with(&piped.coloured(), &AxlLayers::default());
        assert_eq!(p.depth(), Some(Depth::Ansi256));
        assert_eq!(p.params("label.error"), "1;31", "background unknown");
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    /// A run's slot is held weakly: one whose `Env` has gone is dropped at
    /// the next publish, and the living ones take the palette.
    #[test]
    fn publish_updates_living_slots_and_drops_ended_ones() {
        let living = ThemeSlot::new(Arc::new(Palette::plain()));
        let ended = ThemeSlot::new(Arc::new(Palette::plain()));
        let mut state = State {
            slots: vec![Arc::downgrade(&living.0), Arc::downgrade(&ended.0)],
            ..State::default()
        };
        drop(ended);
        let dark = Arc::new(Palette::aspect_dark(Depth::Ansi256));
        update_slots(&mut state, &dark);
        assert_eq!(state.slots.len(), 1);
        assert_eq!(living.get().params("error"), "38;5;203");
    }
}
