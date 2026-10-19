//! The event-graph check, the fake Build Event Service, and (ignored by default,
//! it needs a Bazel release in the buildx cache) real builds through the file and
//! BES sinks.

use axl_proto::build_event_stream::{
    BuildEvent, BuildEventId, BuildFinished, build_event, build_event_id,
};
use axl_proto::google::devtools::build::v1::{
    BuildEvent as ServiceEvent, OrderedBuildEvent, PublishBuildToolEventStreamRequest,
    PublishLifecycleEventRequest, StreamId, build_event::Event, publish_build_event_client,
};
use buildx::bep;
use buildx::fake_bes::{Mode, Server};
use prost::Message;

fn id(i: build_event_id::Id) -> BuildEventId {
    BuildEventId { id: Some(i) }
}

fn started() -> BuildEventId {
    id(build_event_id::Id::Started(Default::default()))
}

fn finished_id() -> BuildEventId {
    id(build_event_id::Id::BuildFinished(Default::default()))
}

fn progress(n: i32) -> BuildEventId {
    id(build_event_id::Id::Progress(build_event_id::ProgressId {
        opaque_count: n,
    }))
}

/// started -> (progress 0, finished); progress 0 -> progress 1; progress 1 closes the stream.
fn stream() -> Vec<BuildEvent> {
    vec![
        BuildEvent {
            id: Some(started()),
            children: vec![progress(0), finished_id()],
            payload: Some(build_event::Payload::Started(Default::default())),
            ..Default::default()
        },
        BuildEvent {
            id: Some(progress(0)),
            children: vec![progress(1)],
            payload: Some(build_event::Payload::Progress(Default::default())),
            ..Default::default()
        },
        BuildEvent {
            id: Some(finished_id()),
            payload: Some(build_event::Payload::Finished(BuildFinished::default())),
            ..Default::default()
        },
        BuildEvent {
            id: Some(progress(1)),
            payload: Some(build_event::Payload::Progress(Default::default())),
            last_message: true,
            ..Default::default()
        },
    ]
}

fn encode(events: &[BuildEvent]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in events {
        e.encode_length_delimited(&mut out).unwrap();
    }
    out
}

#[test]
fn a_consistent_stream_passes() {
    let events = stream();
    assert_eq!(bep::check_graph(&events), Ok(()));
    let decoded = bep::read_events(&encode(&events)).unwrap();
    assert_eq!(decoded, events);
    assert_eq!(bep::histogram(&decoded)["progress"], 2);
}

#[test]
fn the_check_names_what_is_wrong() {
    let ok = stream();

    let mut not_first = ok.clone();
    not_first.swap(0, 1);
    assert!(
        bep::check_graph(&not_first)
            .unwrap_err()
            .contains("not started")
    );

    let mut unannounced = ok.clone();
    unannounced[0].children.retain(|c| *c != progress(0));
    assert!(
        bep::check_graph(&unannounced)
            .unwrap_err()
            .contains("never announced")
    );

    let mut missing = ok.clone();
    missing.remove(1);
    let err = bep::check_graph(&missing).unwrap_err();
    assert!(
        err.contains("never announced") || err.contains("never arrived"),
        "{err}"
    );

    let mut twice = ok.clone();
    twice.insert(2, twice[1].clone());
    assert!(bep::check_graph(&twice).unwrap_err().contains("twice"));

    let mut no_last = ok.clone();
    no_last[3].last_message = false;
    assert!(
        bep::check_graph(&no_last)
            .unwrap_err()
            .contains("last_message")
    );

    let mut early_last = ok.clone();
    early_last[1].last_message = true;
    assert!(
        bep::check_graph(&early_last)
            .unwrap_err()
            .contains("last_message")
    );

    let mut unfinished = ok;
    unfinished[0].children.retain(|c| *c != finished_id());
    unfinished.remove(2);
    assert!(
        bep::check_graph(&unfinished)
            .unwrap_err()
            .contains("no build_finished")
    );
}

