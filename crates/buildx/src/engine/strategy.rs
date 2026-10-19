//! The Rust `SpawnStrategy` registered in front of Bazel's local one.
//!
//! `canExec` is where a spawn is routed: it reads the spawn, asks the router
//! for an executor and answers false when Bazel's own strategy runs it (the
//! local executor), so the next registered strategy, Bazel's, executes with
//! its exceptions and output handling intact. A routing error answers true and
//! `exec` fails the spawn with the error as its failure message.
//!
//! A strategy cannot throw the checked `ExecException` family from Rust, so a
//! failing spawn can only be a `SpawnResult`; keeping local execution in
//! Bazel's strategy is what keeps a non-zero exit an action failure.

use super::*;
use crate::execution::{ExecOutcome, Executor, Router, SpawnView};
use crate::reapi::SpawnData;
use blazon_rt::Result as RtResult;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    /// A routing error, from `canExec` to the `exec` that follows on the same thread.
    static ROUTING_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Routing errors of one build, by target label, so a failed action's message can be the error's.
pub type RouteErrors = Arc<std::sync::Mutex<Vec<(String, String)>>>;

pub(super) struct Strategy {
    pub router: Arc<dyn Router>,
    pub errors: RouteErrors,
    /// Bazel's own local strategy, which local executors wrap.
    pub delegate: Global<bazel::StandaloneSpawnStrategy<'static>>,
    /// The same strategy over Bazel's sandboxed runner, when the build needs one.
    pub sandbox: Option<Global<bazel::StandaloneSpawnStrategy<'static>>>,
    pub counters: Arc<metrics::Counters>,
}

pub(super) fn map<'a>(
    m: &bazel::ImmutableMap<'a, bazel::JavaString<'static>, bazel::JavaString<'static>>,
) -> RtResult<Vec<(String, String)>> {
    let mut out = Vec::new();
    for e in m
        .upcast::<bazel::Map<bazel::JavaString, bazel::JavaString>>()
        .entries()?
    {
        let e = e?;
        out.push((e.key()?.to_rust()?, e.value()?.to_rust()?));
    }
    Ok(out)
}

/// The spawn's plain data.
fn read(spawn: &bazel::Spawn) -> RtResult<SpawnData> {
    let mut arguments = Vec::new();
    for a in &spawn
        .arguments()?
        .upcast::<bazel::Iterable<bazel::JavaString>>()
    {
        arguments.push(a?.to_rust()?);
    }
    let mut outputs = Vec::new();
    for o in &spawn
        .output_files()?
        .upcast::<bazel::Iterable<bazel::ActionInput>>()
    {
        outputs.push(o?.exec_path_string()?);
    }
    let execution_info = map(&spawn.execution_info()?)?;
    let timeout_seconds = execution_info
        .iter()
        .find(|(k, _)| k == "timeout")
        .and_then(|(_, v)| v.parse().ok());
    Ok(SpawnData {
        mnemonic: spawn.mnemonic()?,
        arguments,
        environment: map(&spawn.environment()?)?,
        execution_info,
        platform_properties: map(&spawn.combined_exec_properties()?)?,
        outputs,
        working_directory: String::new(),
        timeout_seconds,
    })
}

/// A failed spawn the way Bazel's own strategies report one: a
/// `SpawnExecException` holding a failed `SpawnResult`, with an exit code and a
/// failure detail. A non-zero exit is `NON_ZERO_EXIT`; anything else
/// (transport, a missing output, routing) is `EXECUTION_FAILED` with exit code 1.
pub(super) fn failure<'a>(
    env: &mut JNIEnv<'a>,
    message: &str,
    exit_code: i32,
    runner: &str,
) -> RtResult<blazon_rt::Error> {
    let non_zero = exit_code != 0;
    let code = if non_zero {
        bazel::SpawnDetailCode::non_zero_exit(env)?
    } else {
        bazel::SpawnDetailCode::execution_failed(env)?
    };
    let status = if non_zero {
        bazel::SpawnStatus::non_zero_exit(env)?
    } else {
        bazel::SpawnStatus::execution_failed(env)?
    };
    let spawn_detail = bazel::SpawnDetail::new_builder(env)?
        .set_code(&code)?
        .build()?;
    let detail = bazel::FailureDetail::new_builder(env)?
        .set_message(message)?
        .set_spawn(&spawn_detail)?
        .build()?;
    let result = bazel::SpawnResultBuilder::new(env)?
        .set_failure_detail(&detail)?
        .set_status(&status)?
        .set_exit_code(if non_zero { exit_code } else { 1 })?
        .set_failure_message(message)?
        .set_runner_name(runner)?
        .build()?;
    Ok(bazel::SpawnExecException::new(env, message, &result, false)?.into())
}

fn succeeded<'a>(
    env: &mut JNIEnv<'a>,
    runner: &str,
    cache_hit: bool,
) -> RtResult<bazel::SpawnResult<'a>> {
    bazel::SpawnResultBuilder::new(env)?
        .set_status(&bazel::SpawnStatus::success(env)?)?
        .set_exit_code(0)?
        .set_runner_name(runner)?
        .set_cache_hit(cache_hit)?
        .build()
}

