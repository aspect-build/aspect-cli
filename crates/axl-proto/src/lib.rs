#![allow(clippy::all)]
#![allow(deprecated)]
// starbuf-derive skips deprecated fiels by default
// however, deprecated fields still need to be used
// to initialize structs, and that causes rust to issue
// warnings;
// See: https://github.com/rust-lang/rust/issues/47219

/// Every message's descriptor, a `FileDescriptorSet`: for reading a message
/// from its proto3 JSON form, as Bazel's `--build_event_json_file` writes a
/// build event.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/descriptor.bin"));

pub use prost_types::Any;
pub use prost_types::Duration;
pub use prost_types::Timestamp;

include!(concat!(env!("OUT_DIR"), "/workspace_log.rs"));
include!(concat!(env!("OUT_DIR"), "/build_event_stream.rs"));
include!(concat!(env!("OUT_DIR"), "/blaze_query.rs"));
include!(concat!(env!("OUT_DIR"), "/tools.protos.rs"));
pub mod build {
    pub mod bazel {
        // `semver` is wrapped by `axl-proto/build.rs` — see the include!
        // pattern around it in lib.rs.
        include!(concat!(env!("OUT_DIR"), "/build.bazel.semver.rs"));
        pub mod remote {
            pub mod execution {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/build.bazel.remote.execution.v2.rs"
                ));

                #[starbuf_derive::service(
                    client = "crate::build::bazel::remote::execution::v2::action_cache_client::ActionCacheClient",
                    methods(
                        name = "GetActionResult",
                        method = "get_action_result",
                        request = "crate::build::bazel::remote::execution::v2::GetActionResultRequest",
                        response = "crate::build::bazel::remote::execution::v2::ActionResult",
                    ),
                    methods(
                        name = "UpdateActionResult",
                        method = "update_action_result",
                        request = "crate::build::bazel::remote::execution::v2::UpdateActionResultRequest",
                        response = "crate::build::bazel::remote::execution::v2::ActionResult",
                    )
                )]
                pub struct ActionCache;

                #[starbuf_derive::service(
                    client = "crate::build::bazel::remote::execution::v2::capabilities_client::CapabilitiesClient",
                    methods(
                        name = "GetCapabilities",
                        method = "get_capabilities",
                        request = "crate::build::bazel::remote::execution::v2::GetCapabilitiesRequest",
                        response = "crate::build::bazel::remote::execution::v2::ServerCapabilities",
                    )
                )]
                pub struct Capabilities;

                #[starbuf_derive::service(
                    client = "crate::build::bazel::remote::execution::v2::execution_client::ExecutionClient",
                    methods(
                        name = "Execute",
                        method = "execute",
                        request = "crate::build::bazel::remote::execution::v2::ExecuteRequest",
                        response = "crate::google::longrunning::Operation",
                        streaming = true,
                    ),
                    methods(
                        name = "WaitExecution",
                        method = "wait_execution",
                        request = "crate::build::bazel::remote::execution::v2::WaitExecutionRequest",
                        response = "crate::google::longrunning::Operation",
                        streaming = true,
                    )
                )]
                pub struct Execution;

                #[starbuf_derive::service(
                    client = "crate::build::bazel::remote::execution::v2::content_addressable_storage_client::ContentAddressableStorageClient",
                    methods(
                        name = "FindMissingBlobs",
                        method = "find_missing_blobs",
                        request = "crate::build::bazel::remote::execution::v2::FindMissingBlobsRequest",
                        response = "crate::build::bazel::remote::execution::v2::FindMissingBlobsResponse",
                    ),
                    methods(
                        name = "BatchUpdateBlobs",
                        method = "batch_update_blobs",
                        request = "crate::build::bazel::remote::execution::v2::BatchUpdateBlobsRequest",
                        response = "crate::build::bazel::remote::execution::v2::BatchUpdateBlobsResponse",
                    ),
                    methods(
                        name = "BatchReadBlobs",
                        method = "batch_read_blobs",
                        request = "crate::build::bazel::remote::execution::v2::BatchReadBlobsRequest",
                        response = "crate::build::bazel::remote::execution::v2::BatchReadBlobsResponse",
                    ),
                    methods(
                        name = "GetTree",
                        method = "get_tree",
                        request = "crate::build::bazel::remote::execution::v2::GetTreeRequest",
                        response = "crate::build::bazel::remote::execution::v2::GetTreeResponse",
                        streaming = true,
                    )
                )]
                pub struct ContentAddressableStorage;
            }
        }
    }
}

#[path = "./pb_impl.rs"]
mod pb_impl;

pub mod google {
    // bytestream / rpc / longrunning are wrapped in their own
    // `#[starbuf_derive::types] pub mod <name>` block by
    // `axl-proto/build.rs`, so the `include!`'d content brings in both
    // the inner mod and its sibling `<name>_toplevels` function.
    include!(concat!(env!("OUT_DIR"), "/google.bytestream.rs"));

