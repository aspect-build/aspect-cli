# Colour themes

Every colour the Aspect CLI draws is a **role**: `error`, `muted`,
`label.warning`, `bazel.progress`, and so on. A **theme** says what each role
looks like. The CLI's own lines (`ERROR:`, tips, the phase header, help, the
console's recap and links) use the roles, and so do Bazel's own labels as the
console passes them through.

The model, the built-in themes and the resolver live in `crates/aspect-theme`.
The process's theme is resolved once, in `crates/axl-runtime/src/theme.rs`.

## Built-in themes

Each theme is a family with a dark and a light variant. Two themes are single
files that suit any background.

| Family | Variants | For |
|---|---|---|
| `aspect` (the default) | `aspect-dark`, `aspect-light` | everyone |
| `high-contrast` | `high-contrast-dark`, `high-contrast-light` | low vision, bright rooms, projectors: text at 7:1 or better |
| `colorblind` | `colorblind-dark`, `colorblind-light` | protanopia and deuteranopia: Okabe-Ito hues, no red-green pair |
| `ansi` | `ansi` | the terminal's own 16 colours everywhere |
| `mono` | `mono` | bold, underline and reverse only. The full-screen console uses it under `NO_COLOR` |

The files are in `crates/aspect-theme/themes/`.

## Picking one, and changing it

Three places set the theme, from the user's own to the team's:

1. **`~/.aspect/config.json`**, the user's:

   ```json
   {
     "theme": {
       "name": "colorblind",
       "appearance": "auto",
       "bazel": "shared",
       "roles": {
         "error": "#FF5555",
         "tip.important": { "rgb": "#C084FC", "256": 177, "16": "magenta" },
         "light": { "error": 160 }
       }
     },
     "color_depth": "auto",
     "themes": {
       "mine-dark": { "extends": "aspect-dark", "roles": { "accent": "#FF8A3D" } },
       "mine-light": { "extends": "aspect-light", "roles": { "accent": "#B4520A" } }
     }
   }
   ```

2. **`~/.aspect/config.axl`**, also the user's, through `ctx.theme`.
3. **The repository's `.aspect/config.axl`**, for everyone working in it:

   ```python
   def config(ctx: ConfigContext):
       ctx.theme.select(name = "colorblind", appearance = "auto", bazel = "shared")
       ctx.theme.roles({"accent": "#FF8A3D"})
       ctx.theme.roles({"error": 160}, appearance = "light")
       ctx.theme.define("team-dark", {"extends": "aspect-dark", "roles": {"accent": 208}})
       ctx.theme.define("team-light", {"extends": "aspect-light", "roles": {"accent": 166}})
   ```

| Setting | `config.json` | `config.axl` | Values | Default |
|---|---|---|---|---|
| The theme | `theme` as a name, or `theme.name` | `ctx.theme.select(name = …)` | a family (`aspect`, `high-contrast`, `colorblind`, `ansi`, `mono`, or one of your own), or a variant (`aspect-light`) | `aspect` |
| Which variant of a family | `appearance`, or `theme.appearance` | `select(appearance = …)` | `auto`, `light`, `dark` | `auto` |
| Bazel's own colours | `bazel`, or `theme.bazel` | `select(bazel = …)` | `shared`: Bazel's labels, status line and test results take the theme's `bazel.*` roles. `tool`: Bazel's colours pass through untouched | the theme's, else `shared` |
| Role overrides | `theme.roles` | `ctx.theme.roles({...})` | role names to role values; under `dark` or `light` (`appearance =` in AXL), only for that appearance | |
| Themes of your own | `themes` | `ctx.theme.define(name, {...})` | a theme by name; name two `<family>-dark` and `<family>-light` to make a family | |
| Colour depth | `color_depth` | | `auto`, `24`, `256`, `16`, or `0` for none | `auto`: 24-bit when `COLORTERM` is `truecolor` or `24bit`, 16 for a `TERM` that only does 16, else 256 |

In `config.json`, `theme` may also be a theme written in place: an object with
`extends` (or `family`), as a theme file is written. A `theme` object with only
`roles` overrides the default family.

### Precedence

Whether a stream is coloured at all comes first:

1. A stream that isn't colour-capable (not a terminal or a CI log that renders
   ANSI, `TERM=dumb`, `CLICOLOR=0`) gets plain text.
2. `NO_COLOR` set and not empty: plain text, no SGR at all.

Then, personal before team, the first that says wins:

3. `ASPECT_THEME`, a family or a variant, for this run.
4. The user's `~/.aspect/config.json`.
5. The user's `~/.aspect/config.axl`.
6. The repository's `.aspect/config.axl`.
7. The built-in default, the `aspect` family.

