use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct Jar {
    archive: zip::ZipArchive<File>,
    path: String,
    classes: HashSet<String>,
}

impl Jar {
    pub fn open(path: &Path) -> Result<Jar> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let archive =
            zip::ZipArchive::new(file).with_context(|| format!("read {}", path.display()))?;
        let classes = archive
            .file_names()
            .filter_map(|n| n.strip_suffix(".class"))
            .map(str::to_string)
            .collect();
        Ok(Jar {
            archive,
            path: path.display().to_string(),
            classes,
        })
    }

    /// Whether the jar has this class (internal, slash-separated name).
    pub fn contains(&self, internal: &str) -> bool {
        self.classes.contains(internal)
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

impl Jar {
    fn class_prefix(&mut self, internal: &str, max: u64) -> Result<Vec<u8>> {
        let entry = format!("{internal}.class");
        let file = self
            .archive
            .by_name(&entry)
            .with_context(|| format!("class {internal} not found in {}", self.path))?;
        let mut bytes = Vec::new();
        file.take(max).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// The superclass of every class in the jar, by internal name, read from
    /// the class headers alone (no members, no attributes), which is what finding
    /// "every subclass of X" needs without parsing the whole jar.
    pub fn superclasses(&mut self) -> Result<HashMap<String, String>> {
        let names: Vec<String> = self.classes.iter().cloned().collect();
        let mut out = HashMap::with_capacity(names.len());
        for name in names {
            // The header is at the start; most classes need a few KB of it, so
            // decompress a prefix and read the whole class only if that was short.
            let prefix = self.class_prefix(&name, 8192)?;
            let sup = match header_superclass(&prefix) {
                Header::Super(s) => Some(s),
                Header::NoSuper => None,
                Header::Truncated => match header_superclass(&self.class(&name)?) {
                    Header::Super(s) => Some(s),
                    _ => None,
                },
            };
            if let Some(sup) = sup {
                out.insert(name, sup);
            }
        }
        Ok(out)
    }
}

enum Header {
    Super(String),
    /// A class with no superclass, or not a class file.
    NoSuper,
    /// The bytes end before the header does.
    Truncated,
}

/// The superclass named in a class file's header.
fn header_superclass(b: &[u8]) -> Header {
    match scan_header(b) {
        Some(Some(s)) => Header::Super(s),
        Some(None) => Header::NoSuper,
        None => Header::Truncated,
    }
}

fn scan_header(b: &[u8]) -> Option<Option<String>> {
    let u16_at = |i: usize| -> Option<usize> {
        Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize)
    };
    if b.len() >= 4 && b[0..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return Some(None);
    }
    let count = u16_at(8)?;
    let mut utf8: Vec<(usize, usize)> = vec![(0, 0); count];
    let mut classes: Vec<usize> = vec![0; count];
    let mut i = 10;
    let mut n = 1;
    while n < count {
        let tag = *b.get(i)?;
        match tag {
            1 => {
                let len = u16_at(i + 1)?;
                utf8[n] = (i + 3, len);
                i += 3 + len;
            }
            7 => {
                classes[n] = u16_at(i + 1)?;
                i += 3;
            }
            8 | 16 | 19 | 20 => i += 3,
            15 => i += 4,
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => i += 5,
            5 | 6 => {
                i += 9;
                n += 1;
            }
            _ => return Some(None),
        }
        n += 1;
    }
    let sup = u16_at(i + 4)?;
    if sup == 0 {
        return Some(None);
    }
    let (start, len) = *utf8.get(*classes.get(sup)?)?;
    let name = std::str::from_utf8(b.get(start..start + len)?).ok()?;
    Some(Some(name.to_string()))
}
