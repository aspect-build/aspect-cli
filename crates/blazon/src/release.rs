use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

/// An extracted Bazel release: the server jar and the JDK it shipped with.
pub struct Install {
    pub root: PathBuf,
    pub server_jar: PathBuf,
    pub java_home: PathBuf,
}

impl Install {
    pub fn at(root: PathBuf) -> Result<Install> {
        let server_jar = root.join("A-server.jar");
        let java_home = root.join("embedded_tools").join("jdk");
        if !server_jar.is_file() {
            bail!(
                "{} is not a Bazel install base: no A-server.jar",
                root.display()
            );
        }
        if !java_home.is_dir() {
            bail!("{} has no embedded JDK", root.display());
        }
        Ok(Install {
            root,
            server_jar,
            java_home,
        })
    }

    pub fn libjvm(&self) -> PathBuf {
        let name = if cfg!(target_os = "macos") {
            "libjvm.dylib"
        } else {
            "libjvm.so"
        };
        self.java_home.join("lib").join("server").join(name)
    }
}

/// A Bazel version, resolvable to an [`Install`] in the buildx cache.
pub struct Release {
    pub version: String,
}

impl Release {
    /// `$BLAZON_BAZEL_CACHE`, or the user cache's `buildx` directory.
    pub fn cache_root() -> Result<PathBuf> {
        if let Some(root) = std::env::var_os("BLAZON_BAZEL_CACHE") {
            return Ok(PathBuf::from(root));
        }
        Ok(dirs::cache_dir()
            .context("no cache directory for this user")?
            .join("buildx"))
    }

    /// Returns the extracted release, downloading it on first use. `from` extracts a
    /// local release binary instead of downloading.
    pub fn install(&self, from: Option<&Path>) -> Result<Install> {
        let root = Self::cache_root()?.join("bazel").join(&self.version);
        if root.join("A-server.jar").is_file() {
            return Install::at(root);
        }
        let partial = root.with_extension("partial");
        if partial.exists() {
            fs::remove_dir_all(&partial)?;
        }
        fs::create_dir_all(&partial)?;
        let file = match from {
            Some(path) => File::open(path).with_context(|| format!("open {}", path.display()))?,
            None => self.download()?,
        };
        extract(file, &partial)?;
        let label = fs::read_to_string(partial.join("build-label.txt"))
            .map(|l| l.trim().to_string())
            .unwrap_or_default();
        if label != self.version {
            fs::remove_dir_all(&partial)?;
            bail!("release binary is Bazel {label:?}, not {}", self.version);
        }
        fs::rename(&partial, &root)?;
        Install::at(root)
    }

    fn download(&self) -> Result<File> {
        let platform = platform()?;
        let url = format!(
            "https://releases.bazel.build/{v}/release/bazel-{v}-{platform}",
            v = self.version
        );
        let downloads = Self::cache_root()?.join("downloads");
        fs::create_dir_all(&downloads)?;
        let dest = downloads.join(format!("bazel-{}-{platform}", self.version));
        if !dest.is_file() {
            eprintln!("blazon: downloading {url}");
            let mut resp = reqwest::blocking::get(&url)?.error_for_status()?;
            let tmp = dest.with_extension("part");
            let mut out = File::create(&tmp)?;
            resp.copy_to(&mut out)?;
            fs::rename(&tmp, &dest)?;
        }
        Ok(File::open(&dest)?)
    }
}

fn platform() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        (os, arch) => bail!("no Bazel release for {os}-{arch}"),
    })
}

/// The release binary is a native launcher with a zip appended; zip readers
/// locate the central directory from the end, so the prefix is harmless.
fn extract(file: File, dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(file).context("release binary is not a zip")?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(name);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut target = File::create(&out)?;
        io::copy(&mut entry, &mut target)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}
