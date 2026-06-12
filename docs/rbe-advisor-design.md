# RBE Advisor — Design Doc

**Status:** Draft for team discussion
**Author:** Jeff Pignataro (drafted with Claude)
**Date:** 2026-06-12

## 1. Problem

Teams evaluating remote build execution (RBE) for a Bazel repository face two questions that today require expensive manual discovery: *will my build actually work remotely* (hermeticity), and *what infrastructure do I need to run it well* (capacity). We want a tool an engineer can run against a local checkout that produces a hermeticity readiness report and a capacity plan — estimated cores, memory, and instance counts (EC2/GCE) to run the repo's tests on RBE efficiently and reliably.

## 2. Goals and non-goals

The tool must run with no RBE backend present: everything is derived from static analysis of the workspace plus instrumented local builds. It must be backend-agnostic — output is a generic REAPI worker-pool spec (worker classes × counts) mapped to concrete AWS and GCP instance types. It should be cheap to run (one full local test run, or zero builds in static-only mode) and produce both a human report and machine-readable JSON.

Non-goals for v1: scoring build *correctness* beyond hermeticity (e.g., flakiness root-causing), cost optimization across spot/reserved pricing, and live integration with a running RBE cluster. CI-telemetry ingestion (BEP/exec logs from Buildkite artifacts) is a v2 extension, not v1.

## 3. Data sources

| Source | How obtained | What it gives us |
|---|---|---|
| `bazel query --output=xml` | static | Test targets, `size`, `timeout`, `tags`, `flaky`, rule classes, `local=True` attrs |
| `bazel cquery`/`aquery` | static (analysis phase) | Action mnemonics, input/output counts → cache traffic estimate |
| `.bazelrc` chain + `MODULE.bazel`/`WORKSPACE` | static | Strategy flags, env leakage, unpinned repos, toolchain hermeticity |
| Execution log (`--execution_log_json_file` or compact) | one instrumented run | Per-spawn: runner, cacheability, remote-cacheable bit, wall time, input/output digests and sizes, env |
| JSON profile (`--profile`) | same run | Action timeline → total CPU-seconds, concurrency curve, critical path |
| `test.xml` / BEP | same run | Per-test (and per-shard) durations |
| Second instrumented run (optional) | determinism check | Output digest diffs → non-determinism detection |

The collector writes all of this into a **bundle**: a directory with the raw artifacts plus a normalized SQLite database (`bundle.db`). All analysis runs offline against the bundle, so an engineer can collect on their machine and share the bundle for analysis, and we can build future analyzers without re-running builds.

## 4. Hermeticity analysis

Two layers: static checks that run in seconds without building, and dynamic checks derived from the instrumented run(s).

**Static checks.** Tag-based: `local`, `no-sandbox`, `no-cache`, `no-remote`, `no-remote-exec`, `requires-network`, `exclusive` — each marks a target that will degrade or break under RBE. Attribute-based: `local = True` on genrules/tests. Configuration: `--spawn_strategy=local|standalone`, broad `--action_env`/`--test_env` passthrough, absence of `--incompatible_strict_action_env`, sandboxing disabled, `--workspace_status_command` producing volatile keys consumed by non-stamp actions. Dependency hygiene: `http_archive`/`git_repository` without integrity pins, repository rules invoking system binaries, autodetected (non-hermetic) C++/JVM toolchains.

**Dynamic checks.** From the exec log: spawns that actually ran with the `local` runner despite no tag (strategy fallbacks), spawns marked not-remote-cacheable, absolute paths appearing in command lines or env vars, env vars referencing `$HOME`/user paths, and input files outside the workspace/external roots. With a second clean run: actions whose output digests differ between runs → non-deterministic (timestamps, archive ordering, embedded paths) — these poison the remote cache and must be fixed or tagged.

