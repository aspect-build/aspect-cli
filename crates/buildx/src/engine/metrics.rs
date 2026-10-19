//! What a build did, for the `build_metrics` event.
//!
//! Bazel's `MetricsCollector` is package private and takes a `CommandEnvironment`,
//! so it cannot be built here. What it reports is gathered from what can be
//! read without one: the persistent action cache's own statistics (which
//! `ActionCache.mergeIntoActionCacheStatistics` fills, as `ExecutionTool` does),
//! the package manager's statistics, Bazel's progress counter, and counters of
//! our own on the spawn strategy, because the strategy is where each spawn is
//! routed and its runner is known.

use axl_proto::build_event_stream::build_metrics as bm;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The mnemonic of `BazelWorkspaceStatusAction`.
const STATUS_MNEMONIC: &str = "BazelWorkspaceStatusAction";

/// How spawns of one build ran. Atomics, because spawns finish on many threads;
/// a spawn is counted once, as one of the three.
#[derive(Default)]
pub struct Counters {
    local: AtomicU64,
    remote: AtomicU64,
    cache_hits: AtomicU64,
    mnemonics: Mutex<BTreeMap<String, u64>>,
    /// Spawns run by a sandboxed runner, by the runner's name (`darwin-sandbox`).
    sandboxed: Mutex<BTreeMap<String, u64>>,
}

impl Counters {
    fn note(&self, mnemonic: &str) {
        if let Ok(mut m) = self.mnemonics.lock() {
            *m.entry(mnemonic.to_string()).or_default() += 1;
        }
    }

    /// A spawn run by Bazel's local runner.
    pub fn local(&self, mnemonic: &str) {
        self.local.fetch_add(1, Ordering::Relaxed);
        self.note(mnemonic);
    }

    /// A spawn run by Bazel's local strategy under the runner name its result
    /// carries: `local`, or a sandboxed runner's, which Bazel counts apart.
    pub fn local_runner(&self, runner: &str, mnemonic: &str) {
        if runner == "local" {
            return self.local(mnemonic);
        }
        if let Ok(mut m) = self.sandboxed.lock() {
            *m.entry(runner.to_string()).or_default() += 1;
        }
        self.note(mnemonic);
    }

    /// A spawn run on a remote executor.
    pub fn remote(&self, mnemonic: &str) {
        self.remote.fetch_add(1, Ordering::Relaxed);
        self.note(mnemonic);
    }

    /// A spawn answered from a remote action cache: it did not run, and is not a local run.
    pub fn cache_hit(&self, mnemonic: &str) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
        self.note(mnemonic);
    }
}

/// Everything the metrics event states, filled while the build runs.
#[derive(Default, Clone)]
pub struct Collected {
    pub local: u64,
    pub remote: u64,
    pub cache_hits: u64,
    pub sandboxed: Vec<(String, u64)>,
    pub mnemonics: Vec<(String, u64)>,
    /// The action cache's statistics, as Bazel counted them.
    pub cache: Option<axl_proto::blaze::ActionCacheStatistics>,
    /// Actions enqueued by the build.
    pub actions_enqueued: u64,
    pub targets_loaded: i64,
    pub targets_configured: i64,
    pub packages_loaded: i64,
    pub analysis_ms: i64,
    pub execution_ms: i64,
    /// From the start of the build to the first action.
    pub execution_start_ms: i64,
    pub cpu_ms: i64,
    /// Bazel's workspace status action ran: an executed action without a
    /// spawn, counted as `internal`, with its own `action_data` entry.
    pub status_ran: bool,
}

impl Collected {
    pub fn take(&mut self, counters: &Counters) {
        self.local = counters.local.load(Ordering::Relaxed);
        self.remote = counters.remote.load(Ordering::Relaxed);
        self.cache_hits = counters.cache_hits.load(Ordering::Relaxed);
        self.sandboxed = counters
            .sandboxed
            .lock()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
        self.mnemonics = counters
            .mnemonics
            .lock()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
    }

