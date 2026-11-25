use serde_json::json;

use super::*;

fn depth(n: u32) -> Depth {
    match n {
        24 => Depth::TrueColor,
        256 => Depth::Ansi256,
        16 => Depth::Ansi16,
        _ => unreachable!(),
    }
}

fn library(config: Value) -> Library {
    Library {
        config: match config {
            Value::Object(m) => m.into_iter().collect(),
            _ => BTreeMap::new(),
        },
        dir: None,
        extra: vec![],
    }
}

/// [`select`] with one layer naming `theme` and `appearance`, as a
/// `config.json` with only those keys does.
fn sel(
    aspect_theme: Option<&str>,
    theme: Option<&Value>,
    appearance: Option<Appearance>,
    lib: &Library,
    probe: &mut dyn FnMut() -> Seen,
) -> Theme {
    let layer = Layer {
        source: "config.json".into(),
        theme: theme.cloned(),
        appearance: appearance.map(Some),
        ..Layer::default()
    };
    select(aspect_theme, &[layer], lib, probe)
}

fn params(theme: &Theme, role: &str, d: u32) -> String {
    Palette::new(theme.clone(), Some(depth(d)), true)
        .params(role)
        .to_string()
}

/// `name` as `appearance` names it: forced, or for `unknown` with nothing seen.
fn chosen(lib: &Library, name: &str, appearance: &str) -> Theme {
    let forced = match appearance {
        "dark" => Some(Appearance::Dark),
        "light" => Some(Appearance::Light),
        _ => None,
    };
    lib.choose(name, forced, &mut || Seen::Unknown).unwrap()
}

/// `testdata/palette.txt` is every built-in family and single theme, in each
/// appearance, at every depth, with and without hyperlinks, as the design's
/// reference renderer (`palette-v2.sh`) draws it.
#[test]
fn every_builtin_matches_the_design() {
    let lib = Library::default();
    let mut palettes = BTreeMap::new();
    let mut lines = 0;
    for line in include_str!("testdata/palette.txt").lines() {
        let f: Vec<&str> = line.splitn(6, ' ').collect();
        let (name, appearance, d, links, role, want) = (f[0], f[1], f[2], f[3], f[4], f[5]);
        let palette = palettes
            .entry((name, appearance, d, links))
            .or_insert_with(|| {
                Palette::new(
                    chosen(&lib, name, appearance),
                    Some(depth(d.parse().unwrap())),
                    links == "yes",
                )
            });
        let want = if want == "-" { "" } else { want };
        assert_eq!(
            palette.params(role),
            want,
            "{name} {appearance} at {d}, hyperlinks {links}: {role}"
        );
        lines += 1;
    }
    assert_eq!(palettes.len(), 5 * 3 * 3 * 2);
    assert!(lines > 5000);
}

#[test]
fn every_builtin_loads_clean_and_resolves_every_role() {
    let names: Vec<&str> = builtin_names().collect();
    assert_eq!(names.len(), 8);
    let roles = role_names();
    assert!(roles.len() >= 55, "{roles:?}");
    for name in names {
        let theme = Theme::builtin(name).unwrap();
        assert!(theme.warnings.is_empty(), "{name}: {:?}", theme.warnings);
        assert!(!theme.description.is_empty(), "{name}");
        assert!(
            FAMILIES.contains(&theme.family.as_deref().unwrap()),
            "{name}"
        );
        assert!(theme.follow_terminal.len() >= 9, "{name}: inherited");
        for role in &roles {
            for d in Depth::ALL {
                for hyperlinks in [true, false] {
                    let target = Target {
                        depth: Some(d),
                        hyperlinks,
                    };
                    resolve(&theme.roles, role, target, &mut vec![])
                        .unwrap_or_else(|e| panic!("{name} {role} at {d:?}: {e}"));
                }
            }
        }
    }
    assert_eq!(Theme::builtin("_base").map(|t| t.name), None);
    assert!(Library::default().load("_base").is_err());
}