That holds for the theme, `appearance` and `bazel` each. **Role overrides
stack**: every source's apply to whichever theme is selected, the repository's
first and the user's on top, so a user who picks another theme still gets the
team's overrides of roles they leave alone. Within a source, the overrides for
the theme's own appearance (`dark` or `light`) come after those for every
appearance.

**Theme names are one namespace**, searched in order: `config.json`'s
`themes`, `~/.aspect/themes/<name>.json`, the user's `config.axl`, the
repository's `config.axl`, the built-ins. A name defined in two places is the
first's; `ASPECT_DEBUG=1` logs the clash.

An invalid setting, role name or value is a warning naming its source (the
file, and the key or `ctx.theme` call), and that setting or role falls back to
the layer below. It never fails a run. A `ctx.theme` call given the wrong kind
of argument (`roles` that is not a dict) fails that config as any bad call
does.
### Light, dark, and an unknown background

A family's variant is the forced `appearance`, or naming a variant, when there
is one. With `appearance` `auto`, the CLI looks in this order:

1. A CI log viewer known to be dark (`GITHUB_ACTIONS`, `BUILDKITE`), or
   `COLORFGBG` (of the 16 colours, 7 and 9 to 15 are light; a 256-colour
   index is light when it is lighter than a mid grey). The background is
   known, and the theme's fixed colours apply.
2. What the terminal itself reports: the full-screen console asks it (OSC 11)
   when it takes the screen, and the theme is resolved again with the answer
   (`learn_background` in `crates/axl-runtime/src/theme.rs`). Known, as above.
3. The system's light or dark preference (macOS `AppleInterfaceStyle`, read in
   process), except on a CI host or over SSH, where the system is not the
   viewer's. It picks the variant, but a terminal often differs from the
   system, so the background stays unknown.
4. Nothing: the dark variant, the background unknown.

With the background unknown, the roles in the theme's `follow_terminal` list
(status colours, `muted`, `subtle`, `tip.important`, and the full-screen
backgrounds) take the terminal's own ANSI colours at every depth, so they
follow whatever palette the terminal has. The other roles keep their fixed
values. `ctx.std.theme.info()["follows_terminal"]` says when this is so.

The system appearance is probed only when colour is on, the theme is a family
and `appearance` is `auto`. Linux and Windows have no probe yet; there the
appearance is unknown unless `COLORFGBG` or a CI host says otherwise.

## Writing a theme

A theme is a JSON file in `~/.aspect/themes/<name>.json`, or an entry in
`config.json`'s `themes`:

```json
{
  "family": "mine",
  "appearance": "dark",
  "extends": "aspect-dark",
  "roles": {
    "accent": "#FF8A3D",
    "label.warning": { "fg": "#FFB000", "bold": true }
  }
}
```

Save it as `mine-dark.json` (and a `mine-light.json`) and `"theme": "mine"`
picks between them; `"theme": "mine-dark"` pins the dark one. A file named
`<name>.json` with no variants, like `ansi`, is used as it is.

| Key | Meaning |
|---|---|
| `extends` | The parent theme. Without it, the hidden `_base`, which defines every derived role from the core ones. A child's role replaces the parent's whole role |
| `family`, `appearance` | `appearance` is `dark`, `light` or `any` |
| `bazel` | `shared` or `tool`, as in `config.json` |
| `follow_terminal` | The roles that take ANSI colours when the background is unknown |
| `syntax_theme` | A syntect theme name for the full-screen console's code peek |
| `roles` | Role values, below |

`bazel`, `follow_terminal` and `syntax_theme` inherit unless the child sets them.

A **role value** is a colour (the foreground), `"@role"` (that role's whole
style), `"none"`, or a style object with any of `fg`, `bg`, `bold`, `dim`,
`italic`, `underline`, `reverse` (`true`, `false`, or `"hyperlinks"` for "on
only where the terminal opens OSC 8 links"), `sgr` (raw SGR parameters, used as
they are), and `16` (a whole style used at 16 colours and when following the
terminal).