    /// The protocol buffer. Names and kinds are Bazel's: `total`, then one entry
    /// per runner (`remote cache hit`, `remote`, `local`), then `internal`, the
    /// executed actions that had no spawn.
    pub fn proto(&self, wall_ms: i64) -> axl_proto::build_event_stream::BuildMetrics {
        let spawns = self.local
            + self.remote
            + self.cache_hits
            + self.sandboxed.iter().map(|(_, n)| n).sum::<u64>();
        // Bazel's own executed-action count is the action cache's misses: an action
        // is executed exactly when its cache entry did not satisfy it.
        let executed = self
            .cache
            .as_ref()
            .map(|c| c.misses.max(0) as u64)
            .unwrap_or(spawns)
            .max(spawns);
        let count = |name: &str, n: u64, kind: &str| bm::action_summary::RunnerCount {
            name: name.to_string(),
            count: n as i32,
            exec_kind: kind.to_string(),
        };
        let mut runners = vec![count("total", executed, "")];
        if self.cache_hits > 0 {
            runners.push(count("remote cache hit", self.cache_hits, "Remote"));
        }
        if self.remote > 0 {
            runners.push(count("remote", self.remote, "Remote"));
        }
        if self.local > 0 {
            runners.push(count("local", self.local, "Local"));
        }
        for (name, n) in &self.sandboxed {
            runners.push(count(name, *n, "Local"));
        }
        if executed > spawns {
            runners.push(count("internal", executed - spawns, ""));
        }
        axl_proto::build_event_stream::BuildMetrics {
            action_summary: Some(bm::ActionSummary {
                actions_created: self.actions_enqueued as i64,
                actions_created_not_including_aspects: self.actions_enqueued as i64,
                actions_executed: executed as i64,
                action_data: self
                    .mnemonics
                    .iter()
                    .map(|(mnemonic, n)| (mnemonic.clone(), *n))
                    .chain(self.status_ran.then(|| (STATUS_MNEMONIC.to_string(), 1)))
                    .map(|(mnemonic, n)| bm::action_summary::ActionData {
                        mnemonic,
                        actions_executed: n as i64,
                        ..Default::default()
                    })
                    .collect(),
                runner_count: runners,
                action_cache_statistics: self.cache.clone(),
                ..Default::default()
            }),
            target_metrics: Some(bm::TargetMetrics {
                targets_loaded: self.targets_loaded,
                targets_configured: self.targets_configured,
                ..Default::default()
            }),
            package_metrics: Some(bm::PackageMetrics {
                packages_loaded: self.packages_loaded,
                ..Default::default()
            }),
            timing_metrics: Some(bm::TimingMetrics {
                cpu_time_in_ms: self.cpu_ms,
                wall_time_in_ms: wall_ms,
                analysis_phase_time_in_ms: self.analysis_ms,
                execution_phase_time_in_ms: self.execution_ms,
                actions_execution_start_in_ms: self.execution_start_ms,
            }),
            ..Default::default()
        }
    }
}

/// CPU time this process has used so far (user and system), in milliseconds.
pub fn cpu_ms() -> i64 {
    // SAFETY: `usage` is a plain struct getrusage fills in.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
            return 0;
        }
        usage
    };
    // The field types differ by platform.
    #[allow(clippy::unnecessary_cast)]
    let ms = |t: libc::timeval| t.tv_sec as i64 * 1000 + t.tv_usec as i64 / 1000;
    ms(usage.ru_utime) + ms(usage.ru_stime)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runners(c: &Collected) -> Vec<(String, i32)> {
        let m = c.proto(1);
        m.action_summary
            .unwrap()
            .runner_count
            .into_iter()
            .map(|r| (r.name, r.count))
            .collect()
    }

    #[test]
    fn the_status_action_is_the_internal_action() {
        // Three genrules and the status action: the action cache missed on four
        // and three of them spawned.
        let c = Collected {
            local: 3,
            status_ran: true,
            cache: Some(axl_proto::blaze::ActionCacheStatistics {
                misses: 4,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            runners(&c),
            [
                ("total".to_string(), 4),
                ("local".to_string(), 3),
                ("internal".to_string(), 1)
            ]
        );
        let data = c.proto(1).action_summary.unwrap().action_data;
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].mnemonic, "BazelWorkspaceStatusAction");
        assert_eq!(data[0].actions_executed, 1);
    }

    #[test]
    fn without_the_status_action_there_is_no_entry() {
        let c = Collected {
            local: 3,
            ..Default::default()
        };
        assert!(c.proto(1).action_summary.unwrap().action_data.is_empty());
    }
}
