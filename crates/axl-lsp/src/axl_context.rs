use axl_runtime::eval;
use starlark::docs::DocModule;
use starlark::errors::EvalMessage;
use starlark::syntax::AstModule;
use starlark::syntax::Dialect;
use starlark::syntax::DialectTypes;
use starlark_lsp::error::eval_message_to_lsp_diagnostic;
use starlark_lsp::server::LspContext;
use starlark_lsp::server::LspEvalResult;
use starlark_lsp::server::LspUrl;
use starlark_lsp::server::StringLiteralResult;
pub struct AxlContext {}

impl LspContext for AxlContext {
    fn parse_file_with_contents(
        &self,
        uri: &LspUrl,
        content: String,
    ) -> starlark_lsp::server::LspEvalResult {
        eprintln!("parse_file_with_contents {uri} {content}");
        let dialect = Dialect {
            enable_def: true,
            enable_f_strings: true,
            enable_keyword_only_arguments: true,
            enable_lambda: true,
            enable_load_reexport: false,
            enable_load: false,
            enable_top_level_stmt: true,
            enable_types: DialectTypes::Enable,
            enable_positional_only_arguments: true,
            ..Default::default()
        };
        match uri {
            LspUrl::File(path) => {
                match AstModule::parse(&path.to_string_lossy(), content, &dialect) {
                    Ok(ast) => LspEvalResult {
                        ast: Some(ast),
                        ..Default::default()
                    },
                    Err(err) => {
                        let err = EvalMessage::from_error(path, &err);

                        LspEvalResult {
                            diagnostics: vec![eval_message_to_lsp_diagnostic(err)],
                            ast: None,
                        }
                    }
                }
            }
            _ => LspEvalResult::default(),
        }
    }

    fn resolve_load(
        &self,
        path: &str,
        current_file: &LspUrl,
        workspace_root: Option<&std::path::Path>,
    ) -> anyhow::Result<LspUrl> {
        eprintln!("resolve_load {path} {current_file} {workspace_root:?}");
        anyhow::bail!("not implemented yet: resolve_load")
    }

    fn render_as_load(
        &self,
        target: &LspUrl,
        current_file: &LspUrl,
        workspace_root: Option<&std::path::Path>,
    ) -> anyhow::Result<String> {
        eprintln!("render_as_load {target} {current_file} {workspace_root:?}");
        Ok(String::new())
    }

    fn resolve_string_literal(
        &self,
        literal: &str,
        current_file: &LspUrl,
        workspace_root: Option<&std::path::Path>,
    ) -> anyhow::Result<Option<StringLiteralResult>> {
        eprintln!("resolve_string_literal {literal} {current_file} {workspace_root:?}");
        Ok(None)
    }

    fn get_load_contents(&self, uri: &LspUrl) -> anyhow::Result<Option<String>> {
        eprintln!("get_load_contents {uri}");
        Ok(None)
    }

    fn get_environment(&self, uri: &LspUrl) -> DocModule {
        eprintln!("get_environment {uri}");
        eval::get_globals().build().documentation()
    }

    fn get_url_for_global_symbol(
        &self,
        current_file: &LspUrl,
        symbol: &str,
    ) -> anyhow::Result<Option<LspUrl>> {
        eprintln!("get_url_for_global_symbol {current_file} {symbol}");
        Ok(None)
    }
}
