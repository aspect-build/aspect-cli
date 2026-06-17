use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::starlark_value;

use starlark::{environment::GlobalsBuilder, values::starlark_value_as_type::StarlarkValueAsType};

use crate::engine::store::Env;
use crate::eval::join_confined;

pub mod auth;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<aspect.Aspect>")]
pub struct Aspect {}

starlark_simple_value!(Aspect);

#[starlark_value(type = "aspect.Aspect")]
impl<'v> values::StarlarkValue<'v> for Aspect {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(aspect_methods)
    }
}

#[starlark_module]
pub(crate) fn aspect_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn auth<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<auth::Auth> {
        Ok(auth::Auth {})
    }
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Aspect: StarlarkValueAsType<Aspect> = StarlarkValueAsType::new();
}

/// NEW `aspect.*` AXL GLOBAL. `aspect.read_builtin_text(path)` reads a file
/// sibling to the currently-evaluating `.axl` module and returns its contents
/// as text. `path` is relative (e.g. `"./wrapper.tmpl.sh"`), resolved against
/// the calling module's directory and confined to it. Lets aspect builtins
/// keep large embedded payloads (the `tools/bazel` wrapper template) as real
/// resource files in the builtin tree instead of inline AXL strings.
#[starlark_module]
fn register_read_builtin_text(globals: &mut GlobalsBuilder) {
    fn read_builtin_text<'v>(
        #[starlark(require = pos)] path: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<String> {
        let script = Env::current_script_path(eval)?;
        let dir = script.parent().ok_or_else(|| {
            anyhow::anyhow!("module path {} has no parent directory", script.display())
        })?;
        let resolved = join_confined(dir, std::path::Path::new(path))?;
        std::fs::read_to_string(&resolved)
            .map_err(|e| anyhow::anyhow!("{}: {}", resolved.display(), e))
    }
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);
    register_read_builtin_text(globals);
    auth::register_globals(globals);
}

#[cfg(test)]
mod tests {
    use crate::eval::{Loader, ModuleEnv};
    use crate::module::Mod;

    /// `aspect.read_builtin_text` resolves a sibling path against the calling
    /// module's directory and returns its contents verbatim. Doubles as the
    /// byte-identity guard for the `tools/bazel` wrapper template move: the
    /// fixture mirrors the real `wrapper.tmpl.sh` markers (the marker comment,
    /// all five flag-array placeholders, both version placeholders, and the
    /// `printf '%q'` array_push line).
    #[test]
    fn read_builtin_text_returns_sibling_resource() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        let resource = "#!/usr/bin/env bash\n\
            # aspect-build/tools-bazel-wrapper\n\
            ASPECT_BAZEL_WRAPPER_VERSION=\"{{ASPECT_CLI_VERSION}}\"\n\
            ASPECT_BAZEL_WRAPPER_BAZEL_VERSION=\"{{BAZEL_VERSION}}\"\n\
            BAZEL_VERBS=({{BAZEL_VERBS}})\n\
            BAZEL_VALUE_FLAGS=({{BAZEL_VALUE_FLAGS}})\n\
            BAZEL_BOOL_FLAGS=({{BAZEL_BOOL_FLAGS}})\n\
            BAZEL_SHORT_VALUE_FLAGS=({{BAZEL_SHORT_VALUE_FLAGS}})\n\
            BAZEL_SHORT_BOOL_FLAGS=({{BAZEL_SHORT_BOOL_FLAGS}})\n\
            array_push() { printf '%q' \"$1\"; }\n";
        std::fs::write(root.join("wrapper.tmpl.sh"), resource).unwrap();

        let script_path = root.join("module.axl");
        std::fs::write(
            &script_path,
            "TEMPLATE = aspect.read_builtin_text(\"./wrapper.tmpl.sh\")\n",
        )
        .unwrap();

        let modules: Vec<Mod> = vec![];
        let loader = Loader::new(
            "test".to_string(),
            root.clone(),
            root.clone(),
            None,
            &modules,
        );
        let scope = Mod::new(root.clone(), "_root".to_string(), root.clone());

        let got: Result<String, anyhow::Error> = ModuleEnv::with(|_env| {
            let frozen = loader.eval_module(&scope, &script_path)?;
            Ok(frozen
                .get("TEMPLATE")
                .map_err(|e| anyhow::anyhow!("{e:?}"))?
                .value()
                .unpack_str()
                .expect("TEMPLATE is a string")
                .to_string())
        });
        let got = got.expect("eval succeeds");

        assert_eq!(got, resource);
        for ph in [
            "{{BAZEL_VERBS}}",
            "{{BAZEL_VALUE_FLAGS}}",
            "{{BAZEL_BOOL_FLAGS}}",
            "{{BAZEL_SHORT_VALUE_FLAGS}}",
            "{{BAZEL_SHORT_BOOL_FLAGS}}",
            "{{ASPECT_CLI_VERSION}}",
            "{{BAZEL_VERSION}}",
        ] {
            assert_eq!(got.matches(ph).count(), 1, "placeholder {ph}");
        }
        assert!(got.contains("# aspect-build/tools-bazel-wrapper"));
        assert!(got.contains("printf '%q'"));
    }

    /// A `..` escape out of the module directory is rejected by `join_confined`.
    #[test]
    fn read_builtin_text_rejects_escape() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("secret.txt"), "nope").unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();

        let script_path = sub.join("module.axl");
        std::fs::write(
            &script_path,
            "X = aspect.read_builtin_text(\"../secret.txt\")\n",
        )
        .unwrap();

        let modules: Vec<Mod> = vec![];
        let loader = Loader::new(
            "test".to_string(),
            root.clone(),
            root.clone(),
            None,
            &modules,
        );
        let scope = Mod::new(root.clone(), "_root".to_string(), root.clone());

        let result: Result<(), crate::eval::EvalError> =
            ModuleEnv::with(|_env| loader.eval_module(&scope, &script_path).map(|_| ()));
        assert!(result.is_err(), "escape out of module dir must be rejected");
    }
}
