use crate::errln;
use std::{io, os::unix::ffi::OsStrExt, path::PathBuf, str::FromStr};

use anyhow::anyhow;
use dirs::cache_dir;
use flate2::read::GzDecoder;
use futures_util::TryStreamExt;
use rand::prelude::*;
use reqwest::{self, Client, Method, Request, Url};
use ssri::{Algorithm, Integrity, IntegrityChecker, IntegrityOpts};
use thiserror::Error;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;

use super::module::Mod;
use super::{AxlArchiveDep, AxlLocalDep, Dep};

pub struct DiskStore {
    cache_root: PathBuf,
    root_sha: String,
}

#[derive(Error, Debug)]
pub enum StoreError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    FetchError(#[from] reqwest::Error),
    #[error(transparent)]
    ChecksumError(#[from] ssri::Error),
    #[error("failed to unpack: {0}")]
    UnpackError(std::io::Error),
    #[error("failed to link: {0}")]
    LinkError(std::io::Error),
    #[error("missing integrity for module @{0}, set integrity = \"{1}\"")]
    MissingIntegrity(String, Integrity),
}

enum Processor {
    Check(IntegrityChecker),
    Hash(IntegrityOpts),
}

impl Processor {
    fn new_check(integrity: Integrity) -> Self {
        Processor::Check(IntegrityChecker::new(integrity))
    }

    fn new_hash() -> Self {
        Processor::Hash(IntegrityOpts::new().algorithm(Algorithm::Sha512))
    }

    fn update<B: AsRef<[u8]>>(&mut self, data: B) {
        match self {
            Processor::Check(checker) => checker.input(data),
            Processor::Hash(opts) => opts.input(data),
        }
    }

    fn finalize(self) -> Result<Option<Integrity>, ssri::Error> {
        match self {
            Processor::Check(checker) => checker.result().map(|_| None),
            Processor::Hash(opts) => Ok(Some(opts.result())),
        }
    }
}

impl DiskStore {
    pub fn new(repo_root: PathBuf) -> Self {
        // Cache root resolution:
        //   1. ASPECT_CLI_CACHE env (non-empty) → ${ASPECT_CLI_CACHE}/axl
        //   2. fallback             → ${cache_dir()}/aspect/axl
        // CI runners point ASPECT_CLI_CACHE at an ephemeral mount that's
        // included in the warming archive, so the AXL deps cache survives
        // runner reboots.
        let cache_root = match std::env::var("ASPECT_CLI_CACHE") {
            Ok(val) if !val.is_empty() => PathBuf::from(val).join("axl"),
            _ => cache_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("aspect")
                .join("axl"),
        };
        Self {
            root_sha: sha256::digest(repo_root.as_os_str().as_bytes()),
            cache_root,
        }
    }

    fn root(&self) -> PathBuf {
        self.cache_root.clone()
    }

    pub fn deps_path(&self) -> PathBuf {
        self.root().join("deps").join(&self.root_sha)
    }

    fn dep_path(&self, dep: &str) -> PathBuf {
        self.deps_path().join(dep)
    }

    fn dep_marker_path(&self, dep: &Dep) -> PathBuf {
        self.deps_path().join(format!("{}@marker", dep.name()))
    }

    fn cas_path_for_integrity(&self, integrity: &Integrity) -> PathBuf {
        let hex = integrity.to_hex();
        self.root().join("cas").join(hex.0.to_string()).join(hex.1)
    }

    /// A path beside `path` that no other process will pick, for staging a
    /// file, symlink or directory that is then renamed onto `path`.
    fn staging_path(path: &PathBuf) -> PathBuf {
        #[allow(deprecated)]
        let mut rng = rand::thread_rng();
        let suffix: u64 = rng.r#gen();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        path.with_file_name(format!(".{name}.{suffix:016x}.tmp"))
    }

    fn download_tmp_file(&self) -> PathBuf {
        #[allow(deprecated)]
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let hex_string: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        self.root()
            .join("dl")
            .join(hex_string)
            .with_extension("tmp")
    }

    async fn fetch_dep(
        &self,
        client: &Client,
        dep: &AxlArchiveDep,
        url: &String,
    ) -> Result<Option<Integrity>, StoreError> {
        let compute_integrity = dep.integrity.is_none();

        let tmp_file = self.download_tmp_file();
        let mut tmp = File::create(&tmp_file).await?;

        let req = Request::new(
            Method::GET,
            Url::from_str(url.as_str()).expect("url should have been validated in axl_archive_dep"),
        );

        let mut byte_stream = client
            .execute(req)
            .await?
            .error_for_status()?
            .bytes_stream();

        let mut processor = if compute_integrity {
            Processor::new_hash()
        } else {
            Processor::new_check(dep.integrity.clone().unwrap())
        };

        while let Some(item) = byte_stream.try_next().await? {
            processor.update(&item);
            tmp.write_all(&item).await?;
        }

        let result = match processor.finalize() {
            Ok(res) => res,
            Err(err) => {
                let _ = fs::remove_file(&tmp_file).await;
                return Err(StoreError::ChecksumError(err));
            }
        };

        if compute_integrity {
            let _ = fs::remove_file(&tmp_file).await;
            Ok(result)
        } else {
            let integrity = dep.integrity.as_ref().unwrap();
            let cas_path = self.cas_path_for_integrity(integrity);
            tokio::fs::rename(&tmp_file, &cas_path).await?;
            Ok(None)
        }
    }

    async fn expand_dep(&self, dep: &AxlArchiveDep, dep_path: &PathBuf) -> Result<(), io::Error> {
        let integrity = dep.integrity.as_ref().expect("integrity must be set");
        let cas_path = self.cas_path_for_integrity(integrity);
        let raw = File::open(&cas_path).await?;
        let raw = raw.into_std().await;
        let decoder = GzDecoder::new(raw);
        let mut archive = tar::Archive::new(decoder);
        let entries = archive.entries()?;
        let mut found_matching_entries = false;
        for entry in entries {
            let mut entry = entry?;
            let path = entry.path()?;
            if entry.link_name().is_ok_and(|f| f.is_some()) {
                // We don't know how to safely handle symlinks yet so forbid it
                // for now.
                // TODO: implement this with a chroot style symlink normalization.
                continue;
            }

            // If the strip_prefix is specified and entry does not start with it
            // skip the entry as we won't need it.
            if !dep.strip_prefix.is_empty() && !path.starts_with(&dep.strip_prefix) {
                continue;
            }

            // Set it to true since, there was at least one matching entry.
            found_matching_entries = true;

            let new_dst = path
                .strip_prefix(&dep.strip_prefix)
                .expect("entry must have had strip_prefix. please file a bug");
            if new_dst.as_os_str().eq("/") || new_dst.as_os_str().eq("") {
                continue;
            }
            let new_dst_abs = dep_path.join(new_dst);

            entry.unpack(new_dst_abs)?;
        }

        if !dep.strip_prefix.is_empty() && !found_matching_entries {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                anyhow!(
                    "strip_prefix {} was provided but it does not match any entries.",
                    dep.strip_prefix
                ),
            ));
        }

        Ok(())
    }

    /// Point the dep's symlink at `dep.path`. The link is made under a staging
    /// name and renamed over the old one, which replaces it atomically, so a
    /// concurrent process loading from it never finds the path missing.
    async fn link_dep(&self, dep: &AxlLocalDep) -> Result<(), io::Error> {
        let dep_path = self.dep_path(&dep.name);
        let staged = Self::staging_path(&dep_path);
        fs::symlink(&dep.path, &staged).await?;
        if let Err(err) = fs::rename(&staged, &dep_path).await {
            let _ = fs::remove_file(&staged).await;
            return Err(err);
        }
        Ok(())
    }

    /// Write `contents` to `path` by renaming a staged copy over it, so a
    /// reader sees the old contents or the new, never a truncated file.
    async fn write_atomic(path: &PathBuf, contents: &str) -> Result<(), io::Error> {
        let staged = Self::staging_path(path);
        fs::write(&staged, contents).await?;
        if let Err(err) = fs::rename(&staged, path).await {
            let _ = fs::remove_file(&staged).await;
            return Err(err);
        }
        Ok(())
    }

    pub fn builtins_path(&self) -> PathBuf {
        self.root().join("builtins")
    }

    /// Bring every dep under `deps/` up to date with `store` and `builtins`.
    ///
    /// Runs at every CLI start, so concurrent invocations run it side by side
    /// against the same directory. Each dep's state therefore only ever moves
    /// by atomic renames — its marker, its symlink, its unpacked tree — and is
    /// left alone when it is already current, so one process never removes a
    /// dep that another is loading from.
    pub async fn expand_store(
        &self,
        store: &Mod,
        builtins: Vec<(String, PathBuf)>,
    ) -> Result<Vec<(String, PathBuf)>, StoreError> {
        let root = self.root();
        fs::create_dir_all(&root).await?;
        fs::create_dir_all(self.deps_path()).await?;
        fs::create_dir_all(&root.join("cas")).await?;
        fs::create_dir_all(&root.join("dl")).await?;

        let client = reqwest::Client::new();

        let builtin_deps: Vec<Dep> = builtins
            .into_iter()
            .map(|(name, path)| {
                Dep::Local(AxlLocalDep {
                    name,
                    path,
                    // Builtins tasks are always auto used
                    auto_use_tasks: true,
                })
            })
            .collect();

        let mut module_roots = vec![];

        for dep in builtin_deps.iter().chain(store.deps.iter()) {
            let dep_marker_path = self.dep_marker_path(&dep);
            let dep_path = self.dep_path(dep.name());

            match dep {
                Dep::Local(local) if local.auto_use_tasks => {
                    module_roots.push((local.name.clone(), dep_path.clone()))
                }
                Dep::Remote(remote) if remote.auto_use_tasks => {
                    module_roots.push((remote.name.clone(), dep_path.clone()))
                }
                _ => {}
            };

            let current_hash = match dep {
                Dep::Local(dep) => sha256::digest(dep.path.to_str().unwrap()),
                Dep::Remote(dep) => {
                    if let Some(integrity) = &dep.integrity {
                        sha256::digest(format!("{}{}", integrity, dep.strip_prefix))
                    } else {
                        "".to_string()
                    }
                }
            };

            // A missing marker, or one recording another source, means the dep
            // path cannot be trusted. A local dep's symlink is simply replaced;
            // anything else there is removed and expanded again.
            let prev_hash = fs::read_to_string(&dep_marker_path).await.ok();
            let current =
                prev_hash.as_deref() == Some(current_hash.as_str()) && !current_hash.is_empty();
            let metadata = fs::symlink_metadata(&dep_path).await.ok();
            let linked_here = match (dep, &metadata) {
                (Dep::Local(local), Some(meta)) if meta.is_symlink() => {
                    fs::read_link(&dep_path).await.ok().as_ref() == Some(&local.path)
                }
                _ => false,
            };
            if !current && !linked_here {
                if let Some(meta) = &metadata {
                    if meta.is_dir() {
                        fs::remove_dir_all(&dep_path).await?;
                    } else if !matches!(dep, Dep::Local(_)) {
                        fs::remove_file(&dep_path).await?;
                    }
                }
            }

            let needs_link = matches!(dep, Dep::Local(_)) && !linked_here;
            if needs_link || fs::symlink_metadata(&dep_path).await.is_err() {
                match dep {
                    Dep::Local(local) => {
                        self.link_dep(local)
                            .await
                            .map_err(|err| StoreError::LinkError(err))?;
                    }
                    Dep::Remote(dep) => {
                        let cas_path = dep
                            .integrity
                            .as_ref()
                            .map(|integrity| self.cas_path_for_integrity(integrity));

                        let need_fetch = match &cas_path {
                            Some(cas_path) => !tokio::fs::try_exists(cas_path).await?,
                            None => true,
                        };

                        if need_fetch {
                            if let Some(parent) = cas_path.as_ref().and_then(|p| p.parent()) {
                                fs::create_dir_all(parent).await?;
                            }

                            let mut last_err: Option<StoreError> = None;
                            let mut fetched = false;

                            for (i, url) in dep.urls.iter().enumerate() {
                                match self.fetch_dep(&client, dep, url).await {
                                    Ok(Some(computed)) => {
                                        return Err(StoreError::MissingIntegrity(
                                            dep.name.clone(),
                                            computed,
                                        ));
                                    }
                                    Ok(None) => {
                                        fetched = true;
                                        break;
                                    }
                                    Err(err) => {
                                        if dep.urls.len() > 1 && i != dep.urls.len() - 1 {
                                            errln!("failed to fetch `{url}`: {err}");
                                        }
                                        last_err = Some(err);
                                        if i == dep.urls.len() - 1 {
                                            return Err(last_err.unwrap());
                                        }
                                    }
                                }
                            }

                            if !fetched {
                                return Err(last_err.unwrap());
                            }
                        }

                        // Unpacked beside the dep and renamed into place, so the
                        // dep path holds a whole tree or none. A process that
                        // loses the race to rename keeps the winner's tree.
                        let staged = Self::staging_path(&dep_path);
                        fs::create_dir_all(&staged).await?;
                        if let Err(err) = self.expand_dep(dep, &staged).await {
                            let _ = fs::remove_dir_all(&staged).await;
                            return Err(StoreError::UnpackError(err));
                        }
                        if fs::rename(&staged, &dep_path).await.is_err() {
                            let _ = fs::remove_dir_all(&staged).await;
                            if fs::symlink_metadata(&dep_path).await.is_err() {
                                return Err(StoreError::UnpackError(io::Error::new(
                                    io::ErrorKind::Other,
                                    format!("could not move {} into place", dep_path.display()),
                                )));
                            }
                        }
                    }
                }
            }

            // Written once the dep is in place, and only when it changed: every
            // CLI start passes through here, and rewriting an unchanged marker
            // is what let a concurrent reader catch it empty.
            if !current_hash.is_empty() && prev_hash.as_deref() != Some(current_hash.as_str()) {
                Self::write_atomic(&dep_marker_path, &current_hash).await?;
            }
        }

        module_roots.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(module_roots)
    }
}
