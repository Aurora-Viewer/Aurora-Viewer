//! Remembered login password, like Firestorm's "Remember password": only the
//! `$1$` + md5 form the login sends is kept (never the password itself), in
//! the system credential store (Windows Credential Manager, protected by the
//! user's session) instead of Firestorm's bin_conf.dat file.

const SERVICE: &str = "Aurora Viewer";
const MFA_SERVICE: &str = "Aurora Viewer MFA";
const MFA_LIFETIME: u64 = 30 * 24 * 60 * 60;

/// Firestorm LLLoginInstance::saveMFAHash: separate protected map per user/grid.
/// The authorization is not an OTP and never belongs in the settings JSON.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct MfaAuthorization {
    pub hash: String,
    expires_at: u64,
}

impl MfaAuthorization {
    pub fn renewed(hash: String, previous: Option<&Self>, now: u64) -> Self {
        Self {
            hash,
            // Reusing an authorization must not renew its 30-day lifetime.
            expires_at: previous.map_or_else(|| now.saturating_add(MFA_LIFETIME), |old| old.expires_at),
        }
    }

    pub fn valid_at(&self, now: u64) -> bool {
        !self.hash.is_empty() && self.hash.len() <= 16 * 1024 && now < self.expires_at && self.expires_at - now <= MFA_LIFETIME
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs())
}

/// Credential name: the grid and the user name (case-insensitive).
fn account(grid: &str, username: &str) -> String {
    format!("{}|{}", grid.trim(), username.trim().to_lowercase())
}

fn entry(grid: &str, username: &str) -> Option<keyring::Entry> {
    service_entry(SERVICE, grid, username)
}

fn service_entry(service: &str, grid: &str, username: &str) -> Option<keyring::Entry> {
    if aurora_updater::test_environment().is_some() {
        return None;
    }
    if username.trim().is_empty() {
        return None;
    }
    keyring::Entry::new(service, &account(grid, username))
        .map_err(|e| log::warn!("credential store unavailable: {e}"))
        .ok()
}

pub fn load_mfa(grid: &str, username: &str) -> Option<MfaAuthorization> {
    load_mfa_at(&SystemStore, (grid, username), now())
}

pub fn store_mfa(grid: &str, username: &str, authorization: &MfaAuthorization) {
    save_mfa_at(&SystemStore, (grid, username), authorization, now());
}

pub fn forget_mfa(grid: &str, username: &str) {
    SystemStore.remove(grid, username);
}

/// A one-use choice removes only this account's authorization after success.
pub fn complete_mfa(
    grid: &str,
    username: &str,
    remember: bool,
    hash: Option<&str>,
    previous: Option<&MfaAuthorization>,
) -> Option<MfaAuthorization> {
    complete_mfa_at(&SystemStore, (grid, username), remember, hash, previous, now())
}

trait MfaStore {
    fn read(&self, grid: &str, username: &str) -> Option<String>;
    fn write(&self, grid: &str, username: &str, value: &str);
    fn remove(&self, grid: &str, username: &str);
}

struct SystemStore;

impl MfaStore for SystemStore {
    fn read(&self, grid: &str, username: &str) -> Option<String> {
        let entry = service_entry(MFA_SERVICE, grid, username)?;
        match entry.get_password() {
            Ok(value) => Some(value),
            Err(keyring::Error::NoEntry) => None,
            Err(_) => {
                log::warn!("remembered MFA authorization unreadable");
                None
            }
        }
    }

    fn write(&self, grid: &str, username: &str, value: &str) {
        if let Some(entry) = service_entry(MFA_SERVICE, grid, username)
            && entry.set_password(value).is_err()
        {
            log::warn!("MFA authorization not remembered");
        }
    }

    fn remove(&self, grid: &str, username: &str) {
        if let Some(entry) = service_entry(MFA_SERVICE, grid, username) {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => log::warn!("MFA authorization not removed"),
            }
        }
    }
}

