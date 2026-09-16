//! Real processes, synthetic credentials, and a loopback issuer. No keychain,
//! browser login, or live identity provider is involved.
mod common;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn jwt(exp: u64) -> String {
    let payload = json!({"exp": exp, "tenantId": "test-tenant"});
    format!(
        "e30.{}.synthetic",
        URL_SAFE_NO_PAD.encode(payload.to_string())
    )
}

struct Issuer {
    url: String,
    bearer: String,
    grants: Arc<AtomicUsize>,
    first_grant: mpsc::Receiver<()>,
    release: Option<mpsc::Sender<()>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Issuer {
    fn start(status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let bearer = jwt(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86400);
        let grants = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (first_tx, first_grant) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let worker_grants = grants.clone();
        let worker_stop = stop.clone();
        let worker_bearer = bearer.clone();
        let thread = std::thread::spawn(move || {
            // The first grant blocks here while additional real clients start.
            // Requests arriving meanwhile queue on the listener, exposing an
            // uncoordinated client even though responses are served serially.
            while !worker_stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("accept: {e}"),
                };
                let request = read_request(&mut stream);
                if request.starts_with("GET ") {
                    respond(&mut stream, 404, json!({}));
                    continue;
                }
                assert!(request.starts_with("POST /oauth/token "), "{request}");
                assert!(request.contains("refresh_token=synthetic-old"), "{request}");
                let number = worker_grants.fetch_add(1, Ordering::SeqCst);
                if number == 0 {
                    first_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                }
                let status = if status == 200 && number > 0 {
                    401
                } else {
                    status
                };
                let body = if status == 200 {
                    json!({"access_token": worker_bearer, "id_token": worker_bearer,
                           "refresh_token": "synthetic-new", "token_type": "Bearer", "expires_in": 86400})
                } else {
                    json!({"errors": ["synthetic rejection; never echo this response"]})
                };
                respond(&mut stream, status, body);
            }
        });
        Self {
            url,
            bearer,
            grants,
            first_grant,
            release: Some(release),
            stop,
            thread: Some(thread),
        }
    }

    fn wait_for_grant(&self) {
        self.first_grant
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    }

    fn release(&mut self) {
        self.release.take().unwrap().send(()).unwrap();
    }
}

impl Drop for Issuer {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map(|s| s.trim().parse().unwrap())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: Value) {
    let body = body.to_string();
    write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

struct Fixture(tempfile::TempDir);
impl Fixture {
    fn new(issuer: &Issuer) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".aspect");
        std::fs::create_dir(&config).unwrap();
        std::fs::write(dir.path().join("MODULE.bazel"), "").unwrap();
        std::fs::write(
            config.join("config.json"),
            json!({"deployments": [
                {"name": "refresh-fixture", "hosts": ["refresh-fixture.invalid"]}
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            config.join("config.axl"),
            r#"
def _impl(ctx):
    creds = ctx.aspect.auth.credentials(deployment = "refresh-fixture")
    if not creds:
        fail("missing credential")
    print("AXL_REFRESH_OK")

refresh_probe = task(implementation = _impl, summary = "Exercise credential refresh")

def config(ctx):
    ctx.tasks.add(refresh_probe)
"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("credentials.json"), json!({"refresh-test": {"refresh-fixture": {
            "access_token": jwt(1), "refresh_token": "synthetic-old", "email": "test@example.invalid",
            "name": "Synthetic", "tenant_id": "test-tenant", "auth_domain": issuer.url,
            "auth_client_id": "synthetic-client", "prefer_id_token": true
        }}}).to_string()).unwrap();
        Self(dir)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(common::aspect_cli());
        // Do not inherit an API token that could bypass the synthetic store.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("ASPECT_") {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .current_dir(self.0.path())
            .env("ASPECT_WORKSPACE", self.0.path())
            .env(
                "ASPECT_CREDENTIALS_FILE",
                self.0.path().join("credentials.json"),
            )
            .env("ASPECT_AUTH_PROFILE", "refresh-test")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn helper(&self) -> Child {
        let mut child = self.command(&["get"]).spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"uri":"https://refresh-fixture.invalid"}"#)
            .unwrap();
        child
    }

    fn stored(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.0.path().join("credentials.json")).unwrap())
            .unwrap()
    }
}

fn successful(child: Child) -> Output {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn helpers_and_axl_share_one_refresh_across_processes() {
    let mut issuer = Issuer::start(200);
    let fixture = Fixture::new(&issuer);
    let first = fixture.helper();
    issuer.wait_for_grant();
    let second = fixture.helper();
    let axl = fixture.command(&["refresh-probe"]).spawn().unwrap();
    // Keep the first request pending long enough for both competing processes
    // to reach the locked transaction. Without coordination they submit grants
    // with the old token; the issuer rejects those queued requests.
    std::thread::sleep(Duration::from_millis(500));
    issuer.release();
    for child in [first, second] {
        let output = successful(child);
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            response["headers"]["Authorization"][0],
            format!("Bearer {}", issuer.bearer)
        );
    }
    let output = successful(axl);
    assert!(String::from_utf8_lossy(&output.stderr).contains("AXL_REFRESH_OK"));
    assert_eq!(issuer.grants.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.stored()["refresh-test"]["refresh-fixture"]["refresh_token"],
        "synthetic-new"
    );
    successful(fixture.helper());
    assert_eq!(issuer.grants.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_refresh_preserves_credentials_and_distinguishes_rejection() {
    for (status, message) in [
        (401, "rejected the refresh token"),
        (503, "retry the command"),
    ] {
        let mut issuer = Issuer::start(status);
        let fixture = Fixture::new(&issuer);
        let original = fixture.stored();
        let child = fixture.helper();
        issuer.wait_for_grant();
        issuer.release();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(message), "{stderr}");
        assert!(!stderr.contains("never echo this response"), "{stderr}");
        assert_eq!(fixture.stored(), original);
    }
}

#[test]
fn logout_waits_for_refresh_and_is_not_undone_by_its_write() {
    let mut issuer = Issuer::start(200);
    let fixture = Fixture::new(&issuer);
    let helper = fixture.helper();
    issuer.wait_for_grant();
    let logout = fixture
        .command(&["auth", "logout", "--all"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    issuer.release();
    successful(helper);
    successful(logout);
    assert_eq!(fixture.stored(), json!({}));
}
