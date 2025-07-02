fn main() {
    let target = std::env::var("TARGET").unwrap();
    println!("cargo:rustc-env=BUILD_TARGET_TRIPLE={target}");

    // We use the @platforms names for things
    // Which are bad and terrible but we're sorta stuck with them
    let (bzlos, bzlarch) = match target.as_str() {
        "aarch64-apple-darwin" => ("macos", "aarch64"),
        "x86_64-apple-darwin" => ("macos", "x86_64"),
        "x86_64-unknown-linux-gnu" => ("linux", "x86_64"),
        "aarch64-unknown-linux-gnu" => ("linux", "aarch64"),
        "x86_64-unknown-linux-musl" => ("linux", "x86_64"),
        "aarch64-unknown-linux-musl" => ("linux", "aarch64"),
        "x86_64-pc-windows-msvc" => ("windows", "x86_64"),
        "x86_64-pc-windows-gnu" => ("windows", "x86_64"),
        "aarch64-pc-windows-msvc" => ("windows", "x86_64"),
        "aarch64-pc-windows-gnu" => ("windows", "x86_64"),
        "i686-pc-windows-msvc" => ("windows", "x86_32"),
        "i686-pc-windows-gnu" => ("windows", "x86_32"),
        // Add more mappings as needed for other target triples
        _ => {
            panic!("Warning: Unknown target triple: {target}");
        }
    };

    println!("cargo:rustc-env=BUILD_BZLOS={bzlos}");
    println!("cargo:rustc-env=BUILD_BZLARCH={bzlarch}");

    // Which are not the golang names for things, which we also need, for Gazelle
    // See https://gist.github.com/asukakenji/f15ba7e588ac42795f421b48b8aede63
    let (goos, goarch) = match target.as_str() {
        "aarch64-apple-darwin" => ("darwin", "arm64"),
        "x86_64-apple-darwin" => ("darwin", "amd64"),
        "x86_64-unknown-linux-gnu" => ("linux", "amd64"),
        "aarch64-unknown-linux-gnu" => ("linux", "arm64"),
        "x86_64-unknown-linux-musl" => ("linux", "amd64"),
        "aarch64-unknown-linux-musl" => ("linux", "arm64"),
        "x86_64-pc-windows-msvc" => ("windows", "amd64"),
        "x86_64-pc-windows-gnu" => ("windows", "amd64"),
        "aarch64-pc-windows-msvc" => ("windows", "amd64"),
        "aarch64-pc-windows-gnu" => ("windows", "amd64"),
        "i686-pc-windows-msvc" => ("windows", "386"),
        "i686-pc-windows-gnu" => ("windows", "386"),
        // Add more mappings as needed for other target triples
        _ => {
            panic!("Warning: Unknown target triple: {target}");
        }
    };

    println!("cargo:rustc-env=BUILD_GOOS={goos}");
    println!("cargo:rustc-env=BUILD_GOARCH={goarch}");
    println!("cargo:rustc-env=LLVM_TRIPLE={target}");
}
