//! The ffmpeg the Director records its films with, as a pack of the
//! project: the same scheme as the media plugins (`media_pack.rs`). A viewer
//! is built for one pack of its system, named here with its size and
//! SHA-256; the archive is downloaded only once the user agreed (with its
//! size shown first), kept only when it is that exact archive, checked again
//! on the disk before it is unpacked, and its folder gets its final name
//! once complete. Nothing in it runs before that: the viewer only asks the
//! ffmpeg found in a complete folder what it encodes.
//!
//! Aurora's own: the Legacy Viewer records no film.

use crate::{
    Cancel, Failure, Progress,
    feed::Source,
    install::validate_tree,
    package::{Fetch, failure, parse_sha256, unpack},
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

/// A pack as this viewer names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FfmpegPack {
    /// A whole number: a higher one replaces the lower ones.
    pub version: &'static str,
    /// Bytes of the archive.
    pub size: u64,
    /// The archive's SHA-256, in hexadecimal.
    pub sha256: &'static str,
}

// ---------------------------------------------------------------------------
// The pack this viewer downloads: pack 1, published on 2026-11-18
// (https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/ffmpeg-1).
// An empty entry (every field "" or 0) offers no download (`published` is
// `None`). The "FFmpeg pack" workflow
// (.github/workflows/ffmpeg-pack.yml, scripts/ffmpeg-pack.sh) builds the
// archives and publishes them under the release tag `ffmpeg-<version>` of
// this repository, with their sizes and SHA-256 in its notes, as
// `Aurora-FFmpeg-<version>-<system>.zip` (system: windows-x64, linux-x64,
// macos-universal), holding one folder of that name with
// `aurora-ffmpeg.json` (`{"schema": 1, "version": "<version>"}`), the
// program in `bin/` and its licence files. Never invent these values: they
// are the published archive's own.
// ---------------------------------------------------------------------------
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const PACK: FfmpegPack = FfmpegPack {
    version: "1",
    size: 15_218_154,
    sha256: "65bb1b3fd6f4c5293c9c1f0192ddd59ced30414ef8e03220cc074203355d819f",
};
#[cfg(target_os = "linux")]
pub const PACK: FfmpegPack = FfmpegPack {
    version: "1",
    size: 15_676_239,
    sha256: "4cb6829592810f521dcefd7f49919ecbb157bd9d38abb67ad30c385d86e44605",
};
#[cfg(target_os = "macos")]
pub const PACK: FfmpegPack = FfmpegPack {
    version: "1",
    size: 26_238_094,
    sha256: "947e39e9bad5f56e9b4229dc378d916f6f5550b6df246525f5518a6987d7bc18",
};

/// No ffmpeg pack is larger than this: a size above it is a mistake.
pub const MAX_SIZE: u64 = 512 * 1024 * 1024;

/// The system part of the archive's name.
const SYSTEM: &str = if cfg!(target_os = "linux") {
    "linux-x64"
} else if cfg!(target_os = "macos") {
    "macos-universal"
} else {
    "windows-x64"
};
const MANIFEST: &str = "aurora-ffmpeg.json";
/// Written next to the manifest once the archive is extracted: its SHA-256.
const ORIGIN: &str = "archive.sha256";
const PROGRAM: &str = if cfg!(windows) { "bin/ffmpeg.exe" } else { "bin/ffmpeg" };

/// A pack whose fields are filled in and sound: the one to download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Published {
    pub version: u32,
    pub size: u64,
    pub sha256: [u8; 32],
    hex: &'static str,
}

/// The pack, when its fields name one: a version that is a whole number
/// above 0, a size from 1 byte to `MAX_SIZE`, a SHA-256 of 64 hexadecimal
/// digits. `None` for the empty entry of a viewer built before the pack is
/// published (or a field filled in wrong).
pub fn published(pack: &FfmpegPack) -> Option<Published> {
    let version = pack.version.parse::<u32>().ok().filter(|v| *v > 0)?;
    if pack.size == 0 || pack.size > MAX_SIZE {
        return None;
    }
    let sha256 = parse_sha256(pack.sha256).ok()?;
    Some(Published {
        version,
        size: pack.size,
        sha256,
        hex: pack.sha256,
    })
}

#[derive(serde::Deserialize)]
struct Manifest {
    schema: u32,
    version: String,
}

/// What a pack's `aurora-ffmpeg.json` says: its version, when it reads as
/// a manifest of schema 1.
pub fn manifest_version(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(bytes).ok()?;
    (manifest.schema == 1).then_some(manifest.version)
}

impl Published {
    fn tag(&self) -> String {
        format!("ffmpeg-{}", self.version)
    }

    fn folder(&self) -> String {
        format!("Aurora-FFmpeg-{}-{SYSTEM}", self.version)
    }

