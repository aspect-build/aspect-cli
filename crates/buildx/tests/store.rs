//! The store and the output round trip against the in-memory REAPI server.

use axl_proto::build::bazel::remote::execution::v2::ActionResult;
use buildx::fake_remote::Server;
use buildx::reapi::digest_bytes;
use buildx::remote::{collect_outputs, materialize};
use buildx::store::Store;
use std::sync::Arc;

fn serve() -> (String, Server) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let server = Server::new();
    let s = server.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            let _ = s.serve_incoming(incoming).await;
        });
    });
    (format!("grpc://127.0.0.1:{port}"), server)
}

#[test]
fn blobs_round_trip_small_and_large() {
    let (endpoint, server) = serve();
    let store = Store::connect(&endpoint, "main", Vec::new()).unwrap();
    let small = b"hello".to_vec();
    let large = vec![7u8; 5 * 1024 * 1024];
    let (ds, dl) = (digest_bytes(&small), digest_bytes(&large));
    assert_eq!(
        store.find_missing(&[ds.clone(), dl.clone()]).unwrap().len(),
        2
    );
    store
        .upload(vec![
            (ds.clone(), Arc::new(small.clone())),
            (dl.clone(), Arc::new(large.clone())),
        ])
        .unwrap();
    assert!(
        store
            .find_missing(&[ds.clone(), dl.clone()])
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.download(&ds).unwrap(), small);
    assert_eq!(store.download(&dl).unwrap(), large);
    let counters = &server.state.counters;
    assert_eq!(
        counters
            .bytestream_writes
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        counters
            .bytestream_reads
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
fn action_results_hit_and_miss() {
    let (endpoint, _server) = serve();
    let store = Store::connect(&endpoint, "", Vec::new()).unwrap();
    let action = digest_bytes(b"action");
    assert!(store.get_action_result(&action).unwrap().is_none());
    let result = ActionResult {
        exit_code: 0,
        ..Default::default()
    };
    store.update_action_result(&action, &result).unwrap();
    assert_eq!(store.get_action_result(&action).unwrap(), Some(result));
}

#[test]
fn outputs_round_trip_through_the_store() {
    let (endpoint, _server) = serve();
    let store = Store::connect(&endpoint, "", Vec::new()).unwrap();
    let dir = std::env::temp_dir().join(format!("buildx-outputs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("out/tree/sub")).unwrap();
    std::fs::write(dir.join("out/file.txt"), "file").unwrap();
    std::fs::write(dir.join("out/tree/a"), "a").unwrap();
    std::fs::write(dir.join("out/tree/sub/b"), "b").unwrap();
    let paths = vec![
        "out/file.txt".to_string(),
        "out/tree".to_string(),
        "out/missing".to_string(),
    ];
    let (result, blobs) = collect_outputs(&dir, &paths).unwrap();
    assert_eq!(result.output_files.len(), 1);
    assert_eq!(result.output_directories.len(), 1);
    store.upload(blobs).unwrap();
    std::fs::remove_dir_all(dir.join("out")).unwrap();
    materialize(&store, &dir, &result).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("out/file.txt")).unwrap(),
        "file"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("out/tree/sub/b")).unwrap(),
        "b"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
