mod axl_context;
mod bazel;

fn main() -> anyhow::Result<()> {
    let ctx = axl_context::AxlContext {};
    starlark_lsp::server::stdio_server(ctx)?;
    Ok(())
}
