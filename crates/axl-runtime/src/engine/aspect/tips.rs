//! `ctx.aspect.tips`: the tips silenced in the `config.json` files, and the
//! writer that silences one for this user (see `crate::user_config`).

use std::path::{Path, PathBuf};

use allocative::Allocative;
use derive_more::Display;
use either::Either;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::AllocList;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::structs::AllocStruct;
use starlark::values::{self, NoSerialize, ProvidesStaticType, Value, starlark_value};

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<aspect.Tips>")]
pub struct Tips {}

starlark_simple_value!(Tips);

#[starlark_value(type = "aspect.Tips")]
impl<'v> values::StarlarkValue<'v> for Tips {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("tips_methods", tips_methods);
        Some(RES.methods())
    }
}

/// `path` when given (`""` for none), else `default`.
fn chosen(path: NoneOr<String>, default: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    match path.into_option() {
        Some(path) if path.is_empty() => None,
        Some(path) => Some(PathBuf::from(path)),
        None => default(),
    }
}

/// `warnings` less those already shown in this process
/// (`crate::config_json::first_time`), so each shows once however many
/// readers meet it.
fn unseen(warnings: Vec<String>) -> Vec<String> {
    warnings
        .into_iter()
        .filter(|w| crate::config_json::first_time(w))
        .collect()
}

fn set<'v>(
    ids: Either<String, UnpackListOrTuple<String>>,
    path: NoneOr<String>,
    silence: bool,
    heap: values::Heap<'v>,
) -> Value<'v> {
    let ids = match ids {
        Either::Left(id) => vec![id],
        Either::Right(ids) => ids.items,
    };
    let result = crate::user_config::path_or_user(path.into_option().as_deref())
        .and_then(|path| crate::user_config::set_tips_silenced(Path::new(&path), &ids, silence));
    match result {
        Ok(_) => Value::new_none(),
        Err(e) => heap.alloc(e.to_string()),
    }
}

#[starlark_module]
fn tips_methods(registry: &mut MethodsBuilder) {
    /// Whether `id` can be a tip's id: not empty, and no whitespace or comma
    /// (`--tips:silence=a,b` is one id, so a comma is always a mistake).
    fn is_tip_id<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] id: &str,
    ) -> anyhow::Result<bool> {
        Ok(crate::user_config::valid_id(id))
    }

    /// The tips the `config.json` files silence, as a struct: `user`, the
    /// ids in `tips.silence` of the user's `~/.aspect/config.json`; `repo`,
    /// those of the repository's `.aspect/config.json`; and `warnings`, what
    /// is wrong with an invalid one, whose ids are left out, each warning
    /// returned once per process. `user` and `repo` name other files (`""`
    /// for none).
    fn config_silence<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] user: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] repo: NoneOr<String>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let user = chosen(user, || crate::config_json::user_path().ok());
        let repo = chosen(repo, crate::config_json::repo_path);
        let found = crate::user_config::config_silence(user.as_deref(), repo.as_deref());
        Ok(heap.alloc(AllocStruct([
            ("user", heap.alloc(AllocList(found.user))),
            ("repo", heap.alloc(AllocList(found.repo))),
            ("warnings", heap.alloc(AllocList(unseen(found.warnings)))),
        ])))
    }

    /// Add `id` (one id, or a list) to `tips.silence` of the user's
    /// `~/.aspect/config.json` (or the file at `path`) in one write, creating
    /// it when missing, under a lock and atomically, every other key kept.
    /// `None` when done, already silenced included; else why not, as a
    /// string, and nothing written (one bad id writes none).
    fn silence<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] id: Either<String, UnpackListOrTuple<String>>,
        #[starlark(require = named, default = NoneOr::None)] path: NoneOr<String>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        Ok(set(id, path, true, heap))
    }

    /// Remove `id` (one id, or a list) from `tips.silence` of the user's
    /// `~/.aspect/config.json` (or the file at `path`), as `silence` writes
    /// it. `None` when done, already absent included; else why not, as a
    /// string, and nothing written.
    fn unsilence<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] id: Either<String, UnpackListOrTuple<String>>,
        #[starlark(require = named, default = NoneOr::None)] path: NoneOr<String>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        Ok(set(id, path, false, heap))
    }
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Tips: StarlarkValueAsType<Tips> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);
}
