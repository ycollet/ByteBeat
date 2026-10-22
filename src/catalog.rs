use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{engine::Mode, songs::Song};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub rate: f64,
    pub mode: Mode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub file: PathBuf,
    #[serde(flatten)]
    pub metadata: Metadata,
}

pub fn find_existing(paths: &[PathBuf]) -> Result<Option<PathBuf>, String> {
    for path in paths {
        if path
            .try_exists()
            .map_err(|e| format!("{}: {e}", path.display()))?
        {
            return Ok(Some(path.clone()));
        }
    }
    Ok(None)
}

pub fn load_songs(path: &Path) -> Result<Vec<Song>, String> {
    entries(path)?.iter().map(load).collect()
}

#[derive(Deserialize)]
struct Catalog {
    version: u32,
    songs: Vec<Entry>,
}

pub fn entries(path: &Path) -> Result<Vec<Entry>, String> {
    let contents = read(path)?;
    let catalog: Catalog =
        serde_json::from_str(&contents).map_err(|e| format!("{}: {e}", path.display()))?;
    if catalog.version != 1 {
        return Err(format!(
            "{}: unsupported catalog version {}",
            path.display(),
            catalog.version
        ));
    }
    let directory = path.parent().unwrap_or(Path::new("."));
    Ok(catalog
        .songs
        .into_iter()
        .map(|mut entry| {
            entry.file = directory.join(&entry.file);
            entry
        })
        .collect())
}

pub fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(entry: &Entry) -> Result<Song, String> {
    Ok(Song {
        title: entry.metadata.title.clone(),
        author: entry.metadata.author.clone(),
        rate: entry.metadata.rate,
        mode: entry.metadata.mode,
        code: read(&entry.file)?,
    })
}

pub fn file_entry(path: &Path) -> Result<Entry, String> {
    let sidecar = path.with_extension("json");
    let metadata = if sidecar
        .try_exists()
        .map_err(|e| format!("{}: {e}", sidecar.display()))?
    {
        serde_json::from_str(&read(&sidecar)?).map_err(|e| format!("{}: {e}", sidecar.display()))?
    } else {
        Metadata {
            title: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            author: "Local file".into(),
            mode: Mode::Bytebeat,
            rate: 8_000.0,
        }
    };
    Ok(Entry {
        file: path.into(),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_metadata_preserves_fractional_rates_and_modes() {
        let metadata: Metadata = serde_json::from_str(
            r#"{"title":"Test","author":"Author","rate":35498.66666,"mode":"Signed Bytebeat"}"#,
        )
        .unwrap();
        assert_eq!(metadata.rate, 35498.66666);
        assert_eq!(metadata.mode, Mode::Signed);
    }

    #[test]
    fn missing_script_is_an_explicit_error() {
        let entry = Entry {
            file: PathBuf::from("does-not-exist.js"),
            metadata: Metadata {
                title: "Missing".into(),
                author: "Test".into(),
                rate: 8_000.0,
                mode: Mode::Bytebeat,
            },
        };
        assert!(load(&entry).err().unwrap().contains("does-not-exist.js"));
    }

    #[test]
    fn discovery_prefers_local_then_installed_then_cached_catalogs() {
        let root = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["local.json", "installed.json", "cached.json"]
            .iter()
            .map(|name| root.path().join(name))
            .collect();
        assert!(find_existing(&paths).unwrap().is_none());
        fs::write(&paths[2], "{}").unwrap();
        assert_eq!(find_existing(&paths).unwrap(), Some(paths[2].clone()));
        fs::write(&paths[1], "{}").unwrap();
        assert_eq!(find_existing(&paths).unwrap(), Some(paths[1].clone()));
        fs::write(&paths[0], "{}").unwrap();
        assert_eq!(find_existing(&paths).unwrap(), Some(paths[0].clone()));
    }
}
