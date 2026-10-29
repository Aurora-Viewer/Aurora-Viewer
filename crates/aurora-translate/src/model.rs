//! The translation models the viewer offers, and their download.
//!
//! The files are the community Q4_K_M GGUF builds of Google's TranslateGemma
//! (Gemma terms of use), pinned by size and SHA-256 (the Hugging Face LFS
//! object id). A download goes to `<id>.gguf.part`, resumes from it, and is
//! renamed to `<id>.gguf` only once its checksum is right.

use crate::prompt::PromptKind;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub prompt: PromptKind,
}

pub const DEFAULT_MODEL: &str = "translategemma-4b-q4";

pub static CATALOGUE: &[ModelEntry] = &[
    // https://huggingface.co/mradermacher/translategemma-4b-it-GGUF
    ModelEntry {
        id: DEFAULT_MODEL,
        label: "TranslateGemma 4B",
        url: "https://huggingface.co/mradermacher/translategemma-4b-it-GGUF/resolve/main/translategemma-4b-it.Q4_K_M.gguf",
        bytes: 2_489_909_760,
        sha256: "81200d03e843d2ec1ece6eeafe7d13cb6e5211e1fcd336ade55790b683a08330",
        prompt: PromptKind::TranslateGemma,
    },
    // https://huggingface.co/mradermacher/translategemma-12b-it-GGUF
    ModelEntry {
        id: "translategemma-12b-q4",
        label: "TranslateGemma 12B",
        url: "https://huggingface.co/mradermacher/translategemma-12b-it-GGUF/resolve/main/translategemma-12b-it.Q4_K_M.gguf",
        bytes: 7_300_794_112,
        sha256: "b7aac4b4be7ab0c49b6556c29c4467e74313df7f1e95d9f9676bb2adf0afa528",
        prompt: PromptKind::TranslateGemma,
    },
];

pub fn entry(id: &str) -> Option<&'static ModelEntry> {
    CATALOGUE.iter().find(|e| e.id == id)
}

pub fn model_path(dir: &Path, e: &ModelEntry) -> PathBuf {
    dir.join(format!("{}.gguf", e.id))
}

fn part_path(dir: &Path, e: &ModelEntry) -> PathBuf {
    dir.join(format!("{}.gguf.part", e.id))
}

