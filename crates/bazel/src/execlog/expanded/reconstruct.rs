//! Rebuilding each spawn's `SpawnExec` from a compact log, as Bazel's
//! `SpawnLogReconstructor` does: a port of Bazel 8's, which also honours the
//! `legacy_external_runfiles` and `sibling_repository_layout` fields later
//! Bazels no longer set.
//!
//! A compact log names every file, directory, runfiles tree and nested set
//! once, by id, and each spawn refers to them; the expanded format spells each
//! spawn's inputs and outputs out in full. Ids are consecutive from 1, so the
//! entries are kept in a `Vec` indexed by id.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io;

use axl_proto::tools::protos::exec_log_entry::{self as entry, output};
use axl_proto::tools::protos::{ExecLogEntry, File, SpawnExec};
use prost::Message;

/// The `_repo_mapping` manifest's path under a runfiles tree.
const REPO_MAPPING_MANIFEST: &str = "_repo_mapping";

/// A reconstructed input: a file (or unresolved symlink, a `File` with a
/// `symlink_target_path`), or a directory of files.
#[derive(Clone)]
enum Input {
    File(File),
    Directory { path: String, files: Vec<File> },
}

impl Input {
    fn path(&self) -> &str {
        match self {
            Input::File(file) => &file.path,
            Input::Directory { path, .. } => path,
        }
    }

    fn files(&self) -> &[File] {
        match self {
            Input::File(file) => std::slice::from_ref(file),
            Input::Directory { files, .. } => files,
        }
    }
}

enum Stored {
    Input(Input),
    InputSet(entry::InputSet),
    SymlinkEntrySet(entry::SymlinkEntrySet),
}

/// Feeds on a compact log's entries in order and yields each spawn's
/// `SpawnExec`.
pub struct Reconstructor {
    /// Index 0 is unused: ids start at 1.
    entries: Vec<Option<Stored>>,
    hash_function_name: String,
    workspace_runfiles_directory: String,
    sibling_repository_layout: bool,
}