#[test]
fn the_exit_code_is_read_from_the_finish() {
    let mut events = stream();
    assert_eq!(bep::exit_code(&events), None);
    events[2].payload = Some(build_event::Payload::Finished(BuildFinished {
        exit_code: Some(axl_proto::build_event_stream::build_finished::ExitCode {
            name: "BUILD_FAILURE".into(),
            code: 1,
        }),
        ..Default::default()
    }));
    assert_eq!(bep::exit_code(&events), Some(("BUILD_FAILURE".into(), 1)));
}

fn serve(server: &Server) -> String {
    serve_with(server, None)
}

fn serve_with(server: &Server, tls: Option<(Vec<u8>, Vec<u8>)>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let s = server.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            let _ = match tls {
                None => s.serve_incoming(incoming).await,
                Some((cert, key)) => s.serve_incoming_tls(incoming, &cert, &key).await,
            };
        });
    });
    format!("http://127.0.0.1:{port}")
}

fn ordered(sequence: i64, event: &BuildEvent) -> PublishBuildToolEventStreamRequest {
    PublishBuildToolEventStreamRequest {
        ordered_build_event: Some(OrderedBuildEvent {
            stream_id: Some(StreamId {
                build_id: "build".into(),
                invocation_id: "invocation".into(),
                component: 1,
            }),
            sequence_number: sequence,
            event: Some(ServiceEvent {
                event: Some(Event::BazelEvent(axl_proto::Any {
                    type_url: "type.googleapis.com/build_event_stream.BuildEvent".into(),
                    value: event.encode_to_vec(),
                })),
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}

async fn send(endpoint: String, events: Vec<BuildEvent>) -> Result<Vec<i64>, tonic::Status> {
    let mut client = publish_build_event_client::PublishBuildEventClient::connect(endpoint)
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
    let mut lifecycle = tonic::Request::new(PublishLifecycleEventRequest::default());
    lifecycle
        .metadata_mut()
        .insert("x-team", "core".parse().unwrap());
    client.publish_lifecycle_event(lifecycle).await?;
    let requests: Vec<_> = events
        .iter()
        .enumerate()
        .map(|(i, e)| ordered(i as i64 + 1, e))
        .collect();
    let mut request = tonic::Request::new(tokio_stream::iter(requests));
    request
        .metadata_mut()
        .insert("x-team", "core".parse().unwrap());
    let mut acks = client
        .publish_build_tool_event_stream(request)
        .await?
        .into_inner();
    let mut seen = Vec::new();
    while let Some(ack) = acks.message().await? {
        seen.push(ack.sequence_number);
    }
    Ok(seen)
}

fn run<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Runtime::new().unwrap().block_on(f)
}

#[test]
fn the_fake_bes_acks_in_order_and_records_everything() {
    let server = Server::new();
    let endpoint = serve(&server);
    let events = stream();
    let acks = run(send(endpoint, events.clone())).unwrap();
    assert_eq!(acks, vec![1, 2, 3, 4]);
    assert_eq!(server.bep_events(), events);
    assert_eq!(bep::read_events(&server.bep_file()).unwrap(), events);
    assert_eq!(server.lifecycle().len(), 1);
    assert_eq!(server.invocation_ids(), vec!["invocation".to_string()]);
    assert!(server.saw_header("x-team", "core"));
    assert!(!server.saw_header("x-team", "other"));
    let c = &server.state.counters;
    use std::sync::atomic::Ordering::Relaxed;
    assert_eq!(c.lifecycle_events.load(Relaxed), 1);
    assert_eq!(c.streams.load(Relaxed), 1);
    assert_eq!(c.stream_events.load(Relaxed), 4);
    assert_eq!(c.acks.load(Relaxed), 4);
    assert!(c.json().contains("\"acks\":4"));
}

#[test]
fn the_fake_bes_can_refuse_and_drop() {
    let server = Server::new();
    server.set_mode(Mode::Refuse);
    let endpoint = serve(&server);
    let err = run(send(endpoint, stream())).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
    assert_eq!(
        server
            .state
            .counters
            .refused
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    let server = Server::new();
    server.set_mode(Mode::DropAfter(2));
    let endpoint = serve(&server);
    let err = run(send(endpoint, stream())).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
    assert_eq!(server.bep_events().len(), 2);
    assert_eq!(
        server
            .state
            .counters
            .dropped
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
fn the_fake_bes_serves_tls() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tls");
    let cert = std::fs::read(dir.join("localhost.crt")).unwrap();
    let key = std::fs::read(dir.join("localhost.key")).unwrap();
    let server = Server::new();
    let endpoint = serve_with(&server, Some((cert.clone(), key)))
        .replace("http://127.0.0.1", "https://localhost");
    let events = stream();
    let acks = run(async move {
        let tls = tonic::transport::ClientTlsConfig::new()
            .ca_certificate(tonic::transport::Certificate::from_pem(cert))
            .domain_name("localhost");
        let channel = tonic::transport::Endpoint::from_shared(endpoint)
            .unwrap()
            .tls_config(tls)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = publish_build_event_client::PublishBuildEventClient::new(channel);
        let requests: Vec<_> = events
            .iter()
            .enumerate()
            .map(|(i, e)| ordered(i as i64 + 1, e))
            .collect();
        let mut acks = client
            .publish_build_tool_event_stream(tonic::Request::new(tokio_stream::iter(requests)))
            .await
            .unwrap()
            .into_inner();
        let mut seen = 0;
        while acks.message().await.unwrap().is_some() {
            seen += 1;
        }
        seen
    });
    assert_eq!(acks, 4);
}

/// A scratch workspace with `//:hello`, `//:second` and a failing `//:broken`, and
/// a script that builds them with `sinks` (Starlark source for a list).
struct Workspace {
    dir: std::path::PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Option<Workspace> {
        let cached = dirs_cache()?.join("buildx/bazel/9.0.1/A-server.jar");
        if !cached.exists() {
            eprintln!("skipped: no Bazel 9.0.1 release in the buildx cache");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("buildx-bep-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("MODULE.bazel"), "module(name = \"fixture\")\n").unwrap();
        std::fs::write(dir.join(".bazelversion"), "9.0.1\n").unwrap();
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        std::fs::write(
            dir.join("BUILD.bazel"),
            r#"genrule(name = "hello", srcs = ["README.md"], outs = ["hello.txt"], cmd = "cp $< $@")
genrule(name = "second", srcs = [":hello"], outs = ["second.txt"], cmd = "cp $< $@")
genrule(name = "broken", outs = ["broken.txt"], cmd = "echo boom >&2; exit 3")
filegroup(name = "everything", srcs = [":hello", ":second"])
"#,
        )
        .unwrap();
        Some(Workspace { dir })
    }

    /// Runs a script building the fixture's targets with `events`; returns (success, stderr).
    fn build(&self, events: &str, env: &[(&str, String)]) -> (bool, String) {
        let script = format!(
            "def _i(ctx: Context):\n    ctx.build([\"//:everything\", \"//:broken\"], events = {events})\nmain = buildx_main(impl = _i, cli_args = {{}})\n"
        );
        std::fs::write(self.dir.join("build.star"), script).unwrap();
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_buildx"));
        command
            .arg("build.star:main")
            .current_dir(&self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"));
        for (k, v) in env {
            command.env(k, v);
        }
        let out = command.output().unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn dirs_cache() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Caches")
    } else {
        home.join(".cache")
    })
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn a_build_streams_the_same_events_to_a_file_and_to_bes() {
    let Some(ws) = Workspace::new("both") else {
        return;
    };
    let server = Server::new();
    let endpoint = serve(&server).replace("http://", "grpc://");
    let file = ws.dir.join("out.bep");
    let (_, stderr) = ws.build(
        &format!(
            "[ctx.events.file({:?}), ctx.events.bes({endpoint:?}, headers = {{\"x-team\": \"core\", \"x-secret\": \"hunter2\"}}, instance = \"main\", results_url = \"https://results.example/inv/\", metadata = {{\"ci\": \"no\"}})]",
            file.display().to_string()
        ),
        &[],
    );
    assert!(!stderr.contains("hunter2"), "{stderr}");
    let from_file = bep::read_events(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(bep::check_graph(&from_file), Ok(()), "{stderr}");
    assert_eq!(
        bep::exit_code(&from_file),
        Some(("BUILD_FAILURE".into(), 1))
    );
    let kinds = bep::histogram(&from_file);
    for expected in [
        "started",
        "unstructured_command_line",
        "structured_command_line",
        "options_parsed",
        "workspace_status",
        "build_metadata",
        "pattern",
        "target_configured",
        "target_completed",
        "build_finished",
        "build_metrics",
        "build_tool_logs",
    ] {
        assert!(kinds.contains_key(expected), "no {expected} in {kinds:?}");
    }
    assert_eq!(server.bep_events(), from_file);
    assert!(server.saw_header("x-team", "core"));
    assert!(server.saw_header("x-secret", "hunter2"));
    assert_eq!(server.lifecycle().len(), 4);
    let ids = server.invocation_ids();
    assert_eq!(ids.len(), 1);
    assert!(
        stderr.contains(&format!(
            "buildx: results: https://results.example/inv/{}",
            ids[0]
        )),
        "{stderr}"
    );
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn a_refusing_bes_server_does_not_fail_the_build() {
    let Some(ws) = Workspace::new("refuse") else {
        return;
    };
    let server = Server::new();
    server.set_mode(Mode::Refuse);
    let endpoint = serve(&server).replace("http://", "grpc://");
    let file = ws.dir.join("out.bep");
    let (ok, stderr) = ws.build(
        &format!(
            "[ctx.events.file({:?}), ctx.events.bes({endpoint:?}, results_url = \"https://results.example/inv/\")]",
            file.display().to_string()
        ),
        &[],
    );
    assert!(ok, "{stderr}");
    assert!(stderr.contains("buildx: events: bes grpc://"), "{stderr}");
    assert!(!stderr.contains("results.example"), "{stderr}");
    let from_file = bep::read_events(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(bep::check_graph(&from_file), Ok(()));
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn a_server_that_drops_the_stream_is_reported_and_the_build_completes() {
    let Some(ws) = Workspace::new("drop") else {
        return;
    };
    let server = Server::new();
    server.set_mode(Mode::DropAfter(0));
    let endpoint = serve(&server).replace("http://", "grpc://");
    let (ok, stderr) = ws.build(&format!("[ctx.events.bes({endpoint:?})]"), &[]);
    assert!(ok, "{stderr}");
    assert!(stderr.contains("buildx: events: bes grpc://"), "{stderr}");
    assert!(stderr.contains("dropped the stream"), "{stderr}");
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn grpcs_verifies_against_the_jvm_trust_store() {
    let Some(ws) = Workspace::new("tls") else {
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tls");
    let server = Server::new();
    let endpoint = serve_with(
        &server,
        Some((
            std::fs::read(dir.join("localhost.crt")).unwrap(),
            std::fs::read(dir.join("localhost.key")).unwrap(),
        )),
    )
    .replace("http://127.0.0.1", "grpcs://localhost");
    let trust = format!(
        "-Djavax.net.ssl.trustStore={} -Djavax.net.ssl.trustStorePassword=changeit",
        dir.join("localhost-trust.p12").display()
    );
    let (untrusted, stderr) = ws.build(&format!("[ctx.events.bes({endpoint:?})]"), &[]);
    assert!(untrusted, "{stderr}");
    assert!(stderr.contains("buildx: events: bes grpcs://"), "{stderr}");
    assert!(server.bep_events().is_empty());
    let (ok, stderr) = ws.build(
        &format!("[ctx.events.bes({endpoint:?})]"),
        &[("JAVA_TOOL_OPTIONS", trust)],
    );
    assert!(ok, "{stderr}");
    assert!(!stderr.contains("buildx: events:"), "{stderr}");
    assert_eq!(bep::check_graph(&server.bep_events()), Ok(()));
}

fn serve_remote(server: &buildx::fake_remote::Server) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let s = server.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            let _ = s.serve_incoming(incoming).await;
        });
    });
    format!("grpc://127.0.0.1:{port}")
}

/// The files the named sets of `events` list: (name, digest, length, uri).
fn named_files(events: &[BuildEvent]) -> Vec<(String, String, i64, String)> {
    let mut out = Vec::new();
    for e in events {
        if let Some(build_event::Payload::NamedSetOfFiles(set)) = &e.payload {
            for f in &set.files {
                let uri = match &f.file {
                    Some(axl_proto::build_event_stream::file::File::Uri(u)) => u.clone(),
                    _ => String::new(),
                };
                out.push((f.name.clone(), f.digest.clone(), f.length, uri));
            }
        }
    }
    out.sort();
    out
}

fn read_bep(path: &std::path::Path) -> Vec<BuildEvent> {
    bep::read_events(&std::fs::read(path).unwrap()).unwrap()
}

impl Workspace {
    /// The execution root's output tree is removed so the next build runs its actions again.
    fn forget_outputs(&self) {
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
        let root = self.dir.join("cache/bazel").join(format!("_bazel_{user}"));
        let real = std::fs::canonicalize(&self.dir).unwrap();
        let base = buildx::output_base::output_base(&root, &real);
        let _ = std::fs::remove_dir_all(base.join("execroot/_main/bazel-out"));
    }
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn default_uris_do_not_depend_on_a_remote_cache() {
    let Some(ws) = Workspace::new("uris") else {
        return;
    };
    let remote = buildx::fake_remote::Server::new();
    let endpoint = serve_remote(&remote);
    let with_cache = ws.dir.join("with-cache.bep");
    let script = |executor: &str, file: &std::path::Path| {
        format!(
            "def _i(ctx: Context):\n    store = ctx.execution.store({endpoint:?})\n    ctx.build([\"//:everything\"], executor = {executor}, events = [ctx.events.file({:?})])\nmain = buildx_main(impl = _i, cli_args = {{}})\n",
            file.display().to_string()
        )
    };
    let run = |source: String| {
        std::fs::write(ws.dir.join("uris.star"), source).unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_buildx"))
            .arg("uris.star:main")
            .current_dir(&ws.dir)
            .env("XDG_CACHE_HOME", ws.dir.join("cache"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(script(
        "ctx.execution.cached(store, ctx.execution.local())",
        &with_cache,
    ));
    ws.forget_outputs();
    let without = ws.dir.join("without-cache.bep");
    run(script("ctx.execution.local()", &without));
    let (a, b) = (
        named_files(&read_bep(&with_cache)),
        named_files(&read_bep(&without)),
    );
    assert_eq!(a.len(), 2, "{a:?}");
    assert_eq!(a, b);
    for (name, digest, length, uri) in &a {
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(uri.ends_with(name), "{uri}");
        assert_eq!(digest.len(), 64, "{name} has no digest");
        assert_eq!(*length, 6, "{name} has the wrong length");
    }
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn artifacts_store_names_files_by_bytestream_uris() {
    let Some(ws) = Workspace::new("artifacts") else {
        return;
    };
    let remote = buildx::fake_remote::Server::new();
    let reapi = serve_remote(&remote);
    let bes = Server::new();
    let endpoint = serve(&bes).replace("http://", "grpc://");
    let file = ws.dir.join("out.bep");
    let (_, stderr) = ws.build(
        &format!(
            "[ctx.events.file({:?}), ctx.events.bes({endpoint:?}, artifacts = ctx.execution.store({reapi:?}, instance = \"main\"))]",
            file.display().to_string()
        ),
        &[],
    );
    assert!(!stderr.contains("artifact upload"), "{stderr}");
    let local = named_files(&read_bep(&file));
    let remote_files = named_files(&bes.bep_events());
    assert_eq!(local.len(), 2);
    assert_eq!(remote_files.len(), 2);
    let authority = reapi.trim_start_matches("grpc://");
    for ((_, digest, length, uri), (_, local_digest, local_length, local_uri)) in
        remote_files.iter().zip(&local)
    {
        assert!(local_uri.starts_with("file:///"));
        assert_eq!(digest, local_digest);
        assert_eq!(length, local_length);
        assert_eq!(
            uri,
            &format!("bytestream://{authority}/main/blobs/{digest}/{length}")
        );
        let blob = buildx::reapi::digest_bytes(b"hello\n");
        assert_eq!(&blob.hash, digest);
        assert_eq!(remote.get(&blob).as_deref(), Some(&b"hello\n"[..]));
    }
    assert_eq!(bep::check_complete(&bes.bep_events()), Ok(()));
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn a_failed_artifact_upload_is_reported_and_does_not_fail_the_build() {
    let Some(ws) = Workspace::new("upload-failure") else {
        return;
    };
    let bes = Server::new();
    let endpoint = serve(&bes).replace("http://", "grpc://");
    let (ok, stderr) = ws.build(
        &format!(
            "[ctx.events.bes({endpoint:?}, artifacts = ctx.execution.store(\"grpc://127.0.0.1:1\"))]"
        ),
        &[],
    );
    assert!(ok, "{stderr}");
    assert!(stderr.contains("artifact uploads failed"), "{stderr}");
    let files = named_files(&bes.bep_events());
    assert_eq!(files.len(), 2);
    assert!(
        files.iter().all(|f| f.3.starts_with("file:///")),
        "{files:?}"
    );
    let in_progress = bes.bep_events().iter().any(|e| match &e.payload {
        Some(build_event::Payload::Progress(p)) => p.stderr.contains("artifact upload failed"),
        _ => false,
    });
    assert!(in_progress, "the failure is not in the event stream");
}

impl Workspace {
    /// Runs `source` as a script (it defines `main`) and returns (success, stderr).
    fn run(&self, source: &str) -> (bool, String) {
        std::fs::write(self.dir.join("script.star"), source).unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_buildx"))
            .arg("script.star:main")
            .current_dir(&self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

/// The options of the canonical command line: (name, value).
fn canonical_options(events: &[BuildEvent]) -> Vec<(String, String)> {
    use axl_proto::command_line::command_line_section::SectionType;
    let mut out = Vec::new();
    for e in events {
        let Some(build_event::Payload::StructuredCommandLine(c)) = &e.payload else {
            continue;
        };
        if c.command_line_label != "canonical" {
            continue;
        }
        for section in &c.sections {
            if let Some(SectionType::OptionList(l)) = &section.section_type {
                out.extend(
                    l.option
                        .iter()
                        .map(|o| (o.option_name.clone(), o.option_value.clone())),
                );
            }
        }
    }
    out
}

fn runner_counts(events: &[BuildEvent]) -> std::collections::BTreeMap<String, i32> {
    for e in events {
        if let Some(build_event::Payload::BuildMetrics(m)) = &e.payload {
            return m
                .action_summary
                .iter()
                .flat_map(|a| a.runner_count.iter())
                .map(|r| (r.name.clone(), r.count))
                .collect();
        }
    }
    Default::default()
}

#[test]
#[ignore = "builds with a real Bazel release: cargo test -p buildx --test bep -- --ignored"]
fn the_stream_states_the_remote_policy_and_metrics_and_never_a_header_value() {
    let Some(ws) = Workspace::new("policy") else {
        return;
    };
    let remote = buildx::fake_remote::Server::new();
    let reapi = serve_remote(&remote);
    let bes = Server::new();
    let endpoint = serve(&bes).replace("http://", "grpc://");
    let script = |run: u32| {
        format!(
            "def _route(spawn):\n    return spawn.executors[\"local\"]\n\ndef _i(ctx: Context):\n    store = ctx.execution.store({reapi:?}, instance = \"main\", headers = {{\"x-store\": \"planted-secret-1\"}})\n    cached = ctx.execution.cached(store, ctx.execution.local())\n    other = ctx.execution.remote({reapi:?}, instance = \"main\", name = \"farm\")\n    routed = ctx.execution.route(_route, executors = {{\"local\": cached, \"farm\": other}})\n    ctx.build([\"//:everything\"], executor = routed, events = [ctx.events.file(\"run{run}.bep\"), ctx.events.bes({endpoint:?}, headers = {{\"x-bes\": \"planted-secret-2\"}}, instance = \"team\", results_url = \"https://results.example/inv/\", metadata = {{\"ci\": \"yes\"}})])\nmain = buildx_main(impl = _i, cli_args = {{}})\n"
        )
    };
    let (ok, stderr) = ws.run(&script(1));
    assert!(ok, "{stderr}");
    ws.forget_outputs();
    let (ok, stderr) = ws.run(&script(2));
    assert!(ok, "{stderr}");

    let first_bytes = std::fs::read(ws.dir.join("run1.bep")).unwrap();
    let second_bytes = std::fs::read(ws.dir.join("run2.bep")).unwrap();
    for needle in [&b"planted-secret-1"[..], &b"planted-secret-2"[..]] {
        for bytes in [&first_bytes, &second_bytes, &bes.bep_file()] {
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle),
                "a header value is in the stream"
            );
        }
    }
    let first = bep::read_events(&first_bytes).unwrap();
    let second = bep::read_events(&second_bytes).unwrap();
    assert_eq!(bep::check_complete(&first), Ok(()));

    let options = canonical_options(&first);
    let has = |n: &str, v: &str| options.iter().any(|(k, x)| k == n && x == v);
    assert!(has("remote_cache", &reapi), "{options:?}");
    assert!(
        has("remote_executor", &reapi),
        "the routed-to executor: {options:?}"
    );
    assert!(has("remote_instance_name", "main"));
    assert!(has("remote_accept_cached", "true"));
    assert!(has("remote_upload_local_results", "true"));
    assert!(has("remote_download_outputs", "all"));
    assert!(has("remote_cache_header", "x-store=REDACTED"));
    assert!(has("bes_header", "x-bes=REDACTED"));
    assert!(has("bes_instance_name", "team"));
    assert!(has("bes_results_url", "https://results.example/inv/"));
    assert!(has("build_metadata", "ci=yes"));
    assert!(options.iter().any(|(k, _)| k == "bes_backend"));
    let started_options = first.iter().find_map(|e| match &e.payload {
        Some(build_event::Payload::OptionsParsed(o)) => Some(o.cmd_line.clone()),
        _ => None,
    });
    assert!(
        started_options
            .unwrap()
            .contains(&format!("--remote_cache={reapi}"))
    );

    let missed = runner_counts(&first);
    assert_eq!(missed.get("local"), Some(&2), "{missed:?}");
    assert_eq!(missed.get("remote cache hit"), None, "{missed:?}");
    let hit = runner_counts(&second);
    assert_eq!(hit.get("remote cache hit"), Some(&2), "{hit:?}");
    assert_eq!(hit.get("local"), None, "a hit is not a local run: {hit:?}");
    // Two cache hits and the workspace status action, which is Bazel's `internal` one.
    assert_eq!(hit.get("total"), Some(&3), "{hit:?}");
    assert_eq!(hit.get("internal"), Some(&1), "{hit:?}");
    let statistics = |events: &[BuildEvent]| {
        events.iter().find_map(|e| match &e.payload {
            Some(build_event::Payload::BuildMetrics(m)) => m
                .action_summary
                .as_ref()
                .and_then(|a| a.action_cache_statistics.clone()),
            _ => None,
        })
    };
    assert_eq!(statistics(&first).unwrap().misses, 3);
    let timing = first
        .iter()
        .find_map(|e| match &e.payload {
            Some(build_event::Payload::BuildMetrics(m)) => m.timing_metrics,
            _ => None,
        })
        .unwrap();
    assert!(timing.wall_time_in_ms > 0 && timing.cpu_time_in_ms > 0);
    assert!(timing.analysis_phase_time_in_ms > 0);
}
