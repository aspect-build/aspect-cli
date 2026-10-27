//! What bazel writes, decoded: one module per output, with the framing they
//! share. Each reads from a [`crate::stream`] and reports how it ended.

pub mod bes_binary_file;
pub mod delimited;