A **colour** is `"#rrggbb"` or `"#rgb"`, a 256-colour index, an ANSI name
(`"red"`, `"bright-black"`, …, the terminal's own at every depth),
`"default"`, `"@role"` (that role's colour: its fg, else its bg), or by depth:
`{"rgb": "#979ea8", "256": 247, "16": "default", "ansi": "bright-black"}`. A
missing depth comes from the next richer one, and `ansi`, the value when
following the terminal, defaults to `16`.

Nothing fails a run over a theme. A value that doesn't parse, a reference to a
role that doesn't exist, a reference cycle, or an `extends` cycle is a warning,
and the role keeps its parent's value. A misspelt role gets a "did you mean".

## Roles

**Core**, which a theme author fills in: `text` (never painted), `muted`
(routine and secondary text), `subtle` (chrome: separators, borders), `accent`
(Aspect's voice), `info` (`@muted`), `success`, `warning`, `error`,
`link.hover`, `selection` and `hover` (full-screen backgrounds).

**Derived**, from the core roles in `_base`:

| Role | Used for |
|---|---|
| `strong` | names (flags, tasks), list headers |
| `heading` | headings |
| `title` | report titles, help section headings |
| `highlight` | the `emoji` layout's phase label and verdict, plugin highlights |
| `progress` | the CLI's own progress bars |
| `prompt` | interactive questions |
| `marker`, `scrollbar`, `border` | the full-screen console's peek marker and current match, scrollbar thumb, and boxes |
| `status_bar` | the full-screen console's top bar: `{reverse, bold}` in every theme, so it reads as a bar on any background |
| `active` | the shown tab, the chosen row, a toggle that is on (reverse, without bold) |
| `hover`, `selection` | the tab under the pointer; the mouse selection |
| `search.current`, `search.match` | the current search match; the other matches |
| `header` | a heading drawn without a fill |
| `link` | a link the CLI makes to a web page (`https://…`), on any line, Bazel's and tools' included: the accent, underlined, so it reads as a hyperlink |
| `link.file` | a link the CLI makes to a file, which opens it in an editor: underlined only where the terminal is known to open OSC 8 links, else the text's own style |
| `link.quiet` | the `<url>` written beside a file's link on a terminal that opens no OSC 8 link (Terminal.app), and kept log paths |
| `label.debug`, `label.info`, `label.success`, `label.warning`, `label.error` | `DEBUG:`, `INFO:`, "3 delivered", `WARNING:`, `ERROR:`; `label.error` also the error recap's `── Error recap (N) ──` rule |
| `tip.label`, `tip.id`, `tip.warning`, `tip.important`, `tip.suggestion`, `tip.note` | a tip's `TIP`, `[id]` and severity word |

**The task's lines**, in the default `full` console layout, each defaulting
to a role above, so no theme has to set them:

```
▸ Running build task

▸ Build
▹ Spawning bazel build
...
✓ Passed build task in 1m 12s · Setup 0.4s · Build 1m 11s
```

| Role | Default | Used for |
|---|---|---|
| `task` | `{fg: @accent, bold}` | the task line's `▸` and `Running` |
| `task.label` | `@text` | what runs: `build task` |
| `phase` | `{fg: @accent, bold}` | a phase heading's `▸` |
| `phase.title` | `@highlight` | a phase heading's title: `Build` |
| `phase.update` | `@muted` | an update's `▹` |
| `phase.text` | `@text` | an update's text |
| `verdict.success`, `verdict.failure`, `verdict.cancelled` | `@success`, `@error`, `@warning` | the verdict's `✓` or `✗`, `cancelled` for an interrupted run |
| `verdict.verb` | `@highlight` | the verdict's `Passed`, `Flagged` or `Failed` |

**Bazel's own**, used when `bazel` is `shared`. Only the opening colour of the
word changes; the text, and tools' output, are never touched.

| Role | Bazel's |
|---|---|
| `bazel.error`, `bazel.warning`, `bazel.info`, `bazel.debug` | `ERROR:`, `WARNING:`, `INFO:`, `DEBUG:` labels |
| `bazel.progress` | `[n / m]`, `Loading:`, `Analyzing:` |
| `bazel.status.success`, `bazel.status.failed` | the end-of-build status: the curses row's `INFO:` and `FAILED:`, and the message that stays, `INFO: Build completed…` and `ERROR: Build did NOT complete…`; a message that reports a test `FAILED` is a failure, and the `INFO:` row after it takes that message's role |
| `bazel.fail` | a test's `FAIL:` and `TIMEOUT:` headers |
| `bazel.test.passed`, `bazel.test.flaky`, `bazel.test.failed`, `bazel.test.no_status` | the test summary's `PASSED`, `FLAKY`, `FAILED`/`TIMEOUT`/`FAILED TO BUILD`, `NO STATUS`; the `PASSED:` and `FLAKY:` headers; the progress row's `N failed` and `last test:`; the footer's `N fails locally` |

Only Bazel's own lines change: a message's header, the first line after an
action's output that starts with Bazel's coloured label, and status, progress
and summary rows outside any action's or test's output. A tool's lines that look
like Bazel's keep the tool's colours. When the theme writes no SGR at all
(`color_depth` 0), Bazel's colours pass through as with `tool`. Bazel's bold red
is matched as Bazel writes it and as a plain-lines console (a pipe with colour,
such as CI) merges it.

Under `--test_output=streamed`, a test's output has no header, so from Bazel's
announcement on nothing can be told from it: Bazel's labels between streamed
outputs keep Bazel's colours. From Bazel's closing `Build completed…` or `Build
did NOT complete…` message, straight after its `Elapsed time:` and `N
processes:` lines, the lines are Bazel's again, and that message, the test
summary and its footer take the theme's roles. A nested Bazel in a test writes
those same lines, and Bazel draws a streamed test's lines as it draws its own,
so in every mode a nested Bazel's own closing block is taken for Bazel's.

Links are the one exception to "azure only on the CLI's lines": a web URL the
console links on Bazel's or a tool's line is in `link`, so it reads as a
hyperlink. A link a tool wrote itself (an OSC 8 link of its own, such as gcc's
to a warning's option) is left exactly as the tool wrote it.

The recolouring is the console's first stage, before any other: the built-in
ones and a user's `bazel.output.rewrite`, `intercept`, `drop` and the
rest all see the theme's colours. **Change colours with the theme, not with a
rewrite**: override roles in `~/.aspect/config.json` or a `config.axl`, or write
a theme. In `shared` mode a `rewrite` pattern that matches Bazel's own codes
(`\x1b[35mWARNING`) no longer finds them. Each line is classified from Bazel's
own bytes before any stage runs, so collapsing, the error recap and the other
stages that tell Bazel's messages apart work the same in either mode.

**Syntax**: `syntax.text`, `.comment`, `.keyword`, `.string`, `.constant`,
`.type`, `.function`, `.variable`, `.escape`.

A plugin may use a role of its own (`go.command`) with a default; a theme can
then set it.

## In AXL

`ctx.std.theme`, or `load("@std//theme.axl", "theme")` where there is no
`ctx` (a module's top level). Every method reads the run's theme when it is
called, so it follows the theme when the configs or the terminal's reported
background change it. `stream` is `"stderr"` (the default), `"stdout"`, or a
bool already decided: `True` for a surface that draws regardless, such as a
full-screen console (under `NO_COLOR` that is `mono`). When neither stream
takes colour for another reason (both piped, `TERM=dumb`), the run's theme
writes no SGR, so `True` gives `""` too.

| Method | Returns |
|---|---|
| `sgr(role, stream = "stderr", default = None)` | the escape opening `role`, or `""` when the stream takes no colour |
| `paint(role, text, stream = "stderr", default = None)` | `text` in `role` and a reset, or `text` |
| `fg(role, ...)`, `bg(role, ...)` | only the foreground, or the background, as an escape |
| `params(role, default = None)` | the SGR parameters (`"1;38;5;203"`), ungated |
| `depth(stream = "stderr")` | 24, 256 or 16; 0 when the stream takes no colour |
| `info()` | `name`, `family`, `appearance`, `background` (`dark` or `light` when the terminal's is known, else `None`), `follows_terminal`, `bazel`, `hyperlinks`, `syntax_theme`, `roles` |
| `reset` | `"\x1b[0m"` |
| `testonly_pin(name, depth = 256, appearance = "dark")` | pins the run's theme to a built-in, its background known, for a test that asserts the escapes; only from a `*_test.axl` file, and it fails anywhere else |

`default` is a role value (`"@highlight"`) for a role the theme may not define;
an unknown role with no default fails.

The builtins use `private/lib/ansi.axl`, whose constants (`ansi.ERROR`,
`ansi.MUTED`, `ansi.ERROR_LABEL`, …) are role names that `ansi.style(text, role,
enabled)` and `ansi.code(role, enabled)` resolve the same way.

## Migrating from `ASPECT_CLI_HIGHLIGHT_COLOR`

`ASPECT_CLI_HIGHLIGHT_COLOR` is gone. The phase label it coloured is the
`highlight` role. Where you set, say, `ASPECT_CLI_HIGHLIGHT_COLOR="38;5;208"`,
set the role instead, in `~/.aspect/config.json`:

```json
{ "theme": { "roles": { "highlight": { "sgr": "38;5;208" } } } }
```

or, for everyone in a repository, in `.aspect/config.axl`:

```python
ctx.theme.roles({"highlight": {"fg": 208, "bold": True}})
```

An empty `ASPECT_CLI_HIGHLIGHT_COLOR` (no highlight) is `"highlight": "none"`.