    /// The folder holds this pack, whole.
    fn complete(&self, dir: &Path) -> bool {
        fs::read(dir.join(MANIFEST))
            .ok()
            .and_then(|bytes| manifest_version(&bytes))
            .is_some_and(|version| version == self.version.to_string())
            && fs::read_to_string(dir.join(ORIGIN)).is_ok_and(|hash| hash.trim().eq_ignore_ascii_case(self.hex))
            && dir.join(PROGRAM).is_file()
    }
}

/// The ffmpeg program of this pack, when it is installed under `root`.
pub fn installed(root: &Path, pack: &Published) -> Option<PathBuf> {
    let dir = root.join(pack.version.to_string());
    pack.complete(&dir).then(|| dir.join(PROGRAM))
}

/// The file at `path` is the archive: exactly `size` bytes whose SHA-256 is
/// `sha256`. Read on a blocking thread (it may weigh a hundred MB).
pub fn verify_file(path: &Path, size: u64, sha256: &[u8; 32]) -> std::result::Result<(), Failure> {
    let mut file = File::open(path).map_err(|e| failure(Failure::Filesystem, e.into()))?;
    let length = file.metadata().map_err(|e| failure(Failure::Filesystem, e.into()))?.len();
    if length != size {
        return Err(failure(
            Failure::Integrity,
            anyhow::anyhow!("ffmpeg pack: {length} bytes, {size} expected"),
        ));
    }
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| failure(Failure::Filesystem, e.into()))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if hash.finalize().as_slice() != sha256 {
        return Err(failure(Failure::Integrity, anyhow::anyhow!("ffmpeg pack: SHA-256 mismatch")));
    }
    Ok(())
}

/// Two viewers started together download the pack once: the second is told
/// `Busy`.
fn lock(root: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".lock"))?;
    file.try_lock().context("another viewer is installing ffmpeg")?;
    Ok(file)
}

/// Download and extraction happen here; dropped, it leaves nothing behind.
struct Stage(PathBuf);

impl Drop for Stage {
    fn drop(&mut self) {
        let path = std::mem::take(&mut self.0);
        std::thread::spawn(move || {
            if validate_tree(&path).is_ok()
                && let Err(e) = fs::remove_dir_all(&path)
            {
                log::warn!("ffmpeg pack staging cleanup: {e}");
            }
        });
    }
}

/// Removes what an interrupted viewer left, and the older packs (with the
/// lock held: nothing else installs meanwhile).
fn prune(root: &Path, current: u32, stage: &Path) {
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let (name, path) = (entry.file_name().to_string_lossy().into_owned(), entry.path());
        let leftover = name.starts_with(".staging-");
        let older = name.parse::<u32>().is_ok_and(|v| v < current);
        if path.is_dir() && path != stage && (leftover || older) && validate_tree(&path).is_ok() {
            // (an older ffmpeg still filming keeps its folder on Windows)
            if let Err(e) = fs::remove_dir_all(&path) {
                log::warn!("ffmpeg pack cleanup of {name}: {e}");
            }
        }
    }
}