**Output.** A readiness score (weighted by how many CPU-seconds of the build the violating targets represent, not just target count — one `local` test that's 40% of test wall time matters more than fifty trivial ones), and a severity-ranked violation table: *blocker* (will fail or must run locally), *non-hermetic* (machine-varying inputs leak into actions or caches), *efficiency* (uncacheable/unshardable), *advisory* (deliberate opt-outs like `no-remote`/`no-remote-exec` — surfaced for visibility but excluded from the score, since a declared local-only target is a choice, not a hermeticity defect), each with the concrete remediation (tag to remove, flag to set, toolchain to pin). Remediations acknowledge legitimate trade-offs: for a large non-deterministic output (archive, container image), `no-remote-cache` can be the *right* configuration — rebuilding each run is often faster and cheaper than the I/O and network cost of pushing/pulling the blob through the cache.

## 5. Sizing model

**Per-test resource demand.** CPU: 1 core unless a `cpu:N` tag or `resources:` tag overrides. Memory: measured where available, else Bazel size-class heuristics (small 20 MB, medium 100 MB, large 300 MB, enormous 800 MB) inflated by a safety factor, with measured wall time from the exec log / test.xml replacing the timeout class. Sharded tests are modeled per-shard.

**Throughput model.** From the profile we get total compute `C` (CPU-seconds across all test/action spawns) and the critical path `T_cp`. Wall time on a pool of `W` executor slots is approximately `T(W) = max(T_cp, C / (W × u))` where `u` is utilization efficiency (default 0.8, accounting for scheduling gaps and I/O stalls). Given a target wall time `T*` (user flag, default: 2× critical path), required slots are `W = ⌈C / (T* × u)⌉`. The tool also reports the concurrency curve from the profile so users see the marginal value of more slots, and warns when `T*` < `T_cp` (unachievable without breaking the critical path — usually one long `enormous` test, which we name).

**Worker classes and packing.** Tests are binned into worker classes by their (cpu, mem) demand — default classes: standard (1 cpu / 4 GB), medium (2 cpu / 8 GB), large (4 cpu / 16 GB), with class boundaries derived from the actual demand distribution rather than fixed. Per class we compute peak concurrent demand (p95 of the class's concurrency timeline, plus configurable headroom, default 20%). Executors pack onto instances as `min(⌊vCPU/class_cpu⌋, ⌊RAM/class_mem⌋)` per instance, leaving 1 vCPU / 2 GB for the worker agent and OS. Instance counts come from `⌈slots_needed / slots_per_instance⌉`.

**Cloud mapping.** A static table of sensible defaults — AWS: c7a/m7a families (compute- vs memory-leaning classes); GCP: c2d/n2d equivalents — chosen per class by best slots-per-dollar fit at on-demand pricing ratios (cpu:mem ratio matching, not live pricing in v1). Output includes a min (steady-state) and max (peak, for autoscaling) instance count per class, scaled by a `--builds-per-hour` concurrency multiplier for shared CI pools.

**Efficacy estimate.** From cacheability bits and action input volatility we report: % of compute that is remotely executable, % cacheable, and a projected wall-time range for a warm-cache incremental run vs. cold full run — this is the "is RBE worth it here" number.

## 6. CLI

```
rbe-advisor collect [--targets //...] [--runs 2] [-o bundle/]   # instrumented local run(s) → bundle
rbe-advisor check   [--bundle bundle/ | --workspace .]          # hermeticity report (static-only without bundle)
rbe-advisor size    --bundle bundle/ [--target-wall-time 10m]
                    [--cloud aws|gcp|both] [--headroom 0.2]
                    [--builds-per-hour N] [--utilization 0.8]
rbe-advisor report  --bundle bundle/ [--format md|json]          # full combined report
```

`collect` is the only command that touches Bazel; everything else is offline on the bundle, which keeps the analysis testable and lets us ingest CI-produced logs later by writing an alternate collector.

## 7. Architecture

```
collectors/            analyzers/              renderers/
  bazel_local.py  ─┐     hermeticity.py  ─┐      markdown.py
  (v2: ci_bep.py) ─┤──►  sizing.py       ─┤──►   json.py
                   │     efficacy.py     ─┘
                   ▼
              bundle/ (raw artifacts + SQLite)
```

The normalized SQLite schema (tables: `targets`, `spawns`, `profile_spans`, `violations`) is the contract between collection and analysis. New checks are rows-in/rows-out queries, which keeps them independently testable.

## 8. Risks and open questions

Memory measurement is the weakest input: Bazel doesn't report per-spawn peak RSS in the exec log on all platforms, so v1 leans on size-class heuristics with a safety factor; we should validate against real RBE telemetry from a few known deployments and tune the inflation factor. Local timing ≠ remote timing: remote executors differ in single-core perf and add I/O for input fetching; we apply a configurable remote-overhead factor (default 1.15) and should calibrate it the same way. Finally, the concurrency multiplier for shared pools assumes Poisson-ish CI arrivals; bursty merge-train traffic may need a peak-window model in v2.

## 9. Phasing

v0 (prototype, this session): static hermeticity checks, exec-log/profile ingestion, sizing math, EC2/GCE mapping, markdown+JSON report. v1: SQLite bundle, dynamic determinism check (two-run digest diff), per-shard modeling, validated heuristic factors. v2: CI telemetry collectors (Buildkite artifacts/BEP), historical trending, live pricing.
