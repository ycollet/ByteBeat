use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    catalog::{Entry, Metadata},
    engine::Mode,
};

const BASE: &str = "https://dollchan.net/bytebeat/";
const INDEX: &str = "https://dollchan.net/bytebeat/data/library/all.gz";
const LICENSE: &str =
    "https://raw.githubusercontent.com/SthephanShinkufag/bytebeat-composer/master/LICENSE";
const MAX_DOWNLOAD: u64 = 32 * 1024 * 1024;
const MAX_INDEX: u64 = 64 * 1024 * 1024;

pub fn cache_root() -> Result<PathBuf, String> {
    dirs::cache_dir()
        .map(|path| path.join("bytebeat").join("upstream"))
        .ok_or_else(|| {
            "Cannot locate the user cache directory; provide --catalog explicitly.".into()
        })
}

pub enum Event {
    Progress(String),
    Finished(Result<PathBuf, String>),
}

pub struct Download {
    pub receiver: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
}

impl Download {
    pub fn start(root: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        thread::spawn(move || {
            let result = download(&root, &cancel, &|message| {
                sender
                    .send(Event::Progress(message))
                    .map_err(|_| "Catalog download cancelled.".into())
            });
            let _ = sender.send(Event::Finished(result));
        });
        Self {
            receiver,
            cancelled,
        }
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub fn download(
    root: &Path,
    cancelled: &AtomicBool,
    progress: &(impl Fn(String) -> Result<(), String> + Sync),
) -> Result<PathBuf, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build(),
    );
    download_with(
        root,
        cancelled,
        &|url| {
            for attempt in 0..3 {
                check_cancelled(cancelled)?;
                let result = (|| {
                    let mut response = agent
                        .get(url)
                        .header("User-Agent", "native-bytebeat/0.1")
                        .call()
                        .map_err(|e| format!("{url}: {e}"))?;
                    response
                        .body_mut()
                        .with_config()
                        .limit(MAX_DOWNLOAD)
                        .read_to_vec()
                        .map_err(|e| format!("{url}: {e}"))
                })();
                match result {
                    Ok(bytes) => return Ok(bytes),
                    Err(error) if attempt < 2 => {
                        log::warn!("{error}; retrying");
                        thread::sleep(Duration::from_millis(500 * (attempt + 1)));
                    }
                    Err(error) => return Err(error),
                }
            }
            unreachable!()
        },
        progress,
    )
}

#[derive(Deserialize)]
struct Group {
    author: String,
    songs: Vec<Value>,
}

enum Source {
    Inline(String),
    Remote(String),
}

struct PlannedSong {
    cached: CachedEntry,
    source: Source,
}

#[derive(Serialize, Deserialize)]
struct CachedEntry {
    #[serde(flatten)]
    entry: Entry,
    source: String,
    sha256: String,
    upstream: Value,
}

#[derive(Serialize)]
struct Manifest<'a> {
    version: u32,
    source: &'static str,
    songs: &'a [CachedEntry],
}

