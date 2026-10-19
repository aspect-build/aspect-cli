//! `cargo run -p buildx --example fake_bes -- [--port N] [--refuse] [--drop-after N] [--save file.bep] [--tls cert.pem key.pem]`:
//! an in-memory Build Event Service for the BES examples. It prints its endpoint,
//! serves counters as JSON over HTTP on the next port, and, with `--save`, rewrites
//! the received Build Event Protocol events to `file.bep` (delimited, readable with
//! `bep_dump`) after every stream event.

use buildx::fake_bes::{Mode, Server};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};

fn main() {
    let mut port: u16 = 0;
    let mut mode = Mode::Normal;
    let mut save: Option<String> = None;
    let mut tls: Option<(String, String)> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            "--refuse" => mode = Mode::Refuse,
            "--drop-after" => {
                mode = Mode::DropAfter(args.next().and_then(|p| p.parse().ok()).unwrap_or(0))
            }
            "--save" => save = args.next(),
            "--tls" => {
                let cert = args.next().expect("--tls <cert.pem> <key.pem>");
                let key = args.next().expect("--tls <cert.pem> <key.pem>");
                tls = Some((cert, key));
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let server = Server::new();
    server.set_mode(mode);
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let grpc = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).expect("bind");
    let grpc_port = grpc.local_addr().unwrap().port();
    grpc.set_nonblocking(true).unwrap();
    let stats = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).expect("bind stats");
    let stats_port = stats.local_addr().unwrap().port();
    println!("grpc://127.0.0.1:{grpc_port} stats=http://127.0.0.1:{stats_port}/");
    let _ = std::io::stdout().flush();
    let state = server.state.clone();
    std::thread::spawn(move || {
        for stream in stats.incoming().flatten() {
            let mut stream = stream;
            let mut buffer = [0u8; 512];
            let _ = stream.read(&mut buffer);
            let body = state.counters.json();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    if let Some(path) = save {
        let server = server.clone();
        std::thread::spawn(move || {
            let mut last = 0usize;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                let events = server.bep_events().len();
                if events != last {
                    last = events;
                    let _ = std::fs::write(&path, server.bep_file());
                }
            }
        });
    }
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(grpc).unwrap();
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        match tls {
            None => server.serve_incoming(incoming).await.expect("serve"),
            Some((cert, key)) => server
                .serve_incoming_tls(
                    incoming,
                    &std::fs::read(cert).expect("certificate"),
                    &std::fs::read(key).expect("key"),
                )
                .await
                .expect("serve"),
        }
    });
}