/// The console layout's roles default by reference, so every built-in theme
/// draws each as the role it names, and `task` and `phase` as a bold accent.
#[test]
fn the_console_layout_roles_follow_their_defaults_in_every_theme() {
    for name in builtin_names() {
        let theme = Theme::builtin(name).unwrap();
        for d in [24, 256, 16] {
            for (role, like) in [
                ("task.label", "text"),
                ("phase.title", "highlight"),
                ("phase.update", "muted"),
                ("phase.text", "text"),
                ("verdict.success", "success"),
                ("verdict.failure", "error"),
                ("verdict.cancelled", "warning"),
                ("verdict.verb", "highlight"),
            ] {
                assert_eq!(
                    params(&theme, role, d),
                    params(&theme, like, d),
                    "{name} at {d}: {role}"
                );
            }
            let accent = params(&theme, "accent", d);
            let bold_accent = if accent.is_empty() {
                "1".to_owned()
            } else {
                format!("1;{accent}")
            };
            for role in ["task", "phase"] {
                assert_eq!(
                    params(&theme, role, d),
                    bold_accent,
                    "{name} at {d}: {role}"
                );
            }
        }
    }
}

#[test]
fn a_child_inherits_and_overrides_one_role_whole() {
    let lib = library(json!({
        "mine-dark": {"family": "mine", "appearance": "dark", "extends": "aspect-dark",
                      "roles": {"accent": "#ff8a3d", "label.error": {"fg": "red"}}}
    }));
    let mine = lib.load("mine-dark").unwrap();
    assert!(mine.warnings.is_empty(), "{:?}", mine.warnings);
    assert_eq!(
        mine.syntax_theme.as_deref(),
        Some("base16-ocean.dark"),
        "inherited"
    );
    assert_eq!(params(&mine, "accent", 24), "38;2;255;138;61");
    assert_eq!(params(&mine, "accent", 256), "38;5;209");
    assert_eq!(params(&mine, "heading", 24), "1;38;2;255;138;61", "derived");
    assert_eq!(
        params(&mine, "label.error", 256),
        "31",
        "bold dropped: whole-role"
    );
    assert_eq!(params(&mine, "muted", 256), "38;5;247", "untouched");
    let inline = lib.inline(&json!({"extends": "aspect-dark", "roles": {"muted": 250}}));
    assert_eq!(params(&inline, "muted", 256), "38;5;250");
    assert_eq!(params(&inline, "tip.id", 256), "38;5;250");
}

#[test]
fn settings_inherit_unless_the_child_sets_them() {
    let lib = library(json!({
        "quiet": {"extends": "aspect-dark", "bazel": "tool", "follow_terminal": ["error"]},
        "child": {"extends": "quiet"},
        "loud": {"extends": "quiet", "bazel": "shared"},
        "bad": {"extends": "aspect-dark", "bazel": "off", "follow_terminal": "error"},
    }));
    let child = lib.load("child").unwrap();
    assert_eq!(child.bazel, Some(Bazel::Tool));
    assert_eq!(child.follow_terminal, ["error"]);
    assert_eq!(lib.load("loud").unwrap().bazel, Some(Bazel::Shared));
    let bad = lib.load("bad").unwrap();
    assert_eq!(bad.bazel, Some(Bazel::Shared), "_base's");
    assert_eq!(bad.warnings.len(), 2, "{:?}", bad.warnings);
    assert!(
        bad.warnings[0].contains("\"shared\" or \"tool\""),
        "{:?}",
        bad.warnings
    );
    let unknown = child.following_terminal();
    assert_eq!(params(&unknown, "error", 256), "31");
    assert_eq!(
        params(&unknown, "warning", 256),
        "38;5;221",
        "not in its list"
    );
}

#[test]
fn references_copy_a_style_or_take_its_colour() {
    let lib = library(json!({"t": {"extends": "aspect-dark", "roles": {
        "progress": "@label.error",
        "prompt": {"fg": "@selection", "underline": true},
        "marker": {"bg": "@accent"},
    }}}));
    let t = lib.load("t").unwrap();
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    assert_eq!(params(&t, "progress", 256), "1;38;5;203", "whole style");
    assert_eq!(
        params(&t, "prompt", 256),
        "4;38;5;24",
        "bg when there is no fg"
    );
    assert_eq!(params(&t, "marker", 256), "48;5;75");
}

