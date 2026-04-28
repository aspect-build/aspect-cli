use include_dir::{Dir, include_dir};

pub(crate) static STD_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/std");

pub fn get(filename: &str) -> Option<&'static str> {
    STD_DIR.get_file(filename)?.contents_utf8()
}
