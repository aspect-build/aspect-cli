//! Docsite links the runtime prints, as `aspect.build/go/<slug>` permalinks.
//!
//! A shipped CLI keeps printing a link for good, so printed text never names a
//! docs page directly. Each slug is a redirect in the docsite's `docs.json`
//! that follows the page (and `#heading`) when it moves; the site's CI checks
//! that each one lands on a public page and heading. A new slug lands on the
//! site before the CLI change that prints it. The builtins' AXL equivalent is
//! `private/lib/links.axl`.

/// The CLI docs landing page.
pub const CLI: &str = "https://aspect.build/go/cli";

/// The Build Results API guide: where it is available and how to enable it.
pub const BUILD_RESULTS_API: &str = "https://aspect.build/go/build-results-api";

/// The Build Results API guide's action performance history section.
pub const BUILD_RESULTS_API_ACTION_HISTORY: &str =
    "https://aspect.build/go/build-results-api-action-history";