#[test]
fn a_reference_cycle_or_dangling_reference_falls_back_to_the_parent() {
    let lib = library(json!({"t": {"extends": "aspect-dark", "roles": {
        "accent": "@heading",
        "muted": "@nothing",
    }}}));
    let t = lib.load("t").unwrap();
    let joined = t.warnings.join("\n");
    assert!(
        joined.contains("cycle (accent -> heading -> accent)"),
        "{joined}"
    );
    assert!(
        joined.contains("\"nothing\", which is not a role"),
        "{joined}"
    );
    assert_eq!(params(&t, "accent", 256), "38;5;75");
    assert_eq!(params(&t, "heading", 256), "1;38;5;75");
    assert_eq!(params(&t, "muted", 256), "38;5;247");
}

#[test]
fn a_cycle_among_new_roles_breaks_once() {
    let lib = library(json!({"t": {"extends": "aspect-dark", "roles": {
        "go.a": "@go.b",
        "go.b": "@go.a",
    }}}));
    let t = lib.load("t").unwrap();
    assert_eq!(t.warnings.len(), 1, "{:?}", t.warnings);
    assert_eq!(params(&t, "go.a", 256), "");
    assert_eq!(params(&t, "go.b", 256), "");
}

#[test]
fn invalid_values_warn_and_keep_the_parents() {
    let lib = library(json!({"t": {"extends": "aspect-dark", "roles": {
        "error": "purple",
        "warning": {"fg": "yellow", "blink": true},
        "success": 999,
        "bazel.warning": "tool",
        "acent": "red",
        "label.eror": "red",
        "go.command": "@highlight",
    }}}));
    let t = lib.load("t").unwrap();
    let joined = t.warnings.join("\n");
    for says in [
        "role \"error\": \"purple\" is not a colour",
        "unknown style key \"blink\"",
        "0-255",
        "role \"bazel.warning\": \"tool\" is not a colour",
        "\"acent\" is not a role; did you mean \"accent\"?",
        "\"label.eror\" is not a role; did you mean \"label.error\"?",
    ] {
        assert!(joined.contains(says), "{says}\n{joined}");
    }
    assert_eq!(t.warnings.len(), 6, "{joined}");
    assert_eq!(params(&t, "error", 256), "38;5;203");
    assert_eq!(params(&t, "warning", 256), "38;5;221");
    assert_eq!(params(&t, "success", 256), "38;5;79");
    assert_eq!(params(&t, "bazel.warning", 256), "1;38;5;221");
    assert_eq!(
        params(&t, "go.command", 256),
        "1;38;5;75",
        "a plugin's role"
    );
}

#[test]
fn a_bad_extends_falls_back_to_base() {
    let lib = library(json!({
        "a": {"extends": "b", "roles": {"accent": "red"}},
        "b": {"extends": "a"},
        "orphan": {"extends": "aspect-drak"},
    }));
    let a = lib.load("a").unwrap();
    let joined = a.warnings.join("\n");
    assert!(joined.contains("makes a cycle (a -> b -> a)"), "{joined}");
    assert_eq!(params(&a, "accent", 256), "31");
    let orphan = lib.load("orphan").unwrap();
    let joined = orphan.warnings.join("\n");
    assert!(joined.contains("did you mean \"aspect-dark\"?"), "{joined}");
    assert_eq!(params(&orphan, "accent", 256), "36");
}