fn load_mfa_at(store: &impl MfaStore, (grid, username): (&str, &str), now: u64) -> Option<MfaAuthorization> {
    let value = store.read(grid, username)?;
    if value.len() > 32 * 1024 {
        return None;
    }
    let authorization = serde_json::from_str::<MfaAuthorization>(&value).ok()?;
    authorization.valid_at(now).then_some(authorization)
}

fn save_mfa_at(store: &impl MfaStore, (grid, username): (&str, &str), authorization: &MfaAuthorization, now: u64) {
    if authorization.valid_at(now)
        && let Ok(value) = serde_json::to_string(authorization)
    {
        store.write(grid, username, &value);
    }
}

fn complete_mfa_at(
    store: &impl MfaStore,
    user: (&str, &str),
    remember: bool,
    hash: Option<&str>,
    previous: Option<&MfaAuthorization>,
    now: u64,
) -> Option<MfaAuthorization> {
    if !remember {
        store.remove(user.0, user.1);
        return None;
    }
    let Some(hash) = hash.filter(|hash| !hash.is_empty()) else {
        return previous.filter(|authorization| authorization.valid_at(now)).cloned();
    };
    let authorization = MfaAuthorization::renewed(hash.to_owned(), previous, now);
    save_mfa_at(store, user, &authorization, now);
    authorization.valid_at(now).then_some(authorization)
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

    #[derive(Default)]
    struct MemoryStore(std::cell::RefCell<std::collections::HashMap<String, String>>);

    impl MfaStore for MemoryStore {
        fn read(&self, grid: &str, username: &str) -> Option<String> {
            self.0.borrow().get(&account(grid, username)).cloned()
        }
        fn write(&self, grid: &str, username: &str, value: &str) {
            self.0.borrow_mut().insert(account(grid, username), value.to_owned());
        }
        fn remove(&self, grid: &str, username: &str) {
            self.0.borrow_mut().remove(&account(grid, username));
        }
    }

    #[test]
    fn mfa_survives_account_switches_and_one_use_affects_only_that_account() {
        let store = MemoryStore::default();
        let now = 1_000_000;
        let users = [
            ("test-grid-a", "synthetic-alpha"),
            ("test-grid-a", "synthetic-beta"),
            ("test-grid-b", "synthetic-alpha"),
        ];
        for (index, user) in users.iter().enumerate() {
            complete_mfa_at(&store, *user, true, Some(&format!("synthetic-{index}")), None, now);
        }
        for (index, user) in users.iter().enumerate().rev() {
            assert_eq!(
                load_mfa_at(&store, *user, now + 1).expect("synthetic authorization").hash,
                format!("synthetic-{index}")
            );
        }
        assert!(complete_mfa_at(&store, users[1], false, Some("synthetic-one-use"), None, now).is_none());
        assert!(load_mfa_at(&store, users[1], now).is_none());
        assert!(load_mfa_at(&store, users[0], now).is_some());
        assert!(load_mfa_at(&store, users[2], now).is_some());
        assert!(load_mfa_at(&store, users[0], now + MFA_LIFETIME).is_none());
    }

    #[test]
    fn mfa_expiry_does_not_slide_on_password_only_login() {
        let issued = 1_000_000;
        let first = MfaAuthorization::renewed("synthetic-mfa".into(), None, issued);
        let later = MfaAuthorization::renewed("synthetic-rotation".into(), Some(&first), issued + 500);
        assert!(later.valid_at(issued + MFA_LIFETIME - 1));
        assert!(!later.valid_at(issued + MFA_LIFETIME));
        let fresh = MfaAuthorization::renewed("synthetic-fresh".into(), None, issued + MFA_LIFETIME);
        assert!(fresh.valid_at(issued + MFA_LIFETIME));
        assert!(!fresh.valid_at(issued + 2 * MFA_LIFETIME));
        assert_ne!(account("test-grid-a", "synthetic-user"), account("test-grid-b", "synthetic-user"));
        assert_ne!(account("test-grid-a", "synthetic-user"), account("test-grid-a", "synthetic-other"));
    }

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
