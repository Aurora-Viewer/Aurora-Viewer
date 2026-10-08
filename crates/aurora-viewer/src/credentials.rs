//! Remembered login password, like Firestorm's "Remember password": only the
//! `$1$` + md5 form the login sends is kept (never the password itself), in
//! the system credential store (Windows Credential Manager, protected by the
//! user's session) instead of Firestorm's bin_conf.dat file.

const SERVICE: &str = "Aurora Viewer";

/// Credential name: the grid and the user name (case-insensitive).
fn account(grid: &str, username: &str) -> String {
    format!("{}|{}", grid.trim(), username.trim().to_lowercase())
}

fn entry(grid: &str, username: &str) -> Option<keyring::Entry> {
    if username.trim().is_empty() {
        return None;
    }
    keyring::Entry::new(SERVICE, &account(grid, username))
        .map_err(|e| log::warn!("credential store unavailable: {e}"))
        .ok()
}

/// A login hash looks like `$1$` + 32 hex digits.
fn is_hash(s: &str) -> bool {
    s.len() == 35 && s.starts_with("$1$") && s[3..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// The remembered login hash of this user on this grid.
pub fn load(grid: &str, username: &str) -> Option<String> {
    let e = entry(grid, username)?;
    match e.get_password() {
        Ok(h) if is_hash(&h) => Some(h),
        Ok(_) => None,
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            log::warn!("remembered password unreadable: {err}");
            None
        }
    }
}

pub fn store(grid: &str, username: &str, hash: &str) {
    if !is_hash(hash) {
        return;
    }
    if let Some(e) = entry(grid, username)
        && let Err(err) = e.set_password(hash)
    {
        log::warn!("password not remembered: {err}");
    }
}

pub fn forget(grid: &str, username: &str) {
    if let Some(e) = entry(grid, username) {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(err) => log::warn!("remembered password not removed: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_login_hashes_are_kept() {
        assert!(is_hash("$1$098f6bcd4621d373cade4e832627b4f6"));
        assert!(!is_hash("hunter2"));
        assert!(!is_hash("$1$098f6bcd4621d373cade4e832627b4fZ"));
        assert_eq!(account(" Second Life ", " Prenom.Nom "), "Second Life|prenom.nom");
    }

    /// Real round trip through the system store (run by hand:
    /// `cargo test -p aurora-viewer credentials -- --ignored`).
    #[test]
    #[ignore]
    fn store_round_trip() {
        let (grid, user) = ("test-grid", "aurora-selftest");
        let h = "$1$00000000000000000000000000000000";
        store(grid, user, h);
        assert_eq!(load(grid, user).as_deref(), Some(h));
        forget(grid, user);
        assert_eq!(load(grid, user), None);
    }
}
