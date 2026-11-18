//! `SpawnExec` as Bazel writes it to `--execution_log_json_file`: protobuf's
//! JSON mapping, as Java's `JsonFormat.printer().alwaysPrintFieldsWithNoPresence()`
//! prints it. Fields go in field-number order under their lowerCamelCase
//! names; scalars and repeated fields print even when empty, message fields
//! only when set; 64-bit integers are strings, durations `"1.500s"`,
//! timestamps RFC 3339 in UTC. Each message is pretty-printed, two-space
//! indented, and messages follow one another with nothing between. Only the
//! fields the CLI's protos (Bazel 8.0's) know are printed, so a later
//! Bazel's `measuredMemoryPeakBytes` is not.

use std::fmt::Write as _;

use axl_proto::tools::protos::{
    Digest, EnvironmentVariable, File, Platform, SpawnExec, SpawnMetrics, platform,
};

/// A JSON value in the shape the printer lays out.
enum Value {
    Raw(String),
    Array(Vec<Value>),
    Object(Vec<(&'static str, Value)>),
}

fn string(s: &str) -> Value {
    Value::Raw(quote(s))
}

fn int64(n: i64) -> Value {
    Value::Raw(format!("\"{n}\""))
}

fn int32(n: i32) -> Value {
    Value::Raw(n.to_string())
}

fn boolean(b: bool) -> Value {
    Value::Raw(b.to_string())
}

fn strings(items: &[String]) -> Value {
    Value::Array(items.iter().map(|s| string(s)).collect())
}

/// A JSON string as protobuf's printer escapes it: the JSON escapes, control
/// characters and the HTML-sensitive `<>&='` as `\uXXXX`, and the line and
/// paragraph separators.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `seconds` and `nanos` as a protobuf JSON duration's or timestamp's
/// fraction: none, or 3, 6 or 9 digits.
fn fraction(nanos: i32) -> String {
    let nanos = nanos.unsigned_abs();
    if nanos == 0 {
        String::new()
    } else if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!(".{:06}", nanos / 1_000)
    } else {
        format!(".{nanos:09}")
    }
}

fn duration(d: &prost_types::Duration) -> Value {
    let negative = d.seconds < 0 || d.nanos < 0;
    let sign = if negative { "-" } else { "" };
    Value::Raw(format!(
        "\"{sign}{}{}s\"",
        d.seconds.unsigned_abs(),
        fraction(d.nanos)
    ))
}

fn timestamp(t: &prost_types::Timestamp) -> Value {
    let days = t.seconds.div_euclid(86_400);
    let secs = t.seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    Value::Raw(format!(
        "\"{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{}Z\"",
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        fraction(t.nanos)
    ))
}

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn digest(d: &Digest) -> Value {
    Value::Object(vec![
        ("hash", string(&d.hash)),
        ("sizeBytes", int64(d.size_bytes)),
        ("hashFunctionName", string(&d.hash_function_name)),
    ])
}

fn file(f: &File) -> Value {
    let mut fields = vec![("path", string(&f.path))];
    if let Some(d) = &f.digest {
        fields.push(("digest", digest(d)));
    }
    fields.push(("isTool", boolean(f.is_tool)));
    fields.push(("symlinkTargetPath", string(&f.symlink_target_path)));
    Value::Object(fields)
}

fn environment_variable(e: &EnvironmentVariable) -> Value {
    Value::Object(vec![("name", string(&e.name)), ("value", string(&e.value))])
}

fn property(p: &platform::Property) -> Value {
    Value::Object(vec![("name", string(&p.name)), ("value", string(&p.value))])
}

fn platform(p: &Platform) -> Value {
    Value::Object(vec![(
        "properties",
        Value::Array(p.properties.iter().map(property).collect()),
    )])
}

fn metrics(m: &SpawnMetrics) -> Value {
    let mut fields = Vec::new();
    let mut time = |name, d: &Option<prost_types::Duration>| {
        if let Some(d) = d {
            fields.push((name, duration(d)));
        }
    };
    time("totalTime", &m.total_time);
    time("parseTime", &m.parse_time);
    time("networkTime", &m.network_time);
    time("fetchTime", &m.fetch_time);
    time("queueTime", &m.queue_time);
    time("setupTime", &m.setup_time);
    time("uploadTime", &m.upload_time);
    time("executionWallTime", &m.execution_wall_time);
    time("processOutputsTime", &m.process_outputs_time);
    time("retryTime", &m.retry_time);
    fields.push(("inputBytes", int64(m.input_bytes)));
    fields.push(("inputFiles", int64(m.input_files)));
    fields.push(("memoryEstimateBytes", int64(m.memory_estimate_bytes)));
    fields.push(("inputBytesLimit", int64(m.input_bytes_limit)));
    fields.push(("inputFilesLimit", int64(m.input_files_limit)));
    fields.push(("outputBytesLimit", int64(m.output_bytes_limit)));
    fields.push(("outputFilesLimit", int64(m.output_files_limit)));
    fields.push(("memoryBytesLimit", int64(m.memory_bytes_limit)));
    if let Some(d) = &m.time_limit {
        fields.push(("timeLimit", duration(d)));
    }
    if let Some(t) = &m.start_time {
        fields.push(("startTime", timestamp(t)));
    }
    Value::Object(fields)
}

/// `spawn` as Bazel's JSON log prints it.
pub fn spawn_exec(spawn: &SpawnExec) -> String {
    let mut fields = vec![
        ("commandArgs", strings(&spawn.command_args)),
        (
            "environmentVariables",
            Value::Array(
                spawn
                    .environment_variables
                    .iter()
                    .map(environment_variable)
                    .collect(),
            ),
        ),
    ];
    if let Some(p) = &spawn.platform {
        fields.push(("platform", platform(p)));
    }
    fields.push((
        "inputs",
        Value::Array(spawn.inputs.iter().map(file).collect()),
    ));
    fields.push(("listedOutputs", strings(&spawn.listed_outputs)));
    fields.push(("remotable", boolean(spawn.remotable)));
    fields.push(("cacheable", boolean(spawn.cacheable)));
    fields.push(("timeoutMillis", int64(spawn.timeout_millis)));
    fields.push(("mnemonic", string(&spawn.mnemonic)));
    fields.push((
        "actualOutputs",
        Value::Array(spawn.actual_outputs.iter().map(file).collect()),
    ));
    fields.push(("runner", string(&spawn.runner)));
    fields.push(("cacheHit", boolean(spawn.cache_hit)));
    fields.push(("status", string(&spawn.status)));
    fields.push(("exitCode", int32(spawn.exit_code)));
    fields.push(("remoteCacheable", boolean(spawn.remote_cacheable)));
    fields.push(("targetLabel", string(&spawn.target_label)));
    if let Some(d) = &spawn.digest {
        fields.push(("digest", digest(d)));
    }
    if let Some(m) = &spawn.metrics {
        fields.push(("metrics", metrics(m)));
    }
    let mut out = String::new();
    print(&Value::Object(fields), 0, &mut out);
    out
}

/// Lay `value` out as `JsonFormat`'s pretty printer does: an object's fields
/// one per line, indented two spaces a level; an array's elements on its
/// opening line, `, `-separated.
fn print(value: &Value, indent: usize, out: &mut String) {
    match value {
        Value::Raw(raw) => out.push_str(raw),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print(item, indent, out);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push_str("{\n");
            for (i, (name, field)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&"  ".repeat(indent + 1));
                let _ = write!(out, "\"{name}\": ");
                print(field, indent + 1, out);
            }
            out.push('\n');
            out.push_str(&"  ".repeat(indent));
            out.push('}');
        }
    }
}
