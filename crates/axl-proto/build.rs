use std::{env, fs, path::PathBuf};

use prost::Message;
use prost_build::Config;
use prost_types::{DescriptorProto, EnumDescriptorProto, FileDescriptorSet};
use tonic_prost_build::configure;

fn main() -> Result<(), std::io::Error> {
    let descriptor_path =
        PathBuf::from(std::env::var("DESCRIPTOR").unwrap_or("descriptor.bin".into()));

    let fds = FileDescriptorSet::decode(std::fs::read(descriptor_path).unwrap().as_slice())?;

    let mut config = Config::new();

    let return_expr = "::starlark::values::none::NoneOr::from_option(this.id.as_ref().map(|id| id.id.clone().unwrap()))";
    let return_type = "::starlark::values::none::NoneOr<build_event_id::Id>";
    config.field_attribute(
        "build_event_stream.BuildEvent.id",
        format!(
            r#"#[starbuf(return_expr="{}", return_type="{}")]"#,
            return_expr, return_type,
        ),
    );

    let return_expr = "this.id.as_ref().map(|id| id.id.clone().unwrap()).unwrap().as_str_name()";
    let return_type = "&'static str";

    config.field_attribute(
        "build_event_stream.BuildEvent.last_message",
        format!(
            r#"#[starbuf(rename="kind", return_expr="{}", return_type="{}")]"#,
            return_expr, return_type,
        ),
    );

    config.enable_type_names();

    fn traverse(
        prefix: &str,
        config: &mut Config,
        enums: &Vec<EnumDescriptorProto>,
        desc: &Vec<DescriptorProto>,
    ) {
        for en in enums {
            config.type_attribute(
                format!("{}.{}", prefix, en.name()),
                r#"#[derive(
                    ::starbuf_derive::Enumeration,
                    ::allocative::Allocative,
                    ::starlark::values::NoSerialize,
                    ::starlark::values::ProvidesStaticType,
                )]"#,
            );
        }
        for desc in desc {
            for field in &desc.field {
                let path = format!("{}.{}.{}", prefix, desc.name(), field.name());
                if field.type_name() == ".google.protobuf.Duration" {
                    config.field_attribute(
                        &path,
                        format!(r#"#[starbuf(path = "{}", duration)]"#, path),
                    );
                    config.field_attribute(&path, r#"#[allocative(skip)]"#);
                } else if field.type_name() == ".google.protobuf.Timestamp" {
                    config.field_attribute(
                        &path,
                        format!(r#"#[starbuf(path = "{}", timestamp)]"#, path),
                    );
                    config.field_attribute(&path, r#"#[allocative(skip)]"#);
                } else if field.type_name() == ".google.protobuf.Any" {
                    config.field_attribute(&path, format!(r#"#[starbuf(path = "{}", any)]"#, path));
                    config.field_attribute(&path, r#"#[allocative(skip)]"#);
                    // If this Any field is part of a oneof, also add allocative(skip)
                    // to the variant path so the Oneof derive can skip it.
                    if let Some(oneof_idx) = field.oneof_index {
                        if let Some(oneof) = desc.oneof_decl.get(oneof_idx as usize) {
                            let variant_path = format!(
                                "{}.{}.{}.{}",
                                prefix,
                                desc.name(),
                                oneof.name(),
                                field.name()
                            );
                            config.field_attribute(&variant_path, r#"#[allocative(skip)]"#);
                        }
                    }
                } else {
                    config.field_attribute(&path, format!(r#"#[starbuf(path = "{}")]"#, path));
                }
            }
            config.type_attribute(
                format!("{}.{}", prefix, desc.name()),
                r#"
#[derive(
    ::starlark::values::ProvidesStaticType,
    ::starlark::values::Trace,
    ::starlark::values::NoSerialize,
    ::allocative::Allocative,
    ::starbuf_derive::Message
)]
         "#,
            );

            for oneof in &desc.oneof_decl {
                let path = format!("{}.{}.{}", prefix, desc.name(), oneof.name());
                config.field_attribute(&path, format!(r#"#[starbuf(path = "{}")]"#, path));
                config.type_attribute(
                    &path,
                    "#[derive(::starbuf_derive::Oneof, ::allocative::Allocative)]",
                );
            }

            traverse(
                format!("{}.{}", prefix, desc.name()).as_str(),
                config,
                &desc.enum_type,
                &desc.nested_type,
            );
        }
    }

    for file in &fds.file {
        // `google.devtools.build.v1` is intentionally left out — its
        // types aren't reached from any of the .axl surfaces we expose,
        // and including them would pull in additional `Any` / `Empty`
        // handling without payoff. `google.longrunning` IS traversed now
        // so `OperationInfo`, `CancelOperationRequest`, etc. get the
        // standard `Message` derive — the generic Any-field branch in
        // `traverse` already covers `Operation.metadata` and the Any
        // inside `operation::Result.response`.
        if file.package() == "google.devtools.build.v1" {
            continue;
        }
        traverse(
            file.package(),
            &mut config,
            &file.enum_type,
            &file.message_type,
        );
    }

    configure()
        .build_client(true)
        .build_server(true)
        .compile_fds_with_config(fds, config)?;

    let out_dir = env::var("OUT_DIR").unwrap();

    let build_event_stream = fs::read_to_string(format!("{out_dir}/build_event_stream.rs"))?;

    fs::write(
        format!("{out_dir}/build_event_stream.rs"),
        format!(
            r#"/// @Generated by build.rs

#[starbuf_derive::types]
pub mod build_event_stream {{

{build_event_stream}

}}
"#
        ),
    )?;

    let query = fs::read_to_string(format!("{out_dir}/blaze_query.rs"))?;

    fs::write(
        format!("{out_dir}/blaze_query.rs"),
        format!(
            r#"
/// @Generated by build.rs
#[starbuf_derive::types]
pub mod blaze_query {{

{query}

}}
    "#
        ),
    )?;

    let tools = fs::read_to_string(format!("{out_dir}/tools.protos.rs"))?;

    fs::write(
        format!("{out_dir}/tools.protos.rs"),
        format!(
            r#"
/// @Generated by build.rs
#[starbuf_derive::types]
pub mod tools {{

pub mod protos {{

{tools}

}}

}}
    "#,
            tools = tools,
        ),
    )?;

    let v2 = fs::read_to_string(format!("{out_dir}/build.bazel.remote.execution.v2.rs"))?;

    fs::write(
        format!("{out_dir}/build.bazel.remote.execution.v2.rs"),
        format!(
            r#"
/// @Generated by build.rs
#[starbuf_derive::types]
pub mod v2 {{

{v2}

}}
    "#
        ),
    )?;

    let workspace_log = fs::read_to_string(format!("{out_dir}/workspace_log.rs"))?;

    fs::write(
        format!("{out_dir}/workspace_log.rs"),
        format!(
            r#"
/// @Generated by build.rs
#[starbuf_derive::types]
pub mod workspace_log {{

{workspace_log}

}}
    "#
        ),
    )?;

    let remote_logging = fs::read_to_string(format!("{out_dir}/remote_logging.rs"))?;

    fs::write(
        format!("{out_dir}/remote_logging.rs"),
        format!(
            r#"
/// @Generated by build.rs
#[starbuf_derive::types]
pub mod remote_logging {{

{remote_logging}

}}
    "#
        ),
    )?;

    // Wrap each additional proto package the same way `v2` is wrapped so
    // `#[starbuf_derive::types]` sees the literal items (not an
    // unexpanded `include!`). Each emits a `<pkg>_toplevels` function as
    // a sibling of the wrapper mod, which `axl-runtime/engine/mod.rs`
    // registers under `_proto.<pkg>.*`.
    //
    // `google.longrunning` is intentionally not wrapped — its types
    // (other than `Operation`, special-cased above) don't get the
    // `Message` derive applied, so `#[starbuf_derive::types]` would fail
    // type-checking the `AllocValue` bound. If those types become
    // useful from `.axl`, the fix is to widen the derive coverage in
    // the main traversal above (and handle the `prost_types::Any`
    // inside `operation::Result` carefully).
    for (file, mod_name) in [
        ("google.bytestream.rs", "bytestream"),
        ("google.rpc.rs", "rpc"),
        ("google.longrunning.rs", "longrunning"),
        ("build.bazel.semver.rs", "semver"),
    ] {
        let content = fs::read_to_string(format!("{out_dir}/{file}"))?;
        fs::write(
            format!("{out_dir}/{file}"),
            format!(
                r#"
/// @Generated by build.rs
#[starbuf_derive::types]
pub mod {mod_name} {{

{content}

}}
"#
            ),
        )?;
    }

    Ok(())
}