fn one<'a>(
    env: &mut JNIEnv<'a>,
    result: bazel::SpawnResult<'a>,
) -> RtResult<bazel::ImmutableList<'a, bazel::SpawnResult<'static>>> {
    let list = bazel::List::<bazel::SpawnResult>::new(env)?;
    list.add(&result)?;
    bazel::ImmutableList::copy_of::<bazel::SpawnResult>(env, &list)
}

/// The runner name of a strategy's results, which is how Bazel counts them:
/// `local`, `darwin-sandbox`, `processwrapper-sandbox`.
fn runner_of(results: &bazel::ImmutableList<'_, bazel::SpawnResult<'static>>) -> RtResult<String> {
    let list = results.upcast::<bazel::List<bazel::SpawnResult>>();
    if list.is_empty()? {
        return Ok("local".to_string());
    }
    list.get(0)?.runner_name()
}

type Chosen = (Arc<dyn Executor>, Arc<SpawnView>);

thread_local! {
    /// The executor `canExec` chose, for the `exec` that follows on the same thread.
    static CHOSEN: RefCell<Option<Chosen>> = const { RefCell::new(None) };
}

impl bazel::SpawnStrategyImpl for Strategy {
    fn exec<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        spawn: bazel::Spawn<'a>,
        context: bazel::ActionExecutionContext<'a>,
    ) -> RtResult<bazel::ImmutableList<'a, bazel::SpawnResult<'static>>> {
        let _ = spawn;
        if let Some(message) = ROUTING_ERROR.with(|e| e.borrow_mut().take()) {
            context
                .file_out_err()?
                .upcast::<bazel::OutErr>()
                .print_err_ln(&message)?;
            return Err(failure(env, &message, 0, "buildx-route")?);
        }
        let Some((executor, view)) = CHOSEN.with(|c| c.borrow_mut().take()) else {
            return Err(failure(
                env,
                "no executor was chosen for the spawn",
                0,
                "buildx-route",
            )?);
        };
        let mnemonic = view.data.mnemonic.clone();
        match executor.exec(&view) {
            Ok(ExecOutcome::Bazel(results)) => {
                let results = results.get(env)?;
                self.counters.local_runner(&runner_of(&results)?, &mnemonic);
                Ok(results)
            }
            Ok(ExecOutcome::Done(done)) => {
                if done.cache_hit {
                    self.counters.cache_hit(&mnemonic);
                } else if done.runner.starts_with("remote") {
                    self.counters.remote(&mnemonic);
                } else {
                    self.counters.local(&mnemonic);
                }
                if let Err(e) = view.write_streams(&done.stdout, &done.stderr) {
                    return Err(failure(env, &format!("{e:#}"), 0, &done.runner)?);
                }
                if done.exit_code == 0 {
                    let result = succeeded(env, &done.runner, done.cache_hit)?;
                    return one(env, result);
                }
                let mut message = view
                    .describe_failure()
                    .unwrap_or_else(|_| "the spawn failed".to_string());
                let stderr = String::from_utf8_lossy(&done.stderr);
                let tail: String = stderr
                    .trim()
                    .chars()
                    .rev()
                    .take(500)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if !tail.is_empty() {
                    message.push_str(&format!("\n{} said: {tail}", done.runner));
                }
                Err(failure(env, &message, done.exit_code, &done.runner)?)
            }
            Err(e) => {
                // An exception from a Java delegate (Bazel's own strategy) goes on unchanged.
                let e = match e.downcast::<blazon_rt::Error>() {
                    Ok(java) if java.throwable().is_some() => return Err(java),
                    Ok(other) => anyhow::Error::new(other),
                    Err(e) => e,
                };
                let message = format!("{e:#}");
                let _ = view.write_streams(&[], message.as_bytes());
                Err(failure(env, &message, 0, executor.name())?)
            }
        }
    }

    fn can_exec<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        spawn: bazel::Spawn<'a>,
        registry: bazel::ActionContextActionContextRegistry<'a>,
    ) -> RtResult<bool> {
        let _ = env;
        let context = registry.downcast::<bazel::ActionExecutionContext>()?;
        let data = read(&spawn)?;
        let view = Arc::new(SpawnView::new(
            data,
            spawn.to_global()?,
            context.to_global()?,
            self.delegate.clone(),
            self.sandbox.clone(),
        ));
        Ok(match self.router.route(&view) {
            Ok(executor) if executor.runs_in_bazel() => {
                self.counters.local(&view.data.mnemonic);
                false
            }
            Ok(executor) => {
                CHOSEN.with(|c| *c.borrow_mut() = Some((executor, view)));
                true
            }
            Err(e) => {
                let message = format!("{e:#}");
                let label = match spawn.target_label()? {
                    Some(l) => l.to_string(),
                    None => String::new(),
                };
                if let Ok(mut errors) = self.errors.lock() {
                    errors.push((label, message.clone()));
                }
                ROUTING_ERROR.with(|slot| *slot.borrow_mut() = Some(message));
                true
            }
        })
    }
}
