//! A Bazel spawn as the messages of `build.bazel.remote.execution.v2`.
//!
//! `Command` carries the arguments, the environment sorted by name, the
//! output paths and the platform properties; the input root is a `Directory`
//! tree whose files carry the digests Bazel already recorded; `Action` ties
//! them together by digest. Digests use SHA-256 over the messages' wire
//! encoding, as Bazel's own remote execution does.

#![allow(deprecated)]

use axl_proto::build::bazel::remote::execution::v2::{
    Action, Command, Digest, Directory, DirectoryNode, FileNode, Platform, SymlinkNode, command,
    platform,
};
use prost::Message;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// What of a `Spawn` the `Command` and `Action` are made from.
#[derive(Clone, Debug, Default)]
pub struct SpawnData {
    pub mnemonic: String,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    /// The spawn's `execution_info` tags.
    pub execution_info: Vec<(String, String)>,
    /// The merged platform and action exec properties.
    pub platform_properties: Vec<(String, String)>,
    /// Execution-root relative paths of the declared outputs.
    pub outputs: Vec<String>,
    pub working_directory: String,
    pub timeout_seconds: Option<i64>,
}

/// One entry of the input root.
#[derive(Clone, Debug)]
pub enum Entry {
    File {
        digest: String,
        size: i64,
        executable: bool,
    },
    Symlink {
        target: String,
    },
    /// A directory with no files in it.
    Directory,
}

#[derive(Clone, Debug)]
pub struct Input {
    /// Execution-root relative path.
    pub path: String,
    pub entry: Entry,
    /// The bytes of an input that is not a file on disk (a parameter file).
    pub bytes: Option<std::sync::Arc<Vec<u8>>>,
    /// Where the bytes are, relative to the execution root, when that is not `path`
    /// (a runfiles entry).
    pub source: Option<String>,
}

/// The input root: its digest, the root `Directory` and every other
/// `Directory` it names, as a REAPI `Tree` holds them.
#[derive(Clone, Debug)]
pub struct InputTree {
    pub root_digest: Digest,
    pub root: Directory,
    pub children: Vec<Directory>,
    pub files: usize,
    pub bytes: i64,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub inputs: Vec<Input>,
    pub command: Command,
    pub command_digest: Digest,
    pub input_tree: InputTree,
    pub action: Action,
    pub action_digest: Digest,
}

/// The input root as a REAPI `Tree`.
pub fn tree_message(tree: &InputTree) -> axl_proto::build::bazel::remote::execution::v2::Tree {
    axl_proto::build::bazel::remote::execution::v2::Tree {
        root: Some(tree.root.clone()),
        children: tree.children.clone(),
    }
}

pub fn digest_of(message: &impl Message) -> Digest {
    digest_bytes(&message.encode_to_vec())
}

