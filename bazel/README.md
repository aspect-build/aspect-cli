# bazel

This is a partial clone of proto files from the `github.com/bazelbuild/bazel` repository.
The vendored files here avoid a full clone of the original repository, which is big.

Directories are flattened.
We don't want to make CLI plugin developers type out very long paths including
segments like src/main/protobuf and src/main/java/com/google/devtools/build/lib.
