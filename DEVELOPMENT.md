# Development

## With direnv

We use https://direnv.net with the `.envrc` file to setup tooling on the $PATH.
This includes the most recent release of the `aspect` command.

## Crates

- `aspect-cli` (which is managed by the launcher) serves as an entrypoint to all other components
- `aspect-launcher` (what is actually installed as `aspect` on the PATH) serves the functions of `bazelisk`
- `axl-runtime` AXL engine for extending the CLI

## Syntax highlight .axl files as Starlark

### Visual Studio Code

Under Settings -> Files -> Associations add a `.axl` => `starlark` association.

## Build and run the CLI locally

```
bazel build //:cli
bazel-bin/crates/aspect-cli/aspect-cli <command>
```

## Update docs

```
bazel build //tools:bazel_env
(cd crates/axl-docgen && cargo run)
```

## Managing Dependencies

When adding new Rust dependencies via Cargo, you must run repin to make them available to Bazel:

```bash
# First, add dependency to your Cargo.toml
cargo add my_dependency

# Then repin dependencies for Bazel
CARGO_BAZEL_ISOLATED=1 CARGO_BAZEL_REPIN=1 bazel build //:cli //:launcher
```

If you are adding a crate which is used in multiple `Cargo.toml` files strongly consider making the create a workspace dependency. 

```
cargo add --workspace-root YOUR_CRATE
```

## Releasing

Releases are kicked off when a release tag is pushed.

To determine the release tag for a particular commit run,

```
./bazel/workspace_status.sh | grep STABLE_MONOREPO_SHORT_VERSION
```

To cut a release, push resulting tag. For example,

```
git tag 2025.42.5
git push origin 2025.42.5
```
