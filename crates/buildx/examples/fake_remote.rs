//! `cargo run -p buildx --example fake_remote -- [--port N] [--latency-ms N]`: an in-memory REAPI
//! server for the remote and cache examples. It prints its endpoint, serves
//! counters as JSON over HTTP on the next port, and logs each `Execute`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};

fn main() {
    let mut port: u16 = 0;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--port" {
            port = args.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        } else if a == "--latency-ms" {
            buildx::fake_remote::set_latency_ms(
                args.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            );
        }
    }
    let server = buildx::fake_remote::Server::new();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let grpc = std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).expect("bind");
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
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(grpc).unwrap();
        tonic::transport::Server::builder()
            .add_service(axl_proto::build::bazel::remote::execution::v2::content_addressable_storage_server::ContentAddressableStorageServer::new(server.clone()))
            .add_service(axl_proto::google::bytestream::byte_stream_server::ByteStreamServer::new(server.clone()))
            .add_service(axl_proto::build::bazel::remote::execution::v2::action_cache_server::ActionCacheServer::new(server.clone()))
            .add_service(axl_proto::build::bazel::remote::execution::v2::capabilities_server::CapabilitiesServer::new(server.clone()))
            .add_service(axl_proto::build::bazel::remote::execution::v2::execution_server::ExecutionServer::new(server))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .expect("serve");
    });
}
