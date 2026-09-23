//! Turning a failure back into an error value, for `future.catch()`.

use starlark::eval::Evaluator;
use starlark::values::Value;

use super::raise::RaisedError;
use super::value::ErrorValue;

/// `err` as an error value on the evaluator's heap: the value itself when it
/// was raised with `fail(e)`, or else a plain `error` whose message is the top
/// of the chain and whose `cause` holds the rest, one link per `source()`.
pub(crate) fn error_value_of<'v>(
    err: &anyhow::Error,
    eval: &mut Evaluator<'v, '_, '_>,
) -> Value<'v> {
    if let Some(value) = RaisedError::from_anyhow(err).and_then(|r| r.value(eval)) {
        return value;
    }
    let heap = eval.heap();
    let stack = eval.call_stack();
    let links: Vec<String> = err.chain().map(ToString::to_string).collect();
    links
        .into_iter()
        .rev()
        .fold(Value::new_none(), |cause, message| {
            heap.alloc_complex(ErrorValue {
                typ: Value::new_none(),
                message,
                cause,
                values: Box::new([]),
                stack: stack.clone(),
            })
        })
}

/// A callback's error as an `anyhow::Error`, keeping a raised error value
/// intact so a later `catch()` can still recover it. Anything else is
/// flattened to its rendering, as before.
pub(crate) fn callback_error(err: starlark::Error) -> anyhow::Error {
    if RaisedError::from_starlark(&err).is_some() {
        match err.into_kind() {
            starlark::ErrorKind::Native(e) | starlark::ErrorKind::Other(e) => return e,
            _ => unreachable!("from_starlark only matches Native and Other"),
        }
    }
    anyhow::anyhow!("{}", err)
}