/// Install `pack` under `root`: the path of its ffmpeg program.
pub async fn install(
    source: &Source,
    pack: &Published,
    root: &Path,
    cancel: &Cancel,
    progress: impl Fn(Progress) + Send + Sync + 'static,
) -> std::result::Result<PathBuf, Failure> {
    cancel.check().map_err(|_| Failure::Network)?;
    fs::create_dir_all(root).map_err(|e| failure(Failure::Filesystem, e.into()))?;
    let lock = lock(root).map_err(|e| failure(Failure::Busy, e))?;
    if let Some(program) = installed(root, pack) {
        return Ok(program);
    }
    let stage = root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage).map_err(|e| failure(Failure::Filesystem, e.into()))?;
    let stage = Stage(stage);
    let folder = pack.folder();
    let url = source
        .asset_url(&pack.tag(), &format!("{folder}.zip"))
        .map_err(|e| failure(Failure::Network, e))?;
    let client = source.client().map_err(|e| failure(Failure::Network, e))?;
    let archive = stage.0.join("download.zip");
    let fetch = Fetch {
        url: &url,
        size: pack.size,
        sha256: pack.sha256,
    };
    fetch.to_file(source, &client, &archive, cancel, &progress).await?;
    let (pack, root, cancel) = (*pack, root.to_path_buf(), cancel.clone());
    tokio::task::spawn_blocking(move || -> std::result::Result<PathBuf, Failure> {
        // the file unpacked is the one checked, whatever touched the disk
        verify_file(&archive, pack.size, &pack.sha256)?;
        let prepare = || -> Result<PathBuf> {
            let prepared = stage.0.join("prepared");
            unpack(&archive, &prepared, &folder, &cancel)?;
            fs::write(prepared.join(ORIGIN), pack.hex)?;
            ensure!(pack.complete(&prepared), "incomplete ffmpeg pack");
            let dir = root.join(pack.version.to_string());
            if dir.exists() {
                // another build of the same version, or a damaged folder
                validate_tree(&dir)?;
                fs::remove_dir_all(&dir).context("the ffmpeg to replace is in use")?;
            }
            fs::rename(&prepared, &dir).context("cannot put ffmpeg in place")?;
            prune(&root, pack.version, &stage.0);
            drop(lock);
            Ok(dir.join(PROGRAM))
        };
        prepare().map_err(|e| failure(Failure::Package, e))
    })
    .await
    .map_err(|e| failure(Failure::Package, e.into()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn temp() -> PathBuf {
        let root = std::env::temp_dir().join(format!("aurora-ffmpeg-pack-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("root");
        root
    }

    #[test]
    fn a_pack_is_named_only_by_sound_fields() {
        // the entry of this build: pack 1, published
        let own = published(&PACK).expect("this viewer names pack 1");
        assert_eq!(own.version, 1);
        assert!(own.size > 1_000_000 && own.size <= MAX_SIZE);
        assert_eq!(own.tag(), "ffmpeg-1");
        // an empty entry names nothing: no download offered
        let empty = FfmpegPack {
            version: "",
            size: 0,
            sha256: "",
        };
        assert_eq!(published(&empty), None);
        let good = FfmpegPack {
            version: "1",
            size: 80_000_000,
            sha256: HASH,
        };
        let pack = published(&good).expect("a published pack");
        assert_eq!((pack.version, pack.size), (1, 80_000_000));
        assert_eq!(pack.tag(), "ffmpeg-1");
        assert!(pack.folder().starts_with("Aurora-FFmpeg-1-"));
        // a field filled in wrong names nothing
        for bad in [
            FfmpegPack { version: "", ..good },
            FfmpegPack { version: "0", ..good },
            FfmpegPack { version: "1.2", ..good },
            FfmpegPack { size: 0, ..good },
            FfmpegPack {
                size: MAX_SIZE + 1,
                ..good
            },
            FfmpegPack {
                sha256: "e3b0c442",
                ..good
            },
            FfmpegPack {
                sha256: "zz0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                ..good
            },
        ] {
            assert_eq!(published(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_manifest_reads_as_schema_1_only() {
        assert_eq!(manifest_version(br#"{"schema":1,"version":"3"}"#).as_deref(), Some("3"));
        assert_eq!(manifest_version(br#"{"schema":2,"version":"3"}"#), None);
        assert_eq!(manifest_version(br#"{"version":"3"}"#), None);
        assert_eq!(manifest_version(b"not json"), None);
        assert_eq!(manifest_version(&vec![b' '; 70 * 1024]), None, "too large to be one");
    }

    #[test]
    fn an_archive_of_another_size_or_hash_is_refused() {
        let root = temp();
        let file = root.join("download.zip");
        // the empty file's SHA-256 is HASH
        fs::write(&file, b"").expect("fixture");
        let hash = parse_sha256(HASH).expect("hash");
        assert!(verify_file(&file, 0, &hash).is_ok());
        assert!(matches!(verify_file(&file, 1, &hash), Err(Failure::Integrity)), "size checked");
        fs::write(&file, b"x").expect("fixture");
        assert!(matches!(verify_file(&file, 1, &hash), Err(Failure::Integrity)), "hash checked");
        assert!(matches!(verify_file(&root.join("none"), 0, &hash), Err(Failure::Filesystem)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pack_is_found_only_when_whole() {
        let root = temp();
        let pack = published(&FfmpegPack {
            version: "2",
            size: 1,
            sha256: HASH,
        })
        .expect("pack");
        let dir = root.join("2");
        fs::create_dir_all(dir.join("bin")).expect("fixture");
        assert_eq!(installed(&root, &pack), None, "no manifest");
        fs::write(dir.join(MANIFEST), br#"{"schema":1,"version":"2"}"#).expect("fixture");
        fs::write(dir.join(PROGRAM), b"").expect("fixture");
        assert_eq!(installed(&root, &pack), None, "no origin: not known to be this archive");
        fs::write(dir.join(ORIGIN), HASH).expect("fixture");
        assert_eq!(installed(&root, &pack), Some(dir.join(PROGRAM)));
        fs::write(dir.join(ORIGIN), "0".repeat(64)).expect("fixture");
        assert_eq!(installed(&root, &pack), None, "another build of the version");
        // the older packs and an interrupted download's leftovers go
        fs::create_dir_all(root.join("1/bin")).expect("fixture");
        fs::create_dir_all(root.join(".staging-old")).expect("fixture");
        prune(&root, 2, &root.join(".staging-now"));
        assert!(!root.join("1").exists() && !root.join(".staging-old").exists());
        assert!(dir.is_dir());
        let _ = fs::remove_dir_all(&root);
    }
}
