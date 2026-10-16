use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [spec] = args.as_slice() else {
        eprintln!("usage: buildx path/to/file.star:main");
        exit(2);
    };
    match buildx::starlark::run(spec) {
        Ok(code) => exit(code),
        Err(e) => {
            eprintln!("buildx: {e:#}");
            exit(1);
        }
    }
}
