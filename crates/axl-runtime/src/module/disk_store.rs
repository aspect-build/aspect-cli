use std::collections::HashMap;
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

use crate::builtins;

use super::store::ModuleStore;
use super::{AxlArchiveDep, AxlLocalDep, Dep};

pub struct DiskStore {
    #[allow(unused)]
    root: PathBuf,
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
    pub fn new(root: PathBuf) -> Self {
        Self {
            root_sha: sha256::digest(root.as_os_str().as_bytes()),
            root,
        }
    }

    fn root(&self) -> PathBuf {
        cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("aspect")
            .join("axl")
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

    fn download_tmp_file(&self) -> PathBuf {
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

    async fn expand_dep(&self, dep: &AxlArchiveDep) -> Result<(), io::Error> {
        let dep_path = self.dep_path(&dep.name);
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

    async fn link_dep(&self, dep: &AxlLocalDep) -> Result<(), io::Error> {
        let dep_path = self.dep_path(&dep.name);
        // Remove any existing symlink before creating a new one
        let _ = fs::remove_file(&dep_path).await;
        fs::symlink(&dep.path, dep_path).await
    }

    pub async fn expand_store(
        &self,
        store: &ModuleStore,
    ) -> Result<Vec<(String, PathBuf, bool)>, StoreError> {
        let root = self.root();
        fs::create_dir_all(&root).await?;
        fs::create_dir_all(self.deps_path()).await?;
        fs::create_dir_all(&root.join("cas")).await?;
        fs::create_dir_all(&root.join("dl")).await?;
        fs::create_dir_all(&root.join("builtins")).await?;

        let client = reqwest::Client::new();

        let mut all: HashMap<String, Dep> =
            builtins::expand_builtins(self.root.clone(), root.join("builtins"))?
                .into_iter()
                .map(|(name, path)| {
                    (
                        name.clone(),
                        Dep::Local(AxlLocalDep {
                            name: name,
                            path: path,
                            // Builtins tasks are always auto used
                            auto_use_tasks: true,
                            use_config: true,
                        }),
                    )
                })
                .collect();

        all.extend(store.deps.take());

        let mut module_roots = vec![];

        for dep in all.values() {
            let dep_marker_path = self.dep_marker_path(&dep);
            let dep_path = self.dep_path(dep.name());

            match dep {
                Dep::Local(local) if local.auto_use_tasks || local.use_config => {
                    module_roots.push((local.name.clone(), dep_path.clone(), local.use_config))
                }
                Dep::Remote(remote) if remote.auto_use_tasks || remote.use_config => {
                    module_roots.push((remote.name.clone(), dep_path.clone(), remote.use_config))
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

            if dep_marker_path.exists() {
                let prev_hash = fs::read_to_string(&dep_marker_path).await?;
                if prev_hash != current_hash {
                    if let Ok(metadata) = fs::symlink_metadata(&dep_path).await {
                        if metadata.is_symlink() {
                            fs::remove_file(&dep_path).await?;
                        } else {
                            fs::remove_dir_all(&dep_path).await?;
                        }
                    }
                }
            } else {
                // if we have no marker file and the cas_path exists, the safest thing to do is to delete the
                // current dep path and start over
                if let Ok(metadata) = fs::symlink_metadata(&dep_path).await {
                    if metadata.is_symlink() {
                        fs::remove_file(&dep_path).await?;
                    } else {
                        fs::remove_dir_all(&dep_path).await?;
                    }
                }
            }

            if fs::symlink_metadata(&dep_path).await.is_err() {
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
                                            eprintln!("failed to fetch `{url}`: {err}");
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

                        fs::create_dir_all(&dep_path).await?;
                        self.expand_dep(dep)
                            .await
                            .map_err(|err| StoreError::UnpackError(err))?;
                    }
                }
            }

            // write the marker once the expansion is succesful
            if !current_hash.is_empty() {
                fs::write(&dep_marker_path, current_hash).await?;
            }
        }

        Ok(module_roots)
    }
}
