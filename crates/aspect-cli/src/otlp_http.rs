//! HTTP client for the OTLP `http/protobuf` exporters.
//!
//! `opentelemetry-otlp` is built without a bundled client, so its exporters
//! use the workspace's reqwest (rustls with ring, native roots), as every
//! other HTTP call in the binary does.
//!
//! The client is reqwest's blocking one because exports run on the SDK's
//! background threads, which have no tokio reactor. It must never run inside
//! a tokio `block_on`; `trace::install_late_exporters` replays the startup
//! buffer from a thread that only enters the runtime for that reason.

use std::io::Read as _;
use std::time::Duration;

use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};

/// Per-request timeout: the OTLP exporter default.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Largest response body read. OTLP responses are small status messages.
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct OtlpHttpClient(reqwest::blocking::Client);

impl OtlpHttpClient {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self(
            reqwest::blocking::Client::builder()
                .timeout(TIMEOUT)
                .build()?,
        ))
    }
}

#[async_trait::async_trait]
impl HttpClient for OtlpHttpClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let mut response = self.0.execute(request.try_into()?)?;
        let status = response.status();
        let headers = std::mem::take(response.headers_mut());
        let mut body = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut body)?;
        if body.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(format!("OTLP response body exceeds {MAX_RESPONSE_BYTES} bytes").into());
        }
        let mut out = Response::builder().status(status).body(Bytes::from(body))?;
        *out.headers_mut() = headers;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::TcpListener;

    /// Serves one request on a local port, returning what it received.
    fn serve_once(response: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/traces", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            (&stream).write_all(response.as_bytes()).unwrap();
            head + &String::from_utf8(body).unwrap()
        });
        (url, server)
    }

    #[test]
    fn sends_request_and_returns_status_headers_and_body() {
        let (url, server) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-Reply: yes\r\nConnection: close\r\n\r\nok",
        );
        let request = Request::post(&url)
            .header("x-honeycomb-team", "key")
            .body(Bytes::from_static(b"payload"))
            .unwrap();

        let response =
            futures::executor::block_on(OtlpHttpClient::new().unwrap().send_bytes(request))
                .unwrap();

        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["x-reply"], "yes");
        assert_eq!(response.body().as_ref(), b"ok");
        let received = server.join().unwrap();
        assert!(
            received.starts_with("POST /v1/traces HTTP/1.1\r\n"),
            "{received}"
        );
        assert!(received.contains("x-honeycomb-team: key\r\n"), "{received}");
        assert!(received.ends_with("payload"), "{received}");
    }

    #[test]
    fn passes_error_statuses_through() {
        let (url, server) = serve_once(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let request = Request::post(&url).body(Bytes::new()).unwrap();
        let response =
            futures::executor::block_on(OtlpHttpClient::new().unwrap().send_bytes(request))
                .unwrap();
        assert_eq!(response.status(), 503);
        server.join().unwrap();
    }
}