    #[starbuf_derive::service(
        client = "crate::google::bytestream::byte_stream_client::ByteStreamClient",
        methods(
            name = "Read",
            method = "read",
            request = "crate::google::bytestream::ReadRequest",
            response = "crate::google::bytestream::ReadResponse",
            streaming = true,
        ),
        methods(
            name = "Write",
            method = "write",
            request = "crate::google::bytestream::WriteRequest",
            response = "crate::google::bytestream::WriteResponse",
            request_streaming = true,
        ),
        methods(
            name = "QueryWriteStatus",
            method = "query_write_status",
            request = "crate::google::bytestream::QueryWriteStatusRequest",
            response = "crate::google::bytestream::QueryWriteStatusResponse",
        )
    )]
    pub struct ByteStream;

    pub mod devtools {
        pub mod build {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/google.devtools.build.v1.rs"));
            }
        }
    }
    include!(concat!(env!("OUT_DIR"), "/google.rpc.rs"));
    include!(concat!(env!("OUT_DIR"), "/google.longrunning.rs"));
}

pub mod analysis {
    include!(concat!(env!("OUT_DIR"), "/analysis.rs"));
}
pub mod bazel_flags {
    include!(concat!(env!("OUT_DIR"), "/bazel_flags.rs"));
}
pub mod blaze {
    include!(concat!(env!("OUT_DIR"), "/blaze.rs"));
    pub mod invocation_policy {
        include!(concat!(env!("OUT_DIR"), "/blaze.invocation_policy.rs"));
    }
    pub mod strategy_policy {
        include!(concat!(env!("OUT_DIR"), "/blaze.strategy_policy.rs"));
    }
}

pub mod command_line {
    include!(concat!(env!("OUT_DIR"), "/command_line.rs"));
}
pub mod devtools {
    pub mod build {
        pub mod lib {
            pub mod packages {
                pub mod metrics {
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/devtools.build.lib.packages.metrics.rs"
                    ));
                }
            }
        }
    }
}
pub mod failure_details {
    include!(concat!(env!("OUT_DIR"), "/failure_details.rs"));
}
pub mod options {
    include!(concat!(env!("OUT_DIR"), "/options.rs"));
}
pub mod stardoc_output {
    include!(concat!(env!("OUT_DIR"), "/stardoc_output.rs"));
}
include!(concat!(env!("OUT_DIR"), "/remote_logging.rs"));

#[cfg(test)]
mod tests {
    use starlark::values::Heap;

    use crate::blaze::action_cache_statistics::{MissDetail, MissReason};

    fn reason_of(reason: i32) -> String {
        Heap::temp(|heap| {
            let detail = heap.alloc(MissDetail { reason, count: 1 });
            detail
                .get_attr("reason", heap)
                .expect("reading reason")
                .expect("a reason field")
                .unpack_str()
                .expect("a string")
                .to_owned()
        })
    }

    #[test]
    fn a_known_enum_value_reads_as_its_name() {
        assert_eq!(
            reason_of(MissReason::NotCached as i32).to_uppercase(),
            "NOT_CACHED"
        );
    }

    /// Bazel 9's `DIGEST_MISMATCH` (7) reads by its name, the build event
    /// protos being Bazel 9.3.3's.
    #[test]
    fn bazel_9s_newest_miss_reason_reads_as_its_name() {
        assert_eq!(reason_of(7).to_uppercase(), "DIGEST_MISMATCH");
    }

    /// A `Duration` field reads as `<field>_ms`, its whole milliseconds, and
    /// None when unset; the field itself is no attribute.
    #[test]
    fn a_duration_reads_as_its_whole_milliseconds() {
        use crate::build_event_stream::TestResult;
        let ms = |duration: Option<prost_types::Duration>| {
            Heap::temp(|heap| {
                let result = heap.alloc(TestResult {
                    test_attempt_duration: duration,
                    ..Default::default()
                });
                assert!(
                    result
                        .get_attr("test_attempt_duration", heap)
                        .unwrap()
                        .is_none(),
                    "the Duration itself is not read"
                );
                let got = result
                    .get_attr("test_attempt_duration_ms", heap)
                    .expect("reading it")
                    .expect("a `_ms` attribute");
                got.unpack_i32().map(i64::from)
            })
        };
        let at = |seconds, nanos| Some(prost_types::Duration { seconds, nanos });
        assert_eq!(ms(at(1, 234_567_890)), Some(1234));
        assert_eq!(ms(at(0, 999_999)), Some(0));
        assert_eq!(ms(at(-1, -500_000_000)), Some(-1500));
        assert_eq!(ms(None), None);
    }

    #[test]
    fn an_enum_value_the_proto_does_not_know_reads_as_its_number() {
        assert_eq!(reason_of(99), "99");
    }
}
