use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct Jar {
    archive: zip::ZipArchive<File>,
    path: String,
}

impl Jar {
    pub fn open(path: &Path) -> Result<Jar> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let archive =
            zip::ZipArchive::new(file).with_context(|| format!("read {}", path.display()))?;
        Ok(Jar {
            archive,
            path: path.display().to_string(),
        })
    }

    /// Whether the jar has this class (internal, slash-separated name).
    pub fn contains(&self, internal: &str) -> bool {
        self.archive
            .file_names()
            .any(|n| n.strip_suffix(".class") == Some(internal))
    }

    /// Class bytes for an internal (slash-separated) name.
    pub fn class(&mut self, internal: &str) -> Result<Vec<u8>> {
        let entry = format!("{internal}.class");
        let mut file = match self.archive.by_name(&entry) {
            Ok(f) => f,
            Err(_) => bail!("class {internal} not found in {}", self.path),
        };
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}