/// The model is on disk, complete (its checksum was checked before the
/// rename that gave it this name).
pub fn is_ready(dir: &Path, e: &ModelEntry) -> bool {
    std::fs::metadata(model_path(dir, e)).is_ok_and(|m| m.len() == e.bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    NoSpace { needed: u64 },
    Http(String),
    Io(String),
    Checksum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadState {
    Running { done: u64, total: u64 },
    Verifying,
    Ready(PathBuf),
    Failed(DownloadError),
    Cancelled,
}

/// A model download on its own thread.
pub struct Download {
    state: Arc<Mutex<DownloadState>>,
    cancel: Arc<AtomicBool>,
}

impl Download {
    pub fn start(dir: PathBuf, e: &'static ModelEntry) -> Download {
        let state = Arc::new(Mutex::new(DownloadState::Running { done: 0, total: e.bytes }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (st, c) = (state.clone(), cancel.clone());
        let spawned = std::thread::Builder::new().name("model-download".into()).spawn(move || {
            let end = match fetch(&dir, e, &st, &c) {
                Ok(path) => DownloadState::Ready(path),
                Err(_) if c.load(Ordering::Relaxed) => DownloadState::Cancelled,
                Err(err) => {
                    log::warn!("model download of {} failed: {err:?}", e.id);
                    DownloadState::Failed(err)
                }
            };
            set(&st, end);
        });
        if let Err(err) = spawned {
            set(&state, DownloadState::Failed(DownloadError::Io(err.to_string())));
        }
        Download { state, cancel }
    }

    pub fn progress(&self) -> DownloadState {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(DownloadState::Failed(DownloadError::Io("poisoned".into())))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn set(state: &Mutex<DownloadState>, s: DownloadState) {
    if let Ok(mut g) = state.lock() {
        *g = s;
    }
}

fn io(e: std::io::Error) -> DownloadError {
    DownloadError::Io(e.to_string())
}

fn fetch(dir: &Path, e: &ModelEntry, state: &Mutex<DownloadState>, cancel: &AtomicBool) -> Result<PathBuf, DownloadError> {
    std::fs::create_dir_all(dir).map_err(io)?;
    let part = part_path(dir, e);
    let have = std::fs::metadata(&part).map_or(0, |m| m.len()).min(e.bytes);
    match fs4::available_space(dir) {
        Ok(free) if free < e.bytes - have => return Err(DownloadError::NoSpace { needed: e.bytes - have }),
        Ok(_) => {}
        Err(err) => log::warn!("free space unknown ({err}), downloading anyway"),
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("Aurora Viewer translator ({})", env!("CARGO_PKG_NAME")))
        .build()
        .map_err(|err| DownloadError::Http(err.to_string()))?;
    let mut req = client.get(e.url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = req.send().map_err(|err| DownloadError::Http(err.to_string()))?;
    let status = resp.status();
    // a server that ignores the range sends the whole file again
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if !status.is_success() {
        return Err(DownloadError::Http(status.to_string()));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .map_err(io)?;
    let mut done = if resumed { have } else { 0 };
    let mut buf = vec![0u8; 1 << 16];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Io("cancelled".into()));
        }
        let n = resp.read(&mut buf).map_err(|err| DownloadError::Http(err.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(io)?;
        done += n as u64;
        if done > e.bytes {
            break;
        }
        set(state, DownloadState::Running { done, total: e.bytes });
    }
    file.flush().map_err(io)?;
    drop(file);
    if done != e.bytes {
        let _ = std::fs::remove_file(&part);
        return Err(DownloadError::Http(format!("{done} bytes received of {}", e.bytes)));
    }
    set(state, DownloadState::Verifying);
    if let Err(err) = verify(&part, e.sha256) {
        let _ = std::fs::remove_file(&part);
        return Err(err);
    }
    let path = model_path(dir, e);
    std::fs::rename(&part, &path).map_err(io)?;
    log::info!("translation model {} downloaded", e.id);
    Ok(path)
}

/// The file's SHA-256 is `sha256` (lowercase hex).
pub(crate) fn verify(path: &Path, sha256: &str) -> Result<(), DownloadError> {
    let mut f = File::open(path).map_err(io)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if hex == sha256.to_ascii_lowercase() {
        Ok(())
    } else {
        Err(DownloadError::Checksum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aurora-translate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp dir");
        d
    }

    fn small() -> ModelEntry {
        ModelEntry {
            id: "tiny",
            label: "Tiny",
            url: "http://127.0.0.1:1/tiny.gguf",
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            prompt: PromptKind::TranslateGemma,
        }
    }

    #[test]
    fn partial_file_is_not_ready() {
        let dir = temp("partial");
        let e = small();
        assert!(!is_ready(&dir, &e));
        std::fs::write(dir.join("tiny.gguf.part"), b"abc").expect("write");
        assert!(!is_ready(&dir, &e));
        std::fs::write(model_path(&dir, &e), b"ab").expect("write");
        assert!(!is_ready(&dir, &e));
        std::fs::write(model_path(&dir, &e), b"abc").expect("write");
        assert!(is_ready(&dir, &e));
    }

    #[test]
    fn verify_detects_a_bad_checksum() {
        let dir = temp("verify");
        let f = dir.join("f");
        std::fs::write(&f, b"abc").expect("write");
        assert!(verify(&f, small().sha256).is_ok());
        assert!(matches!(verify(&f, &"0".repeat(64)), Err(DownloadError::Checksum)));
    }

    #[test]
    fn catalogue_is_consistent() {
        assert!(entry(DEFAULT_MODEL).is_some());
        for (i, e) in CATALOGUE.iter().enumerate() {
            assert!(CATALOGUE[..i].iter().all(|o| o.id != e.id), "{}", e.id);
            assert_eq!(e.sha256.len(), 64);
            assert!(e.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(e.bytes > 0 && e.url.starts_with("https://"));
        }
    }

    #[test]
    fn unreachable_server_fails_without_panicking() {
        let dir = temp("fail");
        let e: &'static ModelEntry = Box::leak(Box::new(small()));
        let d = Download::start(dir, e);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match d.progress() {
                DownloadState::Failed(DownloadError::Http(_)) => break,
                DownloadState::Running { .. } if std::time::Instant::now() < end => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