impl Default for Reconstructor {
    fn default() -> Self {
        Self {
            entries: vec![None],
            hash_function_name: String::new(),
            workspace_runfiles_directory: String::new(),
            sibling_repository_layout: false,
        }
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Reconstructor {
    /// Take in one entry; a spawn comes back as its `SpawnExec`. Symlink
    /// actions have no place in the expanded format and yield nothing.
    pub fn push(&mut self, entry: ExecLogEntry) -> io::Result<Option<SpawnExec>> {
        let id = entry.id;
        let Some(kind) = entry.r#type else {
            return Err(invalid(format!("entry {id} has no type")));
        };
        match kind {
            entry::Type::Invocation(invocation) => {
                self.hash_function_name = invocation.hash_function_name;
                self.workspace_runfiles_directory = invocation.workspace_runfiles_directory;
                self.sibling_repository_layout = invocation.sibling_repository_layout;
            }
            entry::Type::File(file) => {
                let file = self.file(None, file);
                self.put(id, Stored::Input(Input::File(file)))?;
            }
            entry::Type::Directory(dir) => {
                let files = dir
                    .files
                    .iter()
                    .map(|f| self.file(Some(&dir.path), f.clone()))
                    .collect();
                self.put(
                    id,
                    Stored::Input(Input::Directory {
                        path: dir.path,
                        files,
                    }),
                )?;
            }
            entry::Type::UnresolvedSymlink(link) => {
                let file = File {
                    path: link.path,
                    symlink_target_path: link.target_path,
                    ..File::default()
                };
                self.put(id, Stored::Input(Input::File(file)))?;
            }
            entry::Type::RunfilesTree(tree) => {
                let dir = self.runfiles_dir(&tree)?;
                self.put(id, Stored::Input(dir))?;
            }
            entry::Type::InputSet(set) => self.put(id, Stored::InputSet(set))?,
            entry::Type::SymlinkEntrySet(set) => self.put(id, Stored::SymlinkEntrySet(set))?,
            entry::Type::Spawn(spawn) => return self.spawn(spawn).map(Some),
            entry::Type::SymlinkAction(_) => {}
        }
        Ok(None)
    }

    fn put(&mut self, id: u32, stored: Stored) -> io::Result<()> {
        if id == 0 {
            // Never referenced, so not kept.
            return Ok(());
        }
        if id as usize != self.entries.len() {
            return Err(invalid(format!(
                "ids must be consecutive, got {id} after {}",
                self.entries.len()
            )));
        }
        self.entries.push(Some(stored));
        Ok(())
    }

    fn input(&self, id: u32) -> io::Result<&Input> {
        match self.entries.get(id as usize) {
            Some(Some(Stored::Input(input))) => Ok(input),
            Some(Some(_)) => Err(invalid(format!("entry {id} is not an input"))),
            _ => Err(invalid(format!("referenced input {id} is missing"))),
        }
    }

    fn input_set(&self, id: u32) -> io::Result<&entry::InputSet> {
        match self.entries.get(id as usize) {
            Some(Some(Stored::InputSet(set))) => Ok(set),
            Some(Some(_)) => Err(invalid(format!("entry {id} is not an input set"))),
            _ => Err(invalid(format!("referenced input set {id} is missing"))),
        }
    }

    fn symlink_entry_set(&self, id: u32) -> io::Result<&entry::SymlinkEntrySet> {
        match self.entries.get(id as usize) {
            Some(Some(Stored::SymlinkEntrySet(set))) => Ok(set),
            Some(Some(_)) => Err(invalid(format!(
                "entry {id} is not a set of symlink entries"
            ))),
            _ => Err(invalid(format!(
                "referenced set of symlink entries {id} is missing"
            ))),
        }
    }

    /// A compact file as an expanded one, under `parent` when it is a
    /// directory's, its digest naming the log's hash function.
    fn file(&self, parent: Option<&str>, file: entry::File) -> File {
        File {
            path: match parent {
                Some(dir) => format!("{dir}/{}", file.path),
                None => file.path,
            },
            digest: file.digest.map(|mut digest| {
                digest.hash_function_name = self.hash_function_name.clone();
                digest
            }),
            ..File::default()
        }
    }

    fn spawn(&self, spawn: entry::Spawn) -> io::Result<SpawnExec> {
        let mut inputs: BTreeMap<String, File> = BTreeMap::new();
        self.visit_input_set(
            spawn.input_set_id,
            &mut |file| {
                inputs.insert(file.path.clone(), file.clone());
            },
            &mut |_| {},
        )?;
        let mut tools: HashSet<String> = HashSet::new();
        self.visit_input_set(
            spawn.tool_set_id,
            &mut |file| {
                tools.insert(file.path.clone());
            },
            &mut |_| {},
        )?;
        let inputs = inputs
            .into_iter()
            .map(|(path, mut file)| {
                if tools.contains(&path) {
                    file.is_tool = true;
                }
                file
            })
            .collect();

        let mut listed: BTreeSet<String> = BTreeSet::new();
        let mut actual = Vec::new();
        for out in &spawn.outputs {
            match &out.r#type {
                Some(output::Type::OutputId(id)) => {
                    let input = self.input(*id)?;
                    listed.insert(input.path().to_string());
                    actual.extend(input.files().iter().cloned());
                }
                Some(output::Type::InvalidOutputPath(path)) => {
                    listed.insert(path.clone());
                }
                None => return Err(invalid("an output has no type".to_string())),
            }
        }

        Ok(SpawnExec {
            command_args: spawn.args,
            environment_variables: spawn.env_vars,
            platform: spawn.platform,
            inputs,
            listed_outputs: listed.into_iter().collect(),
            remotable: spawn.remotable,
            cacheable: spawn.cacheable,
            timeout_millis: spawn.timeout_millis,
            mnemonic: spawn.mnemonic,
            actual_outputs: actual,
            runner: spawn.runner,
            cache_hit: spawn.cache_hit,
            status: spawn.status,
            exit_code: spawn.exit_code,
            remote_cacheable: spawn.remote_cacheable,
            target_label: spawn.target_label,
            digest: spawn.digest.map(|mut digest| {
                digest.hash_function_name = self.hash_function_name.clone();
                digest
            }),
            metrics: spawn.metrics,
        })
    }