fn plan(author: &str, song: Value) -> Result<PlannedSong, String> {
    let id = song["hash"]
        .as_str()
        .ok_or("Upstream entry has no song ID.")?;
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!("Invalid upstream song ID: {id}"));
    }
    let has_file = |key: &str| song[key] == 1 || song[key] == true;
    let source = if let Some(code) = song["code"].as_str().filter(|_| !has_file("fileOrig")) {
        Source::Inline(code.into())
    } else if let Some(kind) = [
        ("fileOrig", "original"),
        ("fileMin", "minified"),
        ("fileForm", "formatted"),
    ]
    .into_iter()
    .find_map(|(flag, kind)| has_file(flag).then_some(kind))
    {
        Source::Remote(format!("{BASE}data/songs/{kind}/{id}.js"))
    } else if let Some(code) = song["codeMin"].as_str() {
        Source::Inline(code.into())
    } else {
        return Err(format!(
            "{id}: no complete upstream source; a preview is not executable code."
        ));
    };
    let mode = match song.get("mode") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?,
        None => Mode::Bytebeat,
    };
    let metadata = Metadata {
        title: song["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Untitled ({})", &id[..8])),
        author: song["author"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(if author.is_empty() { "Unknown" } else { author })
            .into(),
        rate: song["sampleRate"]
            .as_f64()
            .ok_or_else(|| format!("{id}: missing or invalid sample rate."))?,
        mode,
    };
    let url = match &source {
        Source::Inline(_) => INDEX.into(),
        Source::Remote(url) => url.clone(),
    };
    Ok(PlannedSong {
        cached: CachedEntry {
            entry: Entry {
                file: PathBuf::from(format!("songs/{id}.js")),
                metadata,
            },
            source: url,
            sha256: String::new(),
            upstream: song,
        },
        source,
    })
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Catalog download cancelled.".into())
    } else {
        Ok(())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing cache parent directory.")?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    temporary
        .write_all(bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

fn cached_source(root: &Path, song: &PlannedSong) -> Result<Option<Vec<u8>>, String> {
    let path = root.join(&song.cached.entry.file);
    let sidecar = path.with_extension("json");
    if !path
        .try_exists()
        .map_err(|e| format!("{}: {e}", path.display()))?
        || !sidecar
            .try_exists()
            .map_err(|e| format!("{}: {e}", sidecar.display()))?
    {
        return Ok(None);
    }
    let metadata = crate::catalog::read(&sidecar)?;
    let cached: CachedEntry = match serde_json::from_str(&metadata) {
        Ok(cached) => cached,
        Err(error) => {
            log::warn!(
                "Ignoring invalid cached metadata {}: {error}",
                sidecar.display()
            );
            return Ok(None);
        }
    };
    if cached.source != song.cached.source || cached.upstream != song.cached.upstream {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if format!("{:x}", Sha256::digest(&bytes)) != cached.sha256 {
        log::warn!(
            "Cached song {} failed its checksum; downloading again",
            path.display()
        );
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn download_with(
    root: &Path,
    cancelled: &AtomicBool,
    fetch: &(impl Fn(&str) -> Result<Vec<u8>, String> + Sync),
    progress: &(impl Fn(String) -> Result<(), String> + Sync),
) -> Result<PathBuf, String> {
    check_cancelled(cancelled)?;
    progress("Downloading the upstream song index...".into())?;
    let archive = fetch(INDEX)?;
    let mut json = Vec::new();
    GzDecoder::new(archive.as_slice())
        .take(MAX_INDEX + 1)
        .read_to_end(&mut json)
        .map_err(|e| format!("Invalid upstream compressed catalog: {e}"))?;
    if json.len() as u64 > MAX_INDEX {
        return Err("Upstream catalog exceeds the 64 MiB decompressed limit.".into());
    }
    let groups: Vec<Group> =
        serde_json::from_slice(&json).map_err(|e| format!("Invalid upstream catalog: {e}"))?;
    let songs: Vec<_> = groups
        .into_iter()
        .flat_map(|group| {
            group
                .songs
                .into_iter()
                .map(move |song| plan(&group.author, song))
        })
        .collect::<Result<_, _>>()?;
    if songs.is_empty() {
        return Err("Upstream catalog contains no songs.".into());
    }
    let mut ids = std::collections::HashSet::new();
    for song in &songs {
        if !ids.insert(&song.cached.entry.file) {
            return Err(format!(
                "Duplicate upstream song: {}",
                song.cached.entry.file.display()
            ));
        }
    }
    fs::create_dir_all(root.join("songs")).map_err(|e| format!("{}: {e}", root.display()))?;
    let license = fetch(LICENSE)?;
    atomic_write(&root.join("UPSTREAM-LICENSE"), &license)?;
    atomic_write(&root.join("all.gz"), &archive)?;
    let next = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let aborted = AtomicBool::new(false);
    let mut results = thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..4.min(songs.len()) {
            handles.push(
                scope.spawn(|| -> Result<Vec<(usize, CachedEntry)>, String> {
                    let mut results = Vec::new();
                    while !aborted.load(Ordering::Relaxed) {
                        check_cancelled(cancelled)?;
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(song) = songs.get(index) else { break };
                        let result = (|| {
                            let bytes = match &song.source {
                                Source::Inline(code) => code.as_bytes().to_vec(),
                                Source::Remote(url) => match cached_source(root, song)? {
                                    Some(bytes) => bytes,
                                    None => fetch(url)?,
                                },
                            };
                            std::str::from_utf8(&bytes)
                                .map_err(|e| format!("{}: {e}", song.cached.source))?;
                            check_cancelled(cancelled)?;
                            let cached = CachedEntry {
                                entry: song.cached.entry.clone(),
                                source: song.cached.source.clone(),
                                sha256: format!("{:x}", Sha256::digest(&bytes)),
                                upstream: song.cached.upstream.clone(),
                            };
                            let path = root.join(&cached.entry.file);
                            atomic_write(&path, &bytes)?;
                            atomic_write(
                                &path.with_extension("json"),
                                &serde_json::to_vec_pretty(&cached).map_err(|e| e.to_string())?,
                            )?;
                            let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                            if count.is_multiple_of(100) || count == songs.len() {
                                progress(format!(
                                    "Downloaded {count}/{} upstream songs",
                                    songs.len()
                                ))?;
                            }
                            Ok(cached)
                        })();
                        match result {
                            Ok(cached) => results.push((index, cached)),
                            Err(error) => {
                                aborted.store(true, Ordering::Relaxed);
                                return Err(error);
                            }
                        }
                    }
                    Ok(results)
                }),
            );
        }
        let mut results = Vec::new();
        for handle in handles {
            results.extend(
                handle
                    .join()
                    .map_err(|_| "Catalog download worker panicked.")??,
            );
        }
        Ok::<_, String>(results)
    })?;
    check_cancelled(cancelled)?;
    if results.len() != songs.len() {
        return Err("Upstream download was incomplete; the cache index was not published.".into());
    }
    results.sort_by_key(|(index, _)| *index);
    let cached: Vec<_> = results.into_iter().map(|(_, song)| song).collect();
    let manifest = Manifest {
        version: 1,
        source: INDEX,
        songs: &cached,
    };
    let path = root.join("catalog.json");
    atomic_write(
        &path,
        &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )?;
    log::info!("Downloaded {} songs to {}", cached.len(), path.display());
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REMOTE_ID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn archive(songs: Value) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(
                &serde_json::to_vec(&json!([{"author":"Group author","songs":songs}])).unwrap(),
            )
            .unwrap();
        encoder.finish().unwrap()
    }

    fn fixture() -> Vec<u8> {
        archive(json!([
            {"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "name":"Inline", "sampleRate":8000, "code":"t"},
            {"hash":REMOTE_ID, "name":"Remote", "author":"Original author",
                "sampleRate":35498.66666, "mode":"Signed Bytebeat",
                "codeText":"incomplete preview", "fileOrig":1}
        ]))
    }

    #[test]
    fn downloads_complete_sources_and_preserves_metadata_and_order() {
        let root = tempfile::tempdir().unwrap();
        let compressed = fixture();
        let path = download_with(
            root.path(),
            &AtomicBool::new(false),
            &|url| match url {
                INDEX => Ok(compressed.clone()),
                LICENSE => Ok(b"upstream license".to_vec()),
                _ if url.ends_with(&format!("original/{REMOTE_ID}.js")) => Ok(b"t * 2;".to_vec()),
                _ => Err(format!("Unexpected URL: {url}")),
            },
            &|_| Ok(()),
        )
        .unwrap();
        let songs = crate::catalog::load_songs(&path).unwrap();
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].title, "Inline");
        assert_eq!(songs[0].author, "Group author");
        assert_eq!(songs[1].code, "t * 2;");
        assert_eq!(songs[1].author, "Original author");
        assert_eq!(songs[1].rate, 35498.66666);
        assert_eq!(songs[1].mode, Mode::Signed);
        assert_eq!(fs::read(root.path().join("all.gz")).unwrap(), compressed);
        assert_eq!(
            fs::read_to_string(root.path().join("UPSTREAM-LICENSE")).unwrap(),
            "upstream license"
        );
        let sidecar: Value = serde_json::from_str(
            &fs::read_to_string(root.path().join(format!("songs/{REMOTE_ID}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(sidecar["upstream"]["codeText"], "incomplete preview");
        assert_eq!(
            sidecar["sha256"],
            format!("{:x}", Sha256::digest(b"t * 2;"))
        );
    }

    #[test]
    fn resumes_verified_sources_but_refetches_corrupted_files() {
        let root = tempfile::tempdir().unwrap();
        let compressed = fixture();
        let remote_calls = AtomicUsize::new(0);
        let fetch = |url: &str| match url {
            INDEX => Ok(compressed.clone()),
            LICENSE => Ok(b"license".to_vec()),
            _ => {
                remote_calls.fetch_add(1, Ordering::Relaxed);
                Ok(b"t".to_vec())
            }
        };
        for _ in 0..2 {
            download_with(root.path(), &AtomicBool::new(false), &fetch, &|_| Ok(())).unwrap();
        }
        assert_eq!(remote_calls.load(Ordering::Relaxed), 1);
        fs::write(
            root.path().join(format!("songs/{REMOTE_ID}.js")),
            "corrupted",
        )
        .unwrap();
        download_with(root.path(), &AtomicBool::new(false), &fetch, &|_| Ok(())).unwrap();
        assert_eq!(remote_calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn failed_download_does_not_publish_a_partial_catalog() {
        let root = tempfile::tempdir().unwrap();
        let compressed = fixture();
        let error = download_with(
            root.path(),
            &AtomicBool::new(false),
            &|url| match url {
                INDEX => Ok(compressed.clone()),
                LICENSE => Ok(b"license".to_vec()),
                _ => Err("simulated network error".into()),
            },
            &|_| Ok(()),
        )
        .unwrap_err();
        assert!(error.contains("simulated network error"), "{error}");
        assert!(!root.path().join("catalog.json").exists());
    }

    #[test]
    fn rejects_previews_unsafe_ids_and_empty_or_invalid_catalogs() {
        let root = tempfile::tempdir().unwrap();
        for songs in [
            json!([{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sampleRate":8000, "codeText":"preview only"}]),
            json!([{"hash":"../unsafe", "sampleRate":8000, "code":"t"}]),
            json!([]),
        ] {
            let compressed = archive(songs);
            assert!(
                download_with(
                    root.path(),
                    &AtomicBool::new(false),
                    &|_| Ok(compressed.clone()),
                    &|_| Ok(())
                )
                .is_err()
            );
        }
        assert!(
            download_with(
                root.path(),
                &AtomicBool::new(false),
                &|_| Ok(b"not gzip".to_vec()),
                &|_| Ok(())
            )
            .is_err()
        );
        assert!(!root.path().join("catalog.json").exists());
    }

    #[test]
    fn cancellation_and_failed_progress_delivery_stop_downloads() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            download_with(
                root.path(),
                &AtomicBool::new(true),
                &|_| panic!("cancelled downloads must not fetch"),
                &|_| Ok(())
            )
            .is_err()
        );
        assert!(
            download_with(
                root.path(),
                &AtomicBool::new(false),
                &|_| panic!("disconnected GUI must not fetch"),
                &|_| Err("GUI closed".into())
            )
            .is_err()
        );
    }
}
