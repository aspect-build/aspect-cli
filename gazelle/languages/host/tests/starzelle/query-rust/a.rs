use std::io::{self, Write};

fn main() {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "Hello, world!").expect("Failed to write to stdout");
}