    /// Visit the inputs of the nested set `id` in post-order (transitive sets
    /// before direct entries, each left to right), each input once: `on_file`
    /// with each file (a directory's one by one), `on_input` with each input.
    /// Set and input ids share one id space, so one visited map serves both.
    fn visit_input_set(
        &self,
        id: u32,
        on_file: &mut dyn FnMut(&File),
        on_input: &mut dyn FnMut(&Input),
    ) -> io::Result<()> {
        if id == 0 {
            return Ok(());
        }
        let mut stack = vec![id];
        let mut visits: HashMap<u32, u8> = HashMap::new();
        while let Some(current) = stack.pop() {
            match visits.get(&current).copied() {
                None => {
                    visits.insert(current, 0);
                    stack.push(current);
                    for transitive in self.input_set(current)?.transitive_set_ids.iter().rev() {
                        if !visits.contains_key(transitive) {
                            stack.push(*transitive);
                        }
                    }
                }
                Some(0) => {
                    visits.insert(current, 1);
                    for input_id in &self.input_set(current)?.input_ids {
                        if visits.insert(*input_id, 1).is_some() {
                            continue;
                        }
                        let input = self.input(*input_id)?;
                        on_input(input);
                        input.files().iter().for_each(&mut *on_file);
                    }
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Visit a runfiles tree's symlinks (or root symlinks) in post-order, as
    /// `visit_input_set` does, with each entry's runfiles-relative path and
    /// the files it places there. Entries within one set have distinct paths,
    /// so their order (lost to the generated map) is taken as sorted.
    fn visit_symlink_entries(
        &self,
        tree: &entry::RunfilesTree,
        root_symlinks: bool,
        on_entry: &mut dyn FnMut(&str, Vec<File>),
    ) -> io::Result<()> {
        let id = if root_symlinks {
            tree.root_symlinks_id
        } else {
            tree.symlinks_id
        };
        if id == 0 {
            return Ok(());
        }
        let workspace = &self.workspace_runfiles_directory;
        let mut stack = vec![id];
        let mut visits: HashMap<u32, u8> = HashMap::new();
        while let Some(current) = stack.pop() {
            match visits.get(&current).copied() {
                None => {
                    visits.insert(current, 0);
                    stack.push(current);
                    for transitive in self
                        .symlink_entry_set(current)?
                        .transitive_set_ids
                        .iter()
                        .rev()
                    {
                        if !visits.contains_key(transitive) {
                            stack.push(*transitive);
                        }
                    }
                }
                Some(0) => {
                    visits.insert(current, 1);
                    let set = self.symlink_entry_set(current)?;
                    let mut direct: Vec<(&String, &u32)> = set.direct_entries.iter().collect();
                    direct.sort();
                    for (key, target) in direct {
                        let relative = if root_symlinks {
                            key.clone()
                        } else if let Some(rest) = key.strip_prefix("../") {
                            rest.to_string()
                        } else {
                            format!("{workspace}/{key}")
                        };
                        let path = format!("{}/{relative}", tree.path);
                        on_entry(&relative, self.symlink_target(&path, *target)?);
                        if tree.legacy_external_runfiles
                            && !root_symlinks
                            && !relative.starts_with(&format!("{workspace}/"))
                        {
                            let legacy = format!("{workspace}/external/{relative}");
                            let path = format!("{}/{legacy}", tree.path);
                            on_entry(&legacy, self.symlink_target(&path, *target)?);
                        }
                    }
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// The files a runfiles symlink at `path` to input `target` places: an
    /// empty file for no target, else the target's files moved under `path`.
    fn symlink_target(&self, path: &str, target: u32) -> io::Result<Vec<File>> {
        if target == 0 {
            return Ok(vec![File {
                path: path.to_string(),
                ..File::default()
            }]);
        }
        Ok(match self.input(target)? {
            Input::File(file) => vec![File {
                path: path.to_string(),
                ..file.clone()
            }],
            Input::Directory { path: dir, files } => files
                .iter()
                .map(|file| File {
                    path: format!("{path}{}", &file.path[dir.len()..]),
                    ..file.clone()
                })
                .collect(),
        })
    }

    /// A runfiles tree as the directory of files it lays out. On a path
    /// collision later sources win, in Bazel's order: symlinks, artifacts at
    /// their canonical place, empty files, root symlinks, the repo mapping
    /// manifest, and the workspace directory's `.runfile` when nothing else
    /// would create that directory.
    fn runfiles_dir(&self, tree: &entry::RunfilesTree) -> io::Result<Input> {
        let workspace = self.workspace_runfiles_directory.clone();
        let mut runfiles = OrderedFiles::default();
        let mut has_workspace_dir = tree.legacy_external_runfiles;
        let in_workspace = |relative: &str| relative.starts_with(&format!("{workspace}/"));

        self.visit_symlink_entries(tree, false, &mut |relative, files| {
            has_workspace_dir |= in_workspace(relative);
            files.into_iter().for_each(|f| runfiles.put(f));
        })?;

        let mut artifacts: Vec<File> = Vec::new();
        let mut seen: HashSet<Vec<u8>> = HashSet::new();
        let mut workspace_input = false;
        self.visit_input_set(
            tree.input_set_id,
            &mut |file| {
                if seen.insert(file.encode_to_vec()) {
                    artifacts.push(file.clone());
                }
            },
            // Bug-for-bug with Bazel's Runfiles: an empty non-external
            // directory counts as an entry under the workspace directory.
            &mut |input| {
                workspace_input |= self
                    .runfiles_path(input.path())
                    .is_some_and(|(repo, _)| repo.is_none());
            },
        )?;
        has_workspace_dir |= workspace_input;
        for file in artifacts {
            for relative in self.runfiles_paths(&file.path, tree.legacy_external_runfiles)? {
                runfiles.put(File {
                    path: format!("{}/{relative}", tree.path),
                    ..file.clone()
                });
            }
        }

        for empty in &tree.empty_files {
            // Siblings or parents of existing files: they never by themselves
            // create the workspace directory.
            let path = match empty.strip_prefix("../") {
                Some(rest) => format!("{}/{rest}", tree.path),
                None => format!("{}/{workspace}/{empty}", tree.path),
            };
            runfiles.put(File {
                path,
                ..File::default()
            });
        }

        self.visit_symlink_entries(tree, true, &mut |relative, files| {
            has_workspace_dir |= in_workspace(relative);
            files.into_iter().for_each(|f| runfiles.put(f));
        })?;

        if let Some(manifest) = &tree.repo_mapping_manifest {
            runfiles.put_at(
                REPO_MAPPING_MANIFEST.to_string(),
                File {
                    path: format!("{}/{REPO_MAPPING_MANIFEST}", tree.path),
                    ..self.file(None, manifest.clone())
                },
            );
        }

        if !tree.legacy_external_runfiles && !has_workspace_dir {
            let path = format!("{}/{workspace}/.runfile", tree.path);
            runfiles.put(File {
                path,
                ..File::default()
            });
        }

        Ok(Input::Directory {
            path: tree.path.clone(),
            files: runfiles.into_files(),
        })
    }

    /// The runfiles-relative paths of the artifact at `exec_path`: under the
    /// workspace directory for a main-repository file, else under its
    /// repository's, and with legacy external runfiles under
    /// `<workspace>/external/<repo>` too.
    fn runfiles_paths(&self, exec_path: &str, legacy: bool) -> io::Result<Vec<String>> {
        let Some((repo, path)) = self.runfiles_path(exec_path) else {
            return Err(invalid(format!("not a runfiles path: {exec_path:?}")));
        };
        let workspace = &self.workspace_runfiles_directory;
        Ok(match repo {
            None => vec![format!("{workspace}/{path}")],
            Some(repo) if legacy => vec![
                format!("{repo}/{path}"),
                format!("{workspace}/external/{repo}/{path}"),
            ],
            Some(repo) => vec![format!("{repo}/{path}")],
        })
    }

    /// `(repo, repo-relative path)` of an exec path, `repo` `None` for the
    /// main repository, as Bazel's runfiles path patterns read it.
    fn runfiles_path<'a>(&self, exec_path: &'a str) -> Option<(Option<&'a str>, &'a str)> {
        if self.sibling_repository_layout {
            sibling_generated(exec_path).or_else(|| sibling_source(exec_path))
        } else {
            default_generated(exec_path).or_else(|| default_source(exec_path))
        }
    }
}

/// Files by path in first-insertion order, a later file at a path replacing
/// the earlier one in its place, as Java's `LinkedHashMap` keeps them.
#[derive(Default)]
struct OrderedFiles {
    order: Vec<String>,
    files: HashMap<String, File>,
}

impl OrderedFiles {
    fn put(&mut self, file: File) {
        self.put_at(file.path.clone(), file);
    }

    fn put_at(&mut self, key: String, file: File) {
        if self.files.insert(key.clone(), file).is_none() {
            self.order.push(key);
        }
    }

    fn into_files(mut self) -> Vec<File> {
        self.order
            .iter()
            .filter_map(|key| self.files.remove(key))
            .collect()
    }
}

/// The segments after `bazel-out/` or `blaze-out/`, if `path` is an output.
fn after_out_dir(path: &str) -> Option<&str> {
    path.strip_prefix("bazel-out/")
        .or_else(|| path.strip_prefix("blaze-out/"))
}

/// `(first segment, the rest)` of `path`, both non-empty.
fn segment(path: &str) -> Option<(&str, &str)> {
    let (first, rest) = path.split_once('/')?;
    (!first.is_empty() && !rest.is_empty()).then_some((first, rest))
}

/// `(?:bazel|blaze)-out/[^/]+/[^/]+/(?:external/(?<repo>[^/]+)/)?(?<path>.+)`
fn default_generated(path: &str) -> Option<(Option<&str>, &str)> {
    let (_, rest) = segment(after_out_dir(path)?)?;
    let (_, rest) = segment(rest)?;
    Some(external(rest))
}

/// `(?:external/(?<repo>[^/]+)/)?(?<path>.+)`
fn default_source(path: &str) -> Option<(Option<&str>, &str)> {
    (!path.is_empty()).then(|| external(path))
}

/// An optional `external/<repo>/` prefix of a non-empty `path`.
fn external(path: &str) -> (Option<&str>, &str) {
    path.strip_prefix("external/")
        .and_then(segment)
        .map_or((None, path), |(repo, rest)| (Some(repo), rest))
}

/// `(?:bazel|blaze)-out/(?:(?<repo>[^/]+(?=/[^/]+-[^/]+/)(?!/coverage-metadata/))/)?[^/]+/[^/]+/(?<path>.+)`:
/// a repository segment comes before a configuration (which has a hyphen,
/// unlike a repository's mnemonic segment).
fn sibling_generated(path: &str) -> Option<(Option<&str>, &str)> {
    let after = after_out_dir(path)?;
    let with_repo = segment(after).and_then(|(repo, rest)| {
        let (config, _) = segment(rest)?;
        let hyphenated = config
            .find('-')
            .is_some_and(|i| i > 0 && i < config.len() - 1);
        if !hyphenated || config == "coverage-metadata" {
            return None;
        }
        let (_, rest) = segment(rest)?;
        let (_, rest) = segment(rest)?;
        Some((Some(repo), rest))
    });
    with_repo.or_else(|| {
        let (_, rest) = segment(after)?;
        let (_, rest) = segment(rest)?;
        Some((None, rest))
    })
}

/// `(?:\.\./(?<repo>[^/]+)/)?(?<path>.+)`
fn sibling_source(path: &str) -> Option<(Option<&str>, &str)> {
    if path.is_empty() {
        return None;
    }
    Some(
        path.strip_prefix("../")
            .and_then(segment)
            .map_or((None, path), |(repo, rest)| (Some(repo), rest)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runfiles_paths_follow_bazels_patterns() {
        assert_eq!(
            default_generated("bazel-out/k8-fastbuild/bin/pkg/file.txt"),
            Some((None, "pkg/file.txt"))
        );
        assert_eq!(
            default_generated("bazel-out/k8-fastbuild/bin/external/some_repo/pkg/file.txt"),
            Some((Some("some_repo"), "pkg/file.txt"))
        );
        assert_eq!(default_generated("pkg/file.txt"), None);
        assert_eq!(default_source("pkg/file.txt"), Some((None, "pkg/file.txt")));
        assert_eq!(
            default_source("external/some_repo/pkg/file.txt"),
            Some((Some("some_repo"), "pkg/file.txt"))
        );
        assert_eq!(
            sibling_generated("bazel-out/k8-fastbuild/bin/pkg/file.txt"),
            Some((None, "pkg/file.txt"))
        );
        assert_eq!(
            sibling_generated("bazel-out/some_repo/k8-fastbuild/bin/pkg/file.txt"),
            Some((Some("some_repo"), "pkg/file.txt"))
        );
        assert_eq!(
            sibling_generated("bazel-out/k8-fastbuild/k8-fastbuild/bin/pkg/file.txt"),
            Some((Some("k8-fastbuild"), "pkg/file.txt"))
        );
        assert_eq!(
            sibling_source("../some_repo/pkg/file.txt"),
            Some((Some("some_repo"), "pkg/file.txt"))
        );
    }
}
