//! The secret a Blender add-on must send before the preview accepts it.
//!
//! An Aurora addition (the Legacy Viewer has no live link): the loopback port
//! alone let any local program, another user's included, push meshes while
//! the preview was on. Each start of the preview draws a new random secret
//! and writes it to `<data folder>/blender-live/<port>.token`, a folder of the
//! user's own profile that other accounts cannot read; the add-on, which runs
//! as the same user, reads it there before it connects. The file is removed
//! when the preview stops.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Hex characters of a secret, as sent on the wire after the magic.
pub(super) const LEN: usize = 64;

/// The folder of the token files, the add-on's `token_path` in
/// blender/aurora_live/client.py.
pub(super) fn folder() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer").map(|d| d.data_local_dir().join("blender-live"))
}

pub(super) fn file_name(port: u16) -> String {
    format!("{port}.token")
}

/// A fresh secret: two version 4 UUIDs, 244 bits from the system's random
/// source, written as 64 lowercase hex characters.
pub(super) fn generate() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Compares in constant time, so the reply time tells nothing of the secret.
pub(super) fn matches(received: &[u8], secret: &str) -> bool {
    received.len() == secret.len() && received.iter().zip(secret.as_bytes()).fold(0, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The token file of a running preview; dropping it removes the file.
pub(super) struct File {
    path: PathBuf,
    secret: String,
}

impl File {
    pub(super) fn write(folder: &Path, port: u16) -> io::Result<Self> {
        fs::create_dir_all(folder)?;
        let path = folder.join(file_name(port));
        let secret = generate();
        // Written beside it then renamed, so the add-on never reads half a
        // secret.
        let partial = folder.join(format!("{}.partial", file_name(port)));
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Owner only: the profile folder may be readable by others.
            options.mode(0o600);
        }
        {
            use io::Write;
            let mut file = options.open(&partial)?;
            file.write_all(secret.as_bytes())?;
            file.sync_all()?;
        }
        if let Err(error) = fs::rename(&partial, &path) {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        Ok(Self { path, secret })
    }

    pub(super) fn secret(&self) -> &str {
        &self.secret
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // Another viewer may have started on this port since: only our own
        // secret is removed.
        if fs::read(&self.path).is_ok_and(|content| content == self.secret.as_bytes())
            && let Err(error) = fs::remove_file(&self.path)
        {
            log::warn!("blender live: could not remove the token file: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_long_hex_and_differ() {
        let (a, b) = (generate(), generate());
        assert_eq!(a.len(), LEN);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    #[test]
    fn only_the_exact_secret_matches() {
        let secret = generate();
        assert!(matches(secret.as_bytes(), &secret));
        let mut other = secret.clone().into_bytes();
        other[LEN - 1] = if other[LEN - 1] == b'0' { b'1' } else { b'0' };
        assert!(!matches(&other, &secret));
        assert!(!matches(&secret.as_bytes()[..LEN - 1], &secret));
        assert!(!matches(b"", &secret));
    }

    #[test]
    fn the_file_holds_the_secret_and_goes_away_with_the_preview() {
        let folder = std::env::temp_dir().join(format!("aurora-live-token-{}", uuid::Uuid::new_v4().simple()));
        let file = File::write(&folder, 17655).expect("token file");
        let path = folder.join("17655.token");
        assert_eq!(fs::read_to_string(&path).expect("read"), file.secret());
        assert!(!folder.join("17655.token.partial").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).expect("metadata").permissions().mode() & 0o777, 0o600);
        }
        drop(file);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_later_viewer_keeps_its_own_file() {
        let folder = std::env::temp_dir().join(format!("aurora-live-token-{}", uuid::Uuid::new_v4().simple()));
        let first = File::write(&folder, 17656).expect("first");
        let second = File::write(&folder, 17656).expect("second");
        drop(first);
        let path = folder.join("17656.token");
        assert_eq!(fs::read_to_string(&path).expect("kept"), second.secret());
        drop(second);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&folder);
    }
}
