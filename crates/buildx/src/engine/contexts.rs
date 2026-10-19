//! The two C++ include contexts Bazel's `CppCompileAction` asks the action
//! context registry for, implemented in Rust.
//!
//! `StandaloneModule` registers `DummyCppIncludeExtractionContext`, which hands
//! out the build's `ArtifactFactory`, and `DummyCppIncludeScanningContext`,
//! which is never called when include scanning is off (local execution
//! discovers inclusions itself). Both are package-private and the first takes a
//! `CommandEnvironment`, so they are written again here.

use super::*;
use blazon_rt::Result as RtResult;

/// `CppIncludeExtractionContext`: the artifact resolver is the build's `ArtifactFactory`.
pub(super) struct Extraction {
    pub resolver: Global<bazel::ArtifactResolver<'static>>,
}

impl bazel::CppIncludeExtractionContextImpl for Extraction {
    fn artifact_resolver<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<bazel::ArtifactResolver<'a>> {
        // SAFETY of lifetimes: a new local reference to the global resolver.
        let resolver = self.resolver.get(env)?;
        Ok(resolver.upcast::<bazel::ArtifactResolver>())
    }
}

/// `CppIncludeScanningContext`: include scanning is off, so a scan adds no inputs.
pub(super) struct Scanning;

impl bazel::CppIncludeScanningContextImpl for Scanning {
    fn find_additional_inputs<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        _action: bazel::CppCompileAction<'a>,
        _context: bazel::ActionExecutionContext<'a>,
        _headers: bazel::IncludeScannerIncludeScanningHeaderData<'a>,
    ) -> RtResult<Option<bazel::List<'a, bazel::Artifact<'static>>>> {
        Ok(Some(bazel::List::new(env)?))
    }
}