#[test]
fn per_depth_values_and_the_16_colour_style() {
    let dark = Theme::aspect_dark();
    assert_eq!(params(&dark, "selection", 24), "48;2;44;74;110");
    assert_eq!(
        params(&dark, "selection", 256),
        "48;5;24",
        "pinned, not nearest"
    );
    assert_eq!(params(&dark, "selection", 16), "7", "the 16 style");
    assert_eq!(params(&dark, "error", 16), "91", "the pinned 16 name");
    assert_eq!(params(&dark, "muted", 16), "", "16 is the terminal's own");
    let lib = library(json!({"t": {"extends": "aspect-dark", "roles": {
        "accent": "#6aa6ff", "muted": 247, "link.hover": {"sgr": "4;38;5;153"}
    }}}));
    let t = lib.load("t").unwrap();
    assert_eq!(params(&t, "accent", 256), "38;5;75", "nearest");
    assert_eq!(params(&t, "accent", 16), "94", "nearest of the 16");
    assert_eq!(params(&t, "muted", 24), "38;5;247", "an index at 24-bit");
    assert_eq!(params(&t, "muted", 16), "90");
    assert_eq!(params(&t, "link.hover", 16), "4;38;5;153", "raw SGR as is");
}

#[test]
fn hyperlinks_decides_link_file() {
    let dark = Theme::aspect_dark();
    let with = Palette::new(dark.clone(), Some(Depth::Ansi256), true);
    let without = Palette::new(dark, Some(Depth::Ansi256), false);
    assert_eq!(with.params("link.file"), "4");
    assert_eq!(without.params("link.file"), "");
    assert_eq!(without.params("link"), "4;38;5;75", "true stays true");
    let var = |vars: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    };
    assert!(hyperlinks(var(&[("TERM_PROGRAM", "iTerm.app")]), false));
    assert!(hyperlinks(var(&[("TERM_PROGRAM", "ghostty")]), false));
    assert!(hyperlinks(var(&[("KITTY_WINDOW_ID", "1")]), false));
    assert!(hyperlinks(var(&[("WT_SESSION", "x")]), false));
    assert!(hyperlinks(var(&[("VTE_VERSION", "6003")]), false));
    assert!(!hyperlinks(var(&[("VTE_VERSION", "4205")]), false));
    assert!(!hyperlinks(
        var(&[("TERM_PROGRAM", "Apple_Terminal")]),
        false
    ));
    assert!(!hyperlinks(
        var(&[("TERM_PROGRAM", "vscode"), ("TMUX", "/tmp/t")]),
        false
    ));
    assert!(!hyperlinks(var(&[("TERM_PROGRAM", "vscode")]), true), "CI");
    assert!(!hyperlinks(var(&[]), false), "unknown");
}

#[test]
fn user_theme_files_are_found_by_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("night-dark.json"),
        r##"{"family": "night", "extends": "high-contrast-dark", "roles": {"accent": "#ff0000"}}"##,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("night-light.json"),
        r##"{"family": "night", "extends": "aspect-light"}"##,
    )
    .unwrap();
    std::fs::write(dir.path().join("broken.json"), "{").unwrap();
    std::fs::write(
        dir.path().join("aspect-dark.json"),
        r#"{"roles": {"error": "red"}}"#,
    )
    .unwrap();
    let lib = Library {
        config: BTreeMap::new(),
        dir: Some(dir.path().to_path_buf()),
        extra: vec![],
    };
    let night = lib
        .choose("night", Some(Appearance::Dark), &mut || Seen::Unknown)
        .unwrap();
    assert_eq!(night.name, "night-dark");
    assert_eq!(params(&night, "accent", 256), "38;5;196");
    assert_eq!(
        params(&night, "muted", 256),
        "38;5;251",
        "from high-contrast-dark"
    );
    let light = lib
        .choose("night", None, &mut || Seen::Known(Appearance::Light))
        .unwrap();
    assert_eq!(light.name, "night-light");
    assert!(lib.load("broken").unwrap_err().contains("failed to parse"));
    assert_eq!(
        params(&lib.load("aspect-dark").unwrap(), "error", 256),
        "31",
        "a user file shadows"
    );
    let err = lib
        .choose("nigth", None, &mut || Seen::Unknown)
        .unwrap_err();
    assert!(err.contains("did you mean \"night\"?"), "{err}");
}

