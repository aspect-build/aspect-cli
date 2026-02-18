use std::fs::File;
use std::io::{BufWriter, Write};
use std::thread::{self, JoinHandle};

use axl_proto::build_event_stream::BuildEvent;
use prost::Message;

use super::broadcaster::Subscriber;

pub struct FileBuildEventSink;

impl FileBuildEventSink {
    /// Spawns a thread that writes BES events to a file in length-delimited
    /// binary proto format (same as `--build_event_binary_file`).
    pub fn spawn(recv: Subscriber<BuildEvent>, path: String) -> JoinHandle<()> {
        thread::spawn(move || {
            let file = File::create(&path).expect("failed to create BES output file");
            let mut file = BufWriter::new(file);
            loop {
                match recv.recv() {
                    Ok(event) => {
                        if let Err(e) = file.write_all(&event.encode_length_delimited_to_vec()) {
                            eprintln!("FileBuildEventSink: failed to write event: {}", e);
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    }
}
