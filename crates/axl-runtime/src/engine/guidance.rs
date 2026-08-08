//! `guidance(...)` — prose an agent can fetch at runtime.
//!
//! Tasks and features describe *what can be run*. Guidance describes *how to do
//! something*, and is consumed before anything runs — so unlike a tip it is not
//! scoped to a task invocation, and unlike a `description` it is too long to
//! live in `--help`.
//!
//! Bodies are markdown files rather than inline strings: a few hundred lines of
//! prose inside a Starlark literal is neither reviewable nor diffable. The file
//! is resolved and existence-checked at eval, then read only when a caller asks
//! for that topic by id — the index carries a byte count, and paying to read
//! every body to answer it would defeat the point.

use std::fmt::{self, Display};
use std::path::{Path, PathBuf};

use allocative::Allocative;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::UnpackList;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{NoSerialize, ProvidesStaticType, StarlarkValue, starlark_value};

use super::store::Env;

/// How far a topic can be trusted. Mirrors the `stability` argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Allocative)]
pub enum GuidanceStability {
    Stable,
    Preview,
    Deprecated,
}

impl GuidanceStability {
    const VALID: &'static [&'static str] = &["stable", "preview", "deprecated"];

    fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "stable" => Ok(Self::Stable),
            "preview" => Ok(Self::Preview),
            "deprecated" => Ok(Self::Deprecated),
            other => Err(anyhow::anyhow!(
                "unknown guidance stability {other:?}; valid: {}",
                Self::VALID.join(", ")
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
            Self::Deprecated => "deprecated",
        }
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Guidance {
    id: String,
    title: String,
    summary: String,
    /// Absolute path to the markdown body, resolved against the defining
    /// `.axl` at eval. Read on demand — see the module docs.
    body_file: PathBuf,
    /// Repo-relative globs hinting when this topic is relevant. A hint for
    /// topic selection, never a gate: we are not in the business of detecting
    /// project types authoritatively, and a polyglot repo would defeat us.
    applies_to: Vec<String>,
    stability: GuidanceStability,
    /// The `.axl` that declared it, for the `defined_in` label.
    path: PathBuf,
}

impl Guidance {
    /// Fields are private so a topic cannot be half-built; this is the only
    /// way in, for the `guidance()` global and for tests.
    pub fn new(
        id: String,
        title: String,
        summary: String,
        body_file: PathBuf,
        applies_to: Vec<String>,
        stability: GuidanceStability,
        path: PathBuf,
    ) -> Self {
        Self {
            id,
            title,
            summary,
            body_file,
            applies_to,
            stability,
            path,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
    pub fn body_file(&self) -> &Path {
        &self.body_file
    }
    pub fn applies_to(&self) -> &[String] {
        &self.applies_to
    }
    pub fn stability(&self) -> GuidanceStability {
        self.stability
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Size of the body, for the index. `None` if it has become unreadable
    /// since eval checked it.
    pub fn body_len(&self) -> Option<u64> {
        std::fs::metadata(&self.body_file).ok().map(|m| m.len())
    }

    /// The body itself. Only called for a topic a caller named.
    pub fn read_body(&self) -> std::io::Result<String> {
        std::fs::read_to_string(&self.body_file)
    }
}

impl Display for Guidance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<guidance {}>", self.id)
    }
}

#[starlark_value(type = "Guidance")]
impl<'v> StarlarkValue<'v> for Guidance {}

starlark_simple_value!(Guidance);

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    const Guidance: StarlarkValueAsType<Guidance> = StarlarkValueAsType::new();

    /// Declares a guidance topic — prose an agent fetches with
    /// `aspect describe --guidance=<id>`.
    ///
    /// ```python
    /// bazelify_go = guidance(
    ///     id = "bazelify-go",
    ///     title = "Bazel-ify a Go repository",
    ///     summary = "bzlmod + rules_go + gazelle, then wire cache and BES.",
    ///     body_file = "./guidance/bazelify-go.md",
    ///     applies_to = ["go.mod"],
    /// )
    /// ```
    ///
    /// `body_file` is relative to the declaring `.axl` and must exist at load
    /// — a topic advertised in the index but unreadable when fetched is worse
    /// than one that was never advertised.
    fn guidance<'v>(
        #[starlark(require = named)] id: String,
        #[starlark(require = named)] title: String,
        #[starlark(require = named, default = String::new())] summary: String,
        #[starlark(require = named)] body_file: String,
        #[starlark(require = named, default = UnpackList::default())] applies_to: UnpackList<
            String,
        >,
        #[starlark(require = named, default = "stable".to_owned())] stability: String,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Guidance> {
        if id.is_empty() {
            return Err(anyhow::anyhow!("guidance id must not be empty"));
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(anyhow::anyhow!(
                "guidance id {id:?} must be lower-case ASCII, digits and '-' \
                 (it is what a caller types after --guidance=)"
            ));
        }

        let path = Env::current_script_path(eval)?;
        let dir = path.parent().ok_or_else(|| {
            anyhow::anyhow!("guidance {id:?}: cannot resolve the declaring script's directory")
        })?;
        let body_file = dir.join(&body_file);
        let body_file = body_file.canonicalize().map_err(|e| {
            anyhow::anyhow!(
                "guidance {id:?}: body_file {} is not readable: {e}",
                body_file.display()
            )
        })?;
        // Canonicalize both sides: comparing a resolved path against an
        // unresolved prefix rejects any repo reached through a symlink.
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        if !body_file.starts_with(&dir) {
            return Err(anyhow::anyhow!(
                "guidance {id:?}: body_file escapes the declaring module directory"
            ));
        }

        Ok(Guidance::new(
            id,
            title,
            summary,
            body_file,
            applies_to.items,
            GuidanceStability::parse(&stability)?,
            path,
        ))
    }
}