/// The theme `select` picks with the probe answering `seen`, whether it
/// follows the terminal, and how many times the probe was asked.
fn pick(
    aspect_theme: Option<&str>,
    config: Option<Value>,
    appearance: Option<Appearance>,
    seen: Seen,
) -> (String, bool, usize) {
    let mut asked = 0;
    let theme = sel(
        aspect_theme,
        config.as_ref(),
        appearance,
        &Library::default(),
        &mut || {
            asked += 1;
            seen
        },
    );
    (theme.name, theme.follows_terminal, asked)
}

const DARK: Seen = Seen::Known(Appearance::Dark);
const LIGHT: Seen = Seen::Known(Appearance::Light);

#[test]
fn precedence_aspect_theme_then_config_then_the_default_family() {
    let ansi = Some(json!("ansi"));
    assert_eq!(
        pick(Some("aspect-light"), ansi.clone(), None, DARK),
        ("aspect-light".into(), false, 0)
    );
    assert_eq!(
        pick(Some("colorblind"), ansi.clone(), None, LIGHT),
        ("colorblind-light".into(), false, 1)
    );
    assert_eq!(pick(None, ansi, None, DARK), ("ansi".into(), false, 0));
    assert_eq!(
        pick(None, None, None, DARK),
        ("aspect-dark".into(), false, 1)
    );
    assert_eq!(
        pick(None, None, None, LIGHT),
        ("aspect-light".into(), false, 1)
    );
    assert_eq!(
        pick(Some("mono"), None, None, DARK),
        ("mono".into(), false, 0)
    );
}

#[test]
fn the_appearance_picks_the_variant_and_whether_colours_are_fixed() {
    let family = Some(json!("high-contrast"));
    let system_light = Seen::System(Appearance::Light);
    assert_eq!(
        pick(None, family.clone(), None, DARK),
        ("high-contrast-dark".into(), false, 1)
    );
    assert_eq!(
        pick(None, family.clone(), None, system_light),
        ("high-contrast-light".into(), true, 1)
    );
    assert_eq!(
        pick(None, family.clone(), None, Seen::Unknown),
        ("high-contrast-dark".into(), true, 1)
    );
    assert_eq!(
        pick(None, family, Some(Appearance::Light), Seen::Unknown),
        ("high-contrast-light".into(), false, 0),
        "forced: fixed colours, no probe"
    );
    assert_eq!(
        pick(None, Some(json!("colorblind-light")), None, DARK),
        ("colorblind-light".into(), false, 0),
        "a variant pins its appearance"
    );
    let lib = library(json!({
        "mine-dark": {"family": "mine", "extends": "ansi"},
        "mine-light": {"family": "mine", "extends": "mono"},
    }));
    let theme = sel(None, Some(&json!("mine")), None, &lib, &mut || LIGHT);
    assert_eq!(theme.name, "mine-light", "a family in config.json's themes");
}

#[test]
fn an_unknown_background_puts_the_follow_terminal_roles_on_ansi() {
    let theme = sel(None, None, None, &Library::default(), &mut || Seen::Unknown);
    assert!(theme.follows_terminal);
    for (role, want) in [
        ("muted", "90"),
        ("error", "31"),
        ("label.error", "1;31"),
        ("label.info", "1;90"),
        ("tip.important", "1;35"),
        ("selection", "7"),
        ("hover", "4"),
        ("search.match", "1;4"),
    ] {
        for d in [24, 256, 16] {
            assert_eq!(params(&theme, role, d), want, "{role} at {d}");
        }
    }
    assert_eq!(params(&theme, "accent", 256), "38;5;75", "fixed");
}

#[test]
fn an_unknown_name_falls_back_to_the_default_family_with_a_warning() {
    let lib = Library::default();
    let unknown = sel(None, Some(&json!("drak")), None, &lib, &mut || DARK);
    assert_eq!(unknown.name, "aspect-dark");
    assert!(
        unknown.warnings[0].starts_with("config.json \"theme\": no theme or family"),
        "{:?}",
        unknown.warnings
    );
    let unknown = sel(Some("colourblind"), None, None, &lib, &mut || LIGHT);
    assert_eq!(unknown.name, "aspect-light");
    assert!(
        unknown.warnings[0].contains("did you mean \"colorblind\"?"),
        "{:?}",
        unknown.warnings
    );
    let bad = sel(None, Some(&json!(7)), None, &lib, &mut || DARK);
    assert_eq!(bad.name, "aspect-dark");
    assert_eq!(bad.warnings.len(), 1);
}