pub fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest {
        hash: hex(&Sha256::digest(bytes)),
        size_bytes: bytes.len() as i64,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn platform(properties: &[(String, String)]) -> Platform {
    let mut properties = properties.to_vec();
    properties.sort();
    Platform {
        properties: properties
            .into_iter()
            .map(|(name, value)| platform::Property { name, value })
            .collect(),
    }
}

/// The `Command`: environment and platform properties sorted by name, the
/// outputs as `output_paths`, sorted.
pub fn command(data: &SpawnData) -> Command {
    let mut environment = data.environment.clone();
    environment.sort();
    let mut outputs = data.outputs.clone();
    outputs.sort();
    Command {
        arguments: data.arguments.clone(),
        environment_variables: environment
            .into_iter()
            .map(|(name, value)| command::EnvironmentVariable { name, value })
            .collect(),
        output_paths: outputs,
        platform: Some(platform(&data.platform_properties)),
        working_directory: data.working_directory.clone(),
        ..Default::default()
    }
}

#[derive(Default)]
struct Node {
    files: BTreeMap<String, (String, i64, bool)>,
    symlinks: BTreeMap<String, String>,
    dirs: BTreeMap<String, Node>,
}

/// The input root of `inputs`, as `Directory` messages bottom-up.
pub fn input_tree(inputs: &[Input]) -> InputTree {
    let mut root = Node::default();
    let mut files = 0;
    let mut bytes = 0;
    for input in inputs {
        let parts: Vec<&str> = input.path.split('/').filter(|p| !p.is_empty()).collect();
        let Some((leaf, parents)) = parts.split_last() else {
            continue;
        };
        let mut node = &mut root;
        for p in parents {
            node = node.dirs.entry((*p).to_string()).or_default();
        }
        match &input.entry {
            Entry::File {
                digest,
                size,
                executable,
            } => {
                files += 1;
                bytes += size;
                node.files
                    .insert((*leaf).to_string(), (digest.clone(), *size, *executable));
            }
            Entry::Symlink { target } => {
                node.symlinks.insert((*leaf).to_string(), target.clone());
            }
            Entry::Directory => {
                node.dirs.entry((*leaf).to_string()).or_default();
            }
        }
    }
    let mut hashed = Vec::new();
    let (root_message, root_digest) = directory(&root, &mut hashed);
    hashed.sort_by(|a, b| a.0.cmp(&b.0));
    hashed.dedup_by(|a, b| a.0 == b.0);
    let children = hashed.into_iter().map(|(_, d)| d).collect();
    InputTree {
        root_digest,
        root: root_message,
        children,
        files,
        bytes,
    }
}

fn directory(node: &Node, all: &mut Vec<(String, Directory)>) -> (Directory, Digest) {
    let mut message = Directory::default();
    for (name, (hash, size, executable)) in &node.files {
        message.files.push(FileNode {
            name: name.clone(),
            digest: Some(Digest {
                hash: hash.clone(),
                size_bytes: *size,
            }),
            is_executable: *executable,
            ..Default::default()
        });
    }
    for (name, child) in &node.dirs {
        let (d, digest) = directory(child, all);
        all.push((digest.hash.clone(), d));
        message.directories.push(DirectoryNode {
            name: name.clone(),
            digest: Some(digest),
        });
    }
    for (name, target) in &node.symlinks {
        message.symlinks.push(SymlinkNode {
            name: name.clone(),
            target: target.clone(),
            ..Default::default()
        });
    }
    let digest = digest_of(&message);
    (message, digest)
}

/// The whole request: `Command`, input root and the `Action` over them.
pub fn request(data: &SpawnData, inputs: &[Input]) -> Request {
    let command = command(data);
    let command_digest = digest_of(&command);
    let input_tree = input_tree(inputs);
    let no_cache = data
        .execution_info
        .iter()
        .any(|(k, _)| k == "no-cache" || k == "no-remote-cache" || k == "local");
    let action = Action {
        command_digest: Some(command_digest.clone()),
        input_root_digest: Some(input_tree.root_digest.clone()),
        timeout: data
            .timeout_seconds
            .map(|seconds| axl_proto::Duration { seconds, nanos: 0 }),
        do_not_cache: no_cache,
        platform: command.platform.clone(),
        ..Default::default()
    };
    let action_digest = digest_of(&action);
    Request {
        inputs: inputs.to_vec(),
        command,
        command_digest,
        input_tree,
        action,
        action_digest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, hash: &str, size: i64) -> Input {
        Input {
            path: path.into(),
            entry: Entry::File {
                digest: hash.into(),
                size,
                executable: false,
            },
            bytes: None,
            source: None,
        }
    }

    #[test]
    fn environment_and_outputs_are_sorted() {
        let data = SpawnData {
            arguments: vec!["/bin/sh".into(), "-c".into(), "x".into()],
            environment: vec![("B".into(), "2".into()), ("A".into(), "1".into())],
            outputs: vec!["out/b".into(), "out/a".into()],
            platform_properties: vec![("z".into(), "1".into()), ("a".into(), "2".into())],
            ..Default::default()
        };
        let c = command(&data);
        assert_eq!(c.environment_variables[0].name, "A");
        assert_eq!(c.output_paths, ["out/a", "out/b"]);
        assert_eq!(c.platform.unwrap().properties[0].name, "a");
    }

    #[test]
    fn tree_is_nested_and_order_independent() {
        let a = [
            file("pkg/a.txt", "aa", 1),
            file("pkg/sub/b.txt", "bb", 2),
            file("top", "cc", 3),
        ];
        let b = [a[2].clone(), a[1].clone(), a[0].clone()];
        let (x, y) = (input_tree(&a), input_tree(&b));
        assert_eq!(x.root_digest, y.root_digest);
        assert_eq!(x.files, 3);
        assert_eq!(x.bytes, 6);
        assert_eq!(x.root.files.len(), 1);
        assert_eq!(x.root.directories.len(), 1);
        assert_eq!(x.children.len(), 2);
    }

    #[test]
    fn action_names_command_and_root_by_digest() {
        let data = SpawnData::default();
        let r = request(&data, &[file("a", "aa", 1)]);
        assert_eq!(r.action.command_digest, Some(r.command_digest.clone()));
        assert_eq!(
            r.action.input_root_digest,
            Some(r.input_tree.root_digest.clone())
        );
        let again = request(&data, &[file("a", "aa", 1)]);
        assert_eq!(r.action_digest, again.action_digest);
    }
}
