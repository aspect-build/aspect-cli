#![allow(clippy::all)]
#![allow(deprecated)]
// starbuf-derive skips deprecated fiels by default
// however, deprecated fields still need to be used
// to initialize structs, and that causes rust to issue
// warnings;
// See: https://github.com/rust-lang/rust/issues/47219

pub use prost_types::Any;
pub use prost_types::Duration;
pub use prost_types::Timestamp;

include!(concat!(env!("OUT_DIR"), "/build_event_stream.rs"));
include!(concat!(env!("OUT_DIR"), "/blaze_query.rs"));
include!(concat!(env!("OUT_DIR"), "/tools.protos.rs"));

#[path = "./pb_impl.rs"]
mod pb_impl;

pub mod google {
    pub mod devtools {
        pub mod build {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/google.devtools.build.v1.rs"));
            }
        }
    }
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