#[test]
fn the_bazel_setting() {
    assert_eq!(DEFAULT_BAZEL, Bazel::Shared);
    assert_eq!(Palette::aspect_dark(Depth::Ansi256).bazel(), Bazel::Shared);
    let mut theme = Theme::aspect_dark();
    theme.bazel = Some(Bazel::Tool);
    assert_eq!(
        Palette::new(theme, Some(Depth::Ansi256), false).bazel(),
        Bazel::Tool
    );
    assert_eq!(Bazel::parse(&json!("tool")), Ok(Bazel::Tool));
    assert!(Bazel::parse(&json!("bazel")).is_err());
}

#[test]
fn colorfgbg() {
    assert_eq!(background(Some("0;15")), Some(Appearance::Light));
    assert_eq!(background(Some("0;7")), Some(Appearance::Light));
    assert_eq!(background(Some("0;9")), Some(Appearance::Light));
    assert_eq!(background(Some("15;0")), Some(Appearance::Dark));
    assert_eq!(
        background(Some("7;8")),
        Some(Appearance::Dark),
        "8 is bright black"
    );
    assert_eq!(background(Some("0;default;15")), Some(Appearance::Light));
    assert_eq!(background(Some("15;default")), None);
    assert_eq!(background(None), None);
    assert_eq!(
        background(Some("15;234")),
        Some(Appearance::Dark),
        "a dark grey of the 256"
    );
    assert_eq!(
        background(Some("15;16")),
        Some(Appearance::Dark),
        "the cube's black"
    );
    assert_eq!(
        background(Some("0;255")),
        Some(Appearance::Light),
        "a light grey of the 256"
    );
    assert_eq!(
        background(Some("0;231")),
        Some(Appearance::Light),
        "the cube's white"
    );
    assert_eq!(
        background(Some("0;21")),
        Some(Appearance::Dark),
        "pure blue is dark"
    );
}

#[test]
fn no_depth_writes_no_sgr_and_mono_no_colour_or_dim() {
    let plain = Palette::plain();
    for role in role_names() {
        assert_eq!(plain.params(&role), "", "{role}");
        assert_eq!(plain.paint(&role, "x"), "x");
    }
    let mono = Palette::new(
        Theme::builtin("mono").unwrap(),
        Some(Depth::TrueColor),
        true,
    );
    for role in role_names() {
        let style = mono.style(&role).unwrap();
        assert!(
            style.fg.is_none() && style.bg.is_none() && !style.dim,
            "{role}: {style:?}"
        );
    }
    assert_eq!(mono.params("label.error"), "1");
    assert_eq!(mono.params("active"), "7");
}

#[test]
fn a_palette_paints_and_splits_a_role() {
    let p = Palette::aspect_dark(Depth::Ansi256);
    assert_eq!(p.sgr("error"), "\x1b[38;5;203m");
    assert_eq!(
        p.paint("label.error", "ERROR"),
        "\x1b[1;38;5;203mERROR\x1b[0m"
    );
    assert_eq!(p.paint("text", "x"), "x");
    let search = p.entry("search.match").unwrap();
    assert_eq!(search.colour_sgr(false), "\x1b[38;5;230m");
    assert_eq!(search.colour_sgr(true), "\x1b[48;5;58m");
    assert_eq!(p.entry("muted").unwrap().colour_sgr(true), "");
    let default = json!("@highlight");
    assert_eq!(
        p.entry_or("go.command", Some(&default)).unwrap().params,
        "1;38;5;75"
    );
    assert!(
        p.entry_or("go.command", None)
            .unwrap_err()
            .contains("not a role")
    );
    assert!(
        p.entry_or("eror", None)
            .unwrap_err()
            .contains("did you mean \"error\"?")
    );
}

