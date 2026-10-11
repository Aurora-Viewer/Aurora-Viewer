//! Verify the published SHA-256 before extracting into an empty sibling.
//! ZIP names are validated for Windows even when the tests run elsewhere.

use crate::{
    Cancel, Failure, Progress,
    feed::{Release, Source},
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub async fn download(
    source: &Source,
    release: &Release,
    work: &Path,
    cancel: &Cancel,
    progress: impl Fn(Progress) + Send + 'static,
) -> std::result::Result<PathBuf, Failure> {
    cancel.check().map_err(|_| Failure::Network)?;
    let client = source.client().map_err(|e| failure(Failure::Network, e))?;
    let archive = release.archive.as_ref().ok_or(Failure::Unsupported)?;
    let checksum = release.checksum.as_ref().ok_or(Failure::Unsupported)?;
    let expected = source
        .bytes(&client, &checksum.browser_download_url, 1024)
        .await
        .map_err(|e| failure(Failure::Network, e))?;
    let expected = parse_checksum(&expected, &archive.name).map_err(|e| failure(Failure::Integrity, e))?;
    source
        .validate_url(&archive.browser_download_url)
        .map_err(|e| failure(Failure::Network, e))?;
    let mut response = client
        .get(&archive.browser_download_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| failure(Failure::Network, e.into()))?;
    let path = work.join("download.zip");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| failure(Failure::Filesystem, e.into()))?;
    let mut hash = Sha256::new();
    let mut received = 0_u64;
    loop {
        cancel.check().map_err(|_| Failure::Network)?;
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|e| failure(Failure::Network, e.into()))?,
            _ = cancel.cancelled() => return Err(Failure::Network),
        };
        let Some(chunk) = chunk else { break };
        received = received.saturating_add(chunk.len() as u64);
        if received > archive.size {
            return Err(Failure::Integrity);
        }
        file.write_all(&chunk).map_err(|e| failure(Failure::Filesystem, e.into()))?;
        hash.update(&chunk);
        progress(Progress {
            received,
            total: archive.size,
            preparing: false,
        });
    }
    if received != archive.size || hash.finalize().as_slice() != expected {
        return Err(Failure::Integrity);
    }
    file.sync_all().map_err(|e| failure(Failure::Filesystem, e.into()))?;
    drop(file);
    progress(Progress {
        received,
        total: archive.size,
        preparing: true,
    });
    let root = work.join("prepared");
    let cancel = cancel.clone();
    let tag = release.tag.clone();
    tokio::task::spawn_blocking(move || extract(&path, &root, &tag, &cancel))
        .await
        .map_err(|e| failure(Failure::Package, e.into()))?
        .map_err(|e| failure(Failure::Package, e))
}

fn failure(kind: Failure, error: anyhow::Error) -> Failure {
    log::warn!("update {kind:?}: {error:#}");
    kind
}

fn parse_checksum(bytes: &[u8], filename: &str) -> Result<[u8; 32]> {
    let text = std::str::from_utf8(bytes)?;
    let fields: Vec<&str> = text.split_whitespace().collect();
    ensure!(
        fields.len() == 2
            && fields[0].len() == 64
            && fields[0].bytes().all(|b| b.is_ascii_hexdigit())
            && fields[1].trim_start_matches('*') == filename,
        "invalid checksum sidecar"
    );
    let mut hash = [0_u8; 32];
    for (i, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&fields[0][i * 2..i * 2 + 2], 16).context("invalid SHA-256")?;
    }
    Ok(hash)
}

fn components(name: &str) -> Result<Vec<&str>> {
    ensure!(
        !name.starts_with('/') && !name.contains('\\') && !name.contains('\0'),
        "unsafe ZIP path"
    );
    let mut parts = Vec::new();
    for part in name.trim_end_matches('/').split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.contains([':', '<', '>', '"', '|', '?', '*'])
                && !part.chars().any(char::is_control),
            "unsafe Windows path"
        );
        let stem = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
                && !(stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "reserved Windows filename"
        );
        parts.push(part);
    }
    Ok(parts)
}

pub fn extract(path: &Path, root: &Path, tag: &str, cancel: &Cancel) -> Result<PathBuf> {
    ensure!(!root.exists(), "staging directory already exists");
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    ensure!(!zip.is_empty() && zip.len() <= 50_000, "invalid ZIP entry count");
    let prefix = format!("Aurora-Viewer-{tag}-windows-x64");
    let mut names = HashSet::new();
    let mut expanded = 0_u64;
    // Validate every entry before any extraction, including case collisions.
    for i in 0..zip.len() {
        cancel.check()?;
        let entry = zip.by_index(i)?;
        let name = entry.name()?;
        let parts = components(&name)?;
        ensure!(
            parts[0] == prefix
                && !entry.is_symlink()
                && entry
                    .unix_mode()
                    .is_none_or(|m| m & 0o170000 == 0 || matches!(m & 0o170000, 0o100000 | 0o040000)),
            "invalid package root or link"
        );
        ensure!(names.insert(name.trim_end_matches('/').to_lowercase()), "duplicate ZIP path");
        expanded = expanded.saturating_add(entry.size());
        ensure!(expanded <= 4 * 1024 * 1024 * 1024, "expanded package too large");
    }
    fs::create_dir(root)?;
    for i in 0..zip.len() {
        cancel.check()?;
        let mut entry = zip.by_index(i)?;
        let name = entry.name()?.into_owned();
        let parts = components(&name)?;
        let target = parts.iter().skip(1).fold(root.to_path_buf(), |p, name| p.join(name));
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        ensure!(parts.len() > 1, "package root is a file");
        fs::create_dir_all(target.parent().context("ZIP file parent")?)?;
        let mut out = OpenOptions::new().write(true).create_new(true).open(target)?;
        let mut buffer = [0_u8; 64 * 1024];
        let mut size = 0_u64;
        loop {
            cancel.check()?;
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            ensure!(size <= entry.size(), "ZIP size mismatch");
            out.write_all(&buffer[..n])?;
        }
        ensure!(size == entry.size(), "truncated ZIP member");
        out.sync_all()?;
    }
    let manifest: InstallManifest = serde_json::from_slice(&fs::read(root.join("aurora-install.json"))?)?;
    ensure!(
        manifest.schema == 1 && manifest.version == tag.trim_start_matches('v'),
        "package version mismatch"
    );
    ensure!(
        root.join("aurora-viewer.exe").is_file() && root.join("assets").is_dir() && root.join("LICENSE").is_file(),
        "incomplete viewer package"
    );
    Ok(root.to_path_buf())
}

#[derive(serde::Deserialize)]
pub struct InstallManifest {
    pub schema: u32,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksum_known_vector_and_filename() {
        let value = b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad zip.zip\n";
        assert_eq!(parse_checksum(value, "zip.zip").expect("hash"), Sha256::digest(b"abc").as_slice());
        assert!(parse_checksum(value, "other.zip").is_err());
        for text in [
            "bad zip.zip",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff zip.zip extra",
            "éééééééééééééééééééééééééééééééé zip.zip",
        ] {
            assert!(parse_checksum(text.as_bytes(), "zip.zip").is_err());
        }
    }
    #[test]
    fn windows_archive_paths() {
        assert!(components("Aurora/assets/character/a.llm").is_ok());
        for value in [
            "../a",
            "/a",
            "C:/a",
            "a/../b",
            "a\\b",
            "a/file:stream",
            "a/NUL.txt",
            "a/COM1",
            "a/../",
            "a/name.",
            "a//b",
        ] {
            assert!(components(value).is_err(), "{value}");
        }
    }
}
