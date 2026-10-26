# Deliveries

Status: draft. Items under [Open questions](#open-questions) are undecided.

## Summary

Deliveries is a separate service in the Strata web app. It shows what was delivered, and what actually changed inside the delivered package between two revisions. The headline feature is **artifact-aware diffing**: compare the contents of two versions of a package, not only the builds that produced them.

A delivery target is one Bazel target (a deliverable) and everything delivered from it over time. Each delivery is a **revision** of that target. The page opens on the newest revision compared with the one before it.

## Motivation

- Compare answers "how did build A differ from build B" with metrics, flags and which targets re-ran. It does not look inside outputs. It cannot say that a `.deb` gained a file, that an image's dependency layer changed, or that a config file inside an archive differs.
- Teams that ship packaged artifacts (system packages, archives, container images) want to see what is in a release and what changed since the last one, with who shipped it, from which CI run, PR, branch and commit.
- Customers who launch external systems from a delivery target want the target's own output kept and shown with the delivery, not discarded.

## Goals

- Show a delivery target's revisions and what changed inside the package between any two of them.
- Default to diffing against the previous revision. Allow any other base.
- Support viewing a single revision without a diff.
- Show how the deliverable target ran: command, arguments, environment, duration, and whether it executed locally or remotely, plus its console output and structured output.
- Show provenance for every revision: who, which CI run, PR, release branch, tag, commit, with links.
- Link to the existing Compare view for build-level questions.

## Non-goals

- Rebuilding or duplicating Compare. Build-level metrics, flags and target diffs stay in Compare.
- A new AXL API. At most a single `ctx.aspect.delivery()` entry point, and only if the data cannot reach the service another way.
- A release-management board, RC workflow tooling, approvals, or orchestration of downstream validation.
- A GitOps-style reconciler that applies desired state to destinations.
- Streaming of live output in the first version.

## Concepts

| Term | Meaning |
| --- | --- |
| Delivery target | A deliverable Bazel target, identified by repository and label, for example `//aqnav:deb`. |
| Revision | One delivery of a delivery target: the artifact, the run that delivered it, and its provenance. |
| Compared / base | The two revisions being diffed. Compared defaults to the newest, base to the revision before it. |
| Package | The delivered artifact: a `.deb`, a `.tar.gz`, or an OCI image. |

## Features

### Deliveries list

One row per delivery target: package type and size, latest build, last delivered revision and its verification badge. It does not summarize what changed. A delivery means something changed, and the delivery page shows what. Computing a diff per row would mean opening an artifact for every row on every load.

### Delivery page

- **Default view:** newest revision compared with the previous one.
- **Single view:** shows one revision without a diff. Reached with "Exit compare", or "View" in History.
- **Base selection:** the base follows the compared revision until the user picks one. After that the chosen base is kept.
- **First revision:** has no previous revision, so it opens in single view.
- A header shows status, package kind, release branch, tag, repository, who delivered it, the CI run, the PR and when. "Open CI run" and "Compare builds" are the primary actions.

### Contents

The package's files with sizes. In compare mode, files are marked added, modified or removed with size changes, and selecting a file shows its diff.

| Format | Diffed |
| --- | --- |
| `.deb` | File list; `DEBIAN/control` and other text files as line diffs; ELF binaries by section sizes, symbols, build ID and strings; other files by size and digest |
| `.tar.gz` | File list; text files as line diffs; other files by size and digest. Content-hashed filenames show as a remove plus an add |
| OCI image | Merged filesystem, with the same per-file treatment |

### Layers (OCI images)

Layers aligned by name, with digest and size. Layers whose digest is unchanged are reported as reused.

### Metadata

Delivery, artifact and build metadata as field/value tables, grouped by section. In compare mode each table has a column per revision and changed values are marked. Sections:

- Delivery: one row per fact, each with its link. Who delivered it and when, the CI workflow and run, the PR and its author, the release branch and what it was cut from, the tag, the full commit SHA and message, the build, and the artifact location. In compare mode a row links to the git diff between base and compared.
- Artifact: the format, size and digest, from the delivery record. What is inside the package, such as a `.deb`'s control file, is visible in the Contents diff, not repeated here.
- Build: Bazel version, toolchain, build time.
- User: metadata the user attaches to every delivery, shown as field/value rows. Nested values are flattened to dotted names, and lists of objects render as small tables. URL values are links.

### Output

How the deliverable target ran for a revision:

- Duration with the exit code next to it, plus build and run time, and queue time for remote runs.
- Whether it executed locally (named runner) or remotely (named worker pool and worker).
- Retries.
- Arguments and environment, collapsed by default. Secret values are masked. The command line is not shown separately.
- Console output.
- Structured output: a JSON document produced by the target.

In compare mode the tab shows the difference between the two runs, with no separate toggle. Argument, environment, console and structured output diffs are line-based. Single view shows one run.

### History

All revisions of the delivery target with version, branch, tag, commit message, when and by whom, the CI run, the PR and its verification badge. It does not summarize changes either. Filterable by version, person, PR and commit. Each row can open that revision alone or compare it with its previous revision.

## Diffing

Diffs are generated by a separate diff service, called synchronously by the gateway. It reads both artifacts from the remote cache through the existing artifact fetch (URI allow-list, caller token) and stores nothing. Results can be reused only through an in-process cache keyed by the digest pair and the differ version, plus an `ETag`.

**Formats in the first version:** OCI image, `.deb`, tar (`.tar.gz`) and ELF.

- **One shape for every kind.** Each format has a reader that streams the artifact and produces the same normalized list of entries: path, size, digest, kind and mode. The diff compares two such lists and does not know the format. A new format means a new reader.
- **`.deb` and `.tar.gz`:** both sides are read once to build the entry lists. Text files then get a line diff, read on demand when a file is selected.
- **OCI:** the image manifest and config are small. The layer lists are compared first, and only layers whose digest differs are read for the file-level diff. Unchanged layers are never fetched.
- **ELF:** when an ELF file is selected, its sections, symbols, build ID and strings are compared. This runs per file, on demand.

**Starting limits, to be set from measurements:**

| Scope | Limit |
| --- | --- |
| `.deb` or `.tar.gz`, per side | 2 GB |
| OCI image, whole image | between 2 GB and 10 GB |
| ELF file, per file | 100 MB |
| Synchronous request | 30 seconds |

Beyond a limit the page says the diff is too large to compute and shows what is cheap to know: the sizes and digests. The OCI layer comparison needs only the manifest, so it works at any image size.

**Time budget.** Reading 2 GB on both sides within 30 seconds needs sustained throughput of roughly 130 MB per second, including decompression, which depends on the format (xz is slowest). That has not been measured. If it is not met, the options are a longer budget with an asynchronous job, or a smaller size limit.

### Measurements from the prototype

A Rust prototype of the diff service was run on an Apple M2 Pro with synthetic artifacts and warm page cache, so IO is optimistic. Times are wall seconds for diffing two 2 GB (compressed) artifacts.

| Format | Single thread | Both sides in parallel | Notes |
| --- | --- | --- | --- |
| `.deb` with xz | 62 to 68 | 30 to 33 | About 10 s with multi-threaded decode of a multi-block xz stream. Single-block xz was measured only at 500 MB (8.3 s). |
| `.deb` with zstd | 9.0 | 2.9 | |
| `.tar.gz` | 12.0 | 4.9 | |
| `.tar.zst` | 8.9 | 3.3 | |
| OCI 2 GB, top layer changed | 0.64 | 0.25 | |
| OCI 8 GB, top layer changed | 3.7 | 1.4 | A 10 GB limit looks workable. 10 GB itself was not tested. |

- **The 2 GB and 30 s limits hold** for gzip and zstd with a large margin. For xz they hold only with parallel decoding, and the bottleneck is xz decode at about 70 MB/s per thread.
- **Per-file requests re-stream the archive,** because archives have no random access. A file late in an xz archive costs about 30 s, in gzip about 5 s and in zstd about 3 s. Files early in the archive are instant.
- **OCI changed-layers-only works,** but only a shared prefix of layers can be skipped, because layers above a changed one can carry whiteouts. In the fast mode a path present on only one side is flagged ambiguous. An exact mode reads every layer.
- **Limits to define precisely:** the 2 GB cap on compressed bytes, a separate cap on decompressed bytes (20 GB in the prototype), an entry limit (500k in the prototype), and an ELF limit that counts the file's content.
- **Not verified:** real remote-cache latency, cold disks, real Debian or image artifacts, and a 10 GB image.

## Relationship to existing features

- **Compare** keeps ownership of build-to-build analysis. The delivery page links to it from the header, from History, and from the build row in Metadata's Delivery section.
- **Delivery** produces the revisions. The delivery page reads what the delivery backend records and adds package contents and run output.
- **AXL** gains no new surface in this proposal.

## Data requirements

What each feature needs, and what the repository records today. "Not found" means the audit did not find it; it is not proof that it is absent from the service.

| Need | Today |
| --- | --- |
| Per-label delivered record with commit and build URL | Recorded by the current delivery state backend. The new backend keeps its own records (see Storage and retention) |
| Content hash of the delivered output | Not found. Delivery's digest is the action digest of a probe action over the target's inputs, not a hash of the output |
| Previously delivered digest or commit for a label | Not returned by the backend |
| Package contents (file list, file bytes) | Not found. Needs the artifact fetched from storage and unpacked per format |
| Run command, exit code, duration | The delivery hook entry carries label, outcome, message, output digest and paths. It has no stdout, stderr, exit code or duration |
| Run environment and retries | Not found |
| Local versus remote execution of the run | Not found |
| Structured output of the target | Discarded before the hook is called |
| User-provided metadata attached to a delivery | Not found. Same gap as structured output: the target's JSON never reaches the hook or the stored result |
| Execution log spawns (args, env, inputs, digests, cache hit, metrics) | Available for builds through the execution log |
| Build metrics | Available through BES |

References: `crates/aspect-cli/src/builtins/aspect/delivery.axl`, `private/lib/delivery_state.axl`, `private/lib/execlog.axl`.

## Storage and retention

Deliveries is served by a new delivery backend with its own database model, and that backend is also the delivery state store: `aspect delivery` can use it in place of deliveryd or Redis. It is one service on the ivy stack and answers four things: record a delivery, ask which output digests were already delivered, a health probe, and the reads behind the Deliveries pages. Its state is scoped by the caller's credential (the organization is never a request parameter), the repository and the delivery run's `--task:name` prefix.

- **Same retention as build data.** A delivery's details (metadata, provenance, user metadata, run output, console output, verification) expire on the same schedule as the build that produced them: the retention window and size cap the rest of the build data follows. History lists the deliveries whose build data still exists.
- **One thing outlives that.** A small table with one row per delivered output digest of a target. It is the delivery state: it exists so delivery can skip an output it already delivered, so it is written only after a successful delivery, and a failed delivery is retried. A target keeps one row per digest, so going back to an earlier digest is still recognized while that row exists. A write removes that target's rows older than the build retention window, never the row it just wrote, so the table needs no retention job and holds no delivery details.
- **Contents come from the cache.** Package contents and diffs are computed on demand from the artifact in the remote cache, not stored. When the artifact has left the cache, the Contents, Layers and diff views say that contents are not available because the artifact is no longer in the cache. The rest of the revision stays visible.
- **No manifest for now.** The backend does not store a file list or any file contents. A stored manifest (paths, sizes, digests) could be added later if the loss of file-list diffs on older revisions proves to matter.
- **Detected when viewed.** The backend finds out an artifact is gone when it tries to read it, so no background sweep is needed.
- **Consequences.** History can list revisions whose contents can no longer be shown. A comparison involving such a revision shows its metadata, output and provenance differences, but not a contents diff.

## Data model

Four tables, following the existing build-data conventions (primary key columns first, no defaults, JSON stored in `VARCHAR`).

- **`delivery`.** One row per target delivered in an invocation, keyed by the partition date, the invocation id and the interned target label. It is a hypertable with a cascading foreign key to `invocation`, so the existing retention window and size cap drop it with the build. Columns: status, delivered time, artifact format, artifact URI, the artifact's real digest and size, version, tag and user metadata. Provenance (branch, commit, PR, author, CI host) is read from the invocation, not copied.
- **`delivery_exec`.** One row per delivery, with the same key and foreign key as `delivery`, describing how the deliverable target ran: command, environment, exit code, duration, execution locus (local or remote), where it ran, retries and structured output. The name follows ivy's `exec_locus` and `execlog`. It expires with the build.
- **`delivery_digest`.** One row per organization, repository, workspace (the `--task:name` prefix), target and output digest, with the artifact's content hash, when it was delivered and the invocation id as a plain id with no foreign key. Written after a successful delivery, even before the build is ingested, and only moved forward by a newer delivery. The delivery list shows each target's newest row.
- **`delivery_log`.** Console output, which can be large, so it has its own table, modeled on `invocation_log`: a hypertable with a cascading foreign key to `invocation`, keyed by the delivery's key plus a sequence number, with `stdout`, `stderr` and a timestamp, LZ4-compressed. It expires with the build like `delivery`.

A stored file list (a narrow row per file, like `named_set_file`) can be added later without changing these tables.

## Open questions

- Whether xz-compressed `.deb` files should be allowed at 2 GB, given they need parallel decoding to fit 30 seconds, and whether the OCI limit is 2 GB or 10 GB. The prototype numbers use synthetic data and need confirming against real artifacts.
- How to keep per-file diffs fast on large archives, since each one re-streams the archive.
- Whether an asynchronous job is needed for requests that exceed the synchronous budget.
- The size cap for a text file's line diff, and the limit on how many entries an artifact may have.
- What is a delivery target: a Bazel target, a destination environment, or both? Delivery tracks the first only; a destination would be a separate concept, a deployment.
- Should the diff baseline be "previous revision", "last delivered to an environment", or any selectable revision? The proposal assumes previous by default and any revision on request. "Previous" across release lines (for example the first candidate of 4.2 against 4.1.0) needs a rule.
- How is output capture implemented, and where does the delivery run execute (runner or service)? Delivery moving off the runner to the cloud service changes who can record environment data.
- Which environment values are secret? Masking by name is not sufficient on its own.
- Where the evidence for each test (exact, graph, declared) is stored. It is derived from execution-log inputs and exists nowhere else today, so it needs rows that expire with the build.
- Whether delivery targets can return structured output, and through which mechanism, given the no-new-API constraint. User-provided metadata on every delivery depends on the same answer: where it is declared, its size limit, and whether it is stored with the delivery result and returned when a previous result is reused.