#[test]
fn config_values() {
    assert_eq!(parse_depth(&json!("auto")), Ok(None));
    assert_eq!(parse_depth(&json!(24)), Ok(Some(Some(Depth::TrueColor))));
    assert_eq!(parse_depth(&json!(16)), Ok(Some(Some(Depth::Ansi16))));
    assert_eq!(parse_depth(&json!(0)), Ok(Some(None)));
    assert!(parse_depth(&json!(8)).is_err());
    assert_eq!(Appearance::parse_setting(&json!("auto")), Ok(None));
    assert_eq!(
        Appearance::parse_setting(&json!("light")),
        Ok(Some(Appearance::Light))
    );
    assert!(Appearance::parse_setting(&json!("any")).is_err());
}

fn layer(source: &str, theme: Option<Value>, roles: Value) -> Layer {
    Layer {
        source: source.into(),
        theme,
        roles: match roles {
            Value::Object(m) => m,
            _ => Default::default(),
        },
        ..Layer::default()
    }
}

/// The user's `config.json`, the user's `config.axl`, the repository's: the
/// first that names a theme picks it, and every layer's role overrides
/// apply, the higher on top.
#[test]
fn layers_pick_the_theme_and_stack_their_role_overrides() {
    let lib = Library::default();
    let layers = [
        layer(
            "config.json",
            Some(json!("colorblind")),
            json!({"accent": "#ff0000"}),
        ),
        layer(
            "~/.aspect/config.axl",
            None,
            json!({"error": 196, "warning": 220}),
        ),
        layer(
            ".aspect/config.axl",
            Some(json!("high-contrast")),
            json!({"error": "#00ff00", "success": 46, "muted": "@nothing"}),
        ),
    ];
    let theme = select(None, &layers, &lib, &mut || DARK);
    assert_eq!(theme.name, "colorblind-dark", "the user's pick wins");
    assert_eq!(params(&theme, "accent", 256), "38;5;196");
    assert_eq!(
        params(&theme, "error", 256),
        "38;5;196",
        "the user's config.axl over the repo"
    );
    assert_eq!(
        params(&theme, "success", 256),
        "38;5;46",
        "the repo's applies to the user's pick"
    );
    assert_eq!(params(&theme, "warning", 256), "38;5;220");
    assert_eq!(
        params(&theme, "label.error", 256),
        "1;38;5;196",
        "derived roles follow"
    );
    assert_eq!(
        params(&theme, "muted", 256),
        "38;5;247",
        "a bad override keeps the value below"
    );
    assert!(
        theme
            .warnings
            .iter()
            .any(|w| w.starts_with(".aspect/config.axl \"roles\": role \"muted\"")),
        "{:?}",
        theme.warnings
    );
    let theme = select(Some("ansi"), &layers, &lib, &mut || DARK);
    assert_eq!(theme.name, "ansi", "ASPECT_THEME over every layer");
    assert_eq!(
        params(&theme, "error", 256),
        "38;5;196",
        "its overrides still apply"
    );
}

#[test]
fn overrides_by_appearance_and_on_an_unknown_background() {
    let lib = Library::default();
    let roles = json!({
        "accent": 33,
        "dark": {"error": {"rgb": "#ff0000", "256": 196, "ansi": "bright-red"}},
        "light": {"error": 124},
    });
    let pick = |seen: Seen| {
        let mut probe = || seen;
        select(
            None,
            &[layer("config.json", Some(json!("aspect")), roles.clone())],
            &lib,
            &mut probe,
        )
    };
    let dark = pick(DARK);
    assert_eq!(params(&dark, "error", 256), "38;5;196");
    assert_eq!(params(&dark, "accent", 256), "38;5;33");
    let light = pick(LIGHT);
    assert_eq!(params(&light, "error", 256), "38;5;124");
    assert_eq!(params(&light, "accent", 256), "38;5;33");
    let unknown = pick(Seen::Unknown);
    assert_eq!(
        params(&unknown, "error", 256),
        "91",
        "the override's own ansi tier"
    );
    let bad = select(
        None,
        &[layer("config.json", None, json!({"dark": "red"}))],
        &lib,
        &mut || DARK,
    );
    assert!(
        bad.warnings[0].contains("\"roles.dark\" must be an object"),
        "{:?}",
        bad.warnings
    );
}

