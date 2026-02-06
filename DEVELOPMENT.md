# Development

## With direnv

We use https://direnv.net with the `.envrc` file to setup tooling on the $PATH.
This includes the most recent release of the `aspect` command.

## Crates

- `aspect-cli` the main Aspect CLI binary that serves as an entrypoint for all tasks (which is managed by the launcher)
- `aspect-launcher` fetches and hands off control the version of the Aspect CLI binary configured in a repository (what is actually installed as `aspect` on the PATH)
- `axl-runtime` AXL engine for extending the CLI

## Syntax highlight .axl files as Starlark

### Visual Studio Code

Under Settings -> Files -> Associations add `.axl` and `.aspect` to `starlark` associations.

## Build and run the CLI locally

```
bazel build //:cli
bazel-bin/crates/aspect-cli/aspect-cli <command>
```

## Update docs

```
(cd crates/axl-docgen && cargo run)
```

## Managing Dependencies

Just add dependency to your Cargo.toml.

```
cargo add my_dependency
```

If you are adding a crate which is used in multiple `Cargo.toml` files strongly consider making the create a workspace dependency. 

```
cargo add --workspace-root YOUR_CRATE
```

## Releasing

The simplest is to click the green button on
https://github.com/aspect-build/aspect-cli/actions/workflows/tag.yaml

To manually override the release tag for a particular commit run,
choose a version and do something like:

```
git tag v2025.42.5
git push origin v2025.42.5
```
