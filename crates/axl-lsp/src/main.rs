mod axl_context;

fn main() -> anyhow::Result<()> {
    println!("AXL LSP");

    let ctx = axl_context::AxlContext {};
    starlark_lsp::server::stdio_server(ctx)?;
    Ok(())
}