/// One namespace: `config.json`'s `themes`, then theme files, then each
/// `config.axl`'s, the first to define a name winning.
#[test]
fn custom_themes_and_families_from_every_source() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("shared-dark.json"),
        r##"{"extends": "aspect-dark", "roles": {"accent": "#00ff00"}}"##,
    )
    .unwrap();
    let mut user_axl = BTreeMap::new();
    user_axl.insert("shared-dark".to_string(), json!({"extends": "ansi"}));
    user_axl.insert(
        "team-dark".to_string(),
        json!({"extends": "aspect-dark", "roles": {"accent": 201}}),
    );
    user_axl.insert(
        "team-light".to_string(),
        json!({"extends": "aspect-light", "roles": {"accent": 21}}),
    );
    let mut repo_axl = BTreeMap::new();
    repo_axl.insert("team-dark".to_string(), json!({"extends": "mono"}));
    let lib = Library {
        config: BTreeMap::new(),
        dir: Some(dir.path().to_path_buf()),
        extra: vec![user_axl, repo_axl],
    };
    let pick = |name: &str, seen: Seen| {
        let mut probe = || seen;
        select(
            None,
            &[layer("config.json", Some(json!(name)), json!({}))],
            &lib,
            &mut probe,
        )
    };
    assert_eq!(
        params(&pick("shared", DARK), "accent", 256),
        "38;5;46",
        "a file over config.axl"
    );
    let team = pick("team", DARK);
    assert_eq!(team.name, "team-dark");
    assert_eq!(
        params(&team, "accent", 256),
        "38;5;201",
        "the user's config.axl over the repo's"
    );
    assert_eq!(params(&pick("team", LIGHT), "accent", 256), "38;5;21");
}

#[test]
fn config_json_layers_in_every_shape() {
    let mut warnings = Vec::new();
    let from = |theme: Value,
                appearance: Option<Value>,
                bazel: Option<Value>,
                warnings: &mut Vec<String>| {
        Layer::from_config_json(
            "~/.aspect/config.json",
            Some(&theme),
            appearance.as_ref(),
            bazel.as_ref(),
            BTreeMap::new(),
            warnings,
        )
    };
    let named = from(
        json!("colorblind"),
        Some(json!("light")),
        Some(json!("tool")),
        &mut warnings,
    );
    assert_eq!(named.theme, Some(json!("colorblind")));
    assert_eq!(named.appearance, Some(Some(Appearance::Light)));
    assert_eq!(named.bazel, Some(Bazel::Tool));
    let selection = from(
        json!({"name": "aspect", "appearance": "dark", "bazel": "shared", "roles": {"error": 196}}),
        Some(json!("light")),
        None,
        &mut warnings,
    );
    assert_eq!(selection.theme, Some(json!("aspect")));
    assert_eq!(
        selection.appearance,
        Some(Some(Appearance::Dark)),
        "the selection's own wins"
    );
    assert_eq!(selection.bazel, Some(Bazel::Shared));
    assert_eq!(selection.roles.get("error"), Some(&json!(196)));
    let only_roles = from(json!({"roles": {"error": 196}}), None, None, &mut warnings);
    assert_eq!(only_roles.theme, None, "overrides for the default family");
    let inline = from(
        json!({"extends": "aspect-dark", "roles": {"accent": 1}}),
        None,
        None,
        &mut warnings,
    );
    assert!(matches!(inline.theme, Some(Value::Object(_))));
    assert!(warnings.is_empty(), "{warnings:?}");
    from(
        json!({"name": 3, "roles": [], "colour": 1}),
        Some(json!("dim")),
        Some(json!("x")),
        &mut warnings,
    );
    assert_eq!(warnings.len(), 5, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .all(|w| w.starts_with("~/.aspect/config.json \"")),
        "{warnings:?}"
    );
}
