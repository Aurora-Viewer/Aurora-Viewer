//! Firestorm LLPanelLogin::populateUserList / onUserListCommit: independent
//! credentials per account and grid. Only non-secret catalogue data is here;
//! hashes remain in the OS credential store. Merge changes under an OS lock.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastPlace {
    pub region: String,
    pub image: uuid::Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub username: String,
    pub uri: String,
    pub remember_password: bool,
    #[serde(default)]
    pub remember_mfa: bool,
    #[serde(default = "last_location")]
    pub start_location: String,
    #[serde(default)]
    pub start_region: String,
    #[serde(default)]
    pub agent: Option<uuid::Uuid>,
    #[serde(default)]
    pub last_place: Option<LastPlace>,
}

impl Account {
    pub fn from_settings(settings: &crate::settings::Settings, remember_mfa: bool, agent: Option<uuid::Uuid>) -> Self {
        Self {
            username: settings.username.trim().to_owned(),
            uri: settings.login_uri(),
            remember_password: settings.remember_password,
            remember_mfa,
            start_location: settings.start_location.clone(),
            start_region: settings.start_region.clone(),
            agent,
            last_place: None,
        }
    }

    pub fn apply(&self, settings: &mut crate::settings::Settings) {
        settings.grid = grid(&self.uri);
        if settings.grid == crate::settings::GridChoice::Custom {
            settings.custom_login_uri = self.uri.clone();
        }
        settings.start_location = self.start_location.clone();
        settings.start_region = self.start_region.clone();
    }

    pub fn key(&self) -> (String, String) {
        (self.uri.trim().to_owned(), self.username.trim().to_lowercase())
    }
}

fn last_location() -> String {
    "last".into()
}

pub fn grid(uri: &str) -> crate::settings::GridChoice {
    use crate::settings::GridChoice;
    match uri.trim() {
        aurora_net::login::SL_MAIN_GRID => GridChoice::SecondLife,
        aurora_net::login::SL_BETA_GRID => GridChoice::SecondLifeBeta,
        _ => GridChoice::Custom,
    }
}

/// The catalogue, and whether something in it was left out because this
/// build does not understand it (an account saved by a newer Aurora).
fn read(path: &Path) -> std::io::Result<(Vec<Account>, bool)> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => crate::tolerant::from_slice::<Vec<Account>>(&bytes, "login_accounts.json")
            .map(|read| {
                let repaired = read.repaired();
                (read.value, repaired)
            })
            .ok_or_else(|| std::io::Error::other("account catalogue unreadable")),
        Ok(_) => Err(std::io::Error::other("account catalogue too large")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((Vec::new(), false)),
        Err(error) => Err(error),
    }
}

pub fn load() -> Vec<Account> {
    match read(&crate::settings::config_dir().join("login_accounts.json")) {
        Ok((accounts, _)) => accounts,
        Err(error) => {
            log::warn!("account catalogue unreadable: {error}");
            Vec::new()
        }
    }
}

fn change_at(path: &Path, update: impl FnOnce(&mut Vec<Account>)) -> std::io::Result<Vec<Account>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let (mut accounts, repaired) = read(path)?;
    if repaired {
        // Rewriting would drop what a newer build saved there (as an
        // unreadable catalogue always was): shown, never overwritten.
        return Err(std::io::Error::other(
            "account catalogue holds entries from a newer Aurora, not rewritten",
        ));
    }
    update(&mut accounts);
    let bytes = serde_json::to_vec_pretty(&accounts).map_err(std::io::Error::other)?;
    crate::atomic_file::write(path, &bytes)?;
    Ok(accounts)
}

pub fn save(account: Account) -> Vec<Account> {
    replace(&account.key(), account)
}

/// A card can switch grids without deleting credentials for its previous grid.
pub fn replace(previous: &(String, String), account: Account) -> Vec<Account> {
    let result = change_at(&crate::settings::config_dir().join("login_accounts.json"), |accounts| {
        upsert(accounts, previous, account);
    });
    result.unwrap_or_else(|error| {
        log::warn!("account catalogue not saved: {error}");
        load()
    })
}

fn upsert(accounts: &mut Vec<Account>, previous: &(String, String), mut account: Account) {
    let destination = account.key();
    if account.last_place.is_none()
        && let Some(old) = accounts.iter().find(|old| old.key() == destination && old.agent == account.agent)
    {
        account.last_place = old.last_place.clone();
    }
    let position = accounts.iter().position(|old| old.key() == *previous || old.key() == destination);
    accounts.retain(|old| old.key() != *previous && old.key() != destination);
    accounts.insert(position.unwrap_or(accounts.len()).min(accounts.len()), account);
}

/// A scene capture changes only this account's last-place metadata.
pub fn remember_place(account: &Account, place: LastPlace) -> bool {
    change_at(&crate::settings::config_dir().join("login_accounts.json"), |accounts| {
        if let Some(current) = accounts
            .iter_mut()
            .find(|current| current.key() == account.key() && current.agent == account.agent)
        {
            current.last_place = Some(place.clone());
        }
    })
    .map(|accounts| {
        accounts
            .iter()
            .any(|current| current.key() == account.key() && current.last_place.as_ref() == Some(&place))
    })
    .unwrap_or_else(|_| {
        log::warn!("last-place metadata not saved");
        false
    })
}

pub fn remove(account: &Account) -> Vec<Account> {
    change_at(&crate::settings::config_dir().join("login_accounts.json"), |accounts| {
        accounts.retain(|old| old.key() != account.key())
    })
    .unwrap_or_else(|error| {
        log::warn!("account not removed: {error}");
        load()
    })
}

/// Move identities in the latest catalogue without replacing any profile data.
pub fn move_relative(from: &(String, String), target: &(String, String), after: bool) -> std::io::Result<Vec<Account>> {
    change_at(&crate::settings::config_dir().join("login_accounts.json"), |accounts| {
        move_in(accounts, from, target, after);
    })
}

pub fn move_in(accounts: &mut Vec<Account>, from: &(String, String), target: &(String, String), after: bool) -> bool {
    if from == target {
        return false;
    }
    let Some(source) = accounts.iter().position(|account| account.key() == *from) else {
        return false;
    };
    let Some(destination) = accounts.iter().position(|account| account.key() == *target) else {
        return false;
    };
    let destination = destination + usize::from(after) - usize::from(source < destination + usize::from(after));
    if source == destination {
        return false;
    }
    let account = accounts.remove(source);
    accounts.insert(destination, account);
    true
}

/// Update only the chosen preference in the latest catalogue, under its lock.
pub fn forget_preference(account: &Account, mfa: bool) -> Vec<Account> {
    change_at(&crate::settings::config_dir().join("login_accounts.json"), |accounts| {
        if let Some(current) = accounts.iter_mut().find(|current| current.key() == account.key()) {
            if mfa {
                current.remember_mfa = false;
            } else {
                current.remember_password = false;
            }
        }
    })
    .unwrap_or_else(|error| {
        log::warn!("account preference not forgotten: {error}");
        load()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_scoped_to_grid_identity_and_preserves_hidden_profiles() {
        let mut alpha = Account::from_settings(&crate::settings::Settings::default(), true, None);
        alpha.username = "synthetic-alpha".into();
        let mut other_grid = alpha.clone();
        other_grid.uri = "synthetic-other-grid".into();
        let mut hidden = alpha.clone();
        hidden.username = "synthetic-hidden".into();
        let mut beta = alpha.clone();
        beta.username = "synthetic-beta".into();
        let mut accounts = vec![alpha.clone(), hidden.clone(), other_grid.clone(), beta.clone()];
        assert!(move_in(&mut accounts, &beta.key(), &other_grid.key(), false));
        assert_eq!(
            accounts.iter().map(Account::key).collect::<Vec<_>>(),
            vec![alpha.key(), hidden.key(), beta.key(), other_grid.key()]
        );
        assert!(move_in(&mut accounts, &alpha.key(), &other_grid.key(), true));
        assert_eq!(
            accounts.iter().map(Account::key).collect::<Vec<_>>(),
            vec![hidden.key(), beta.key(), other_grid.key(), alpha.key()]
        );
        assert!(!move_in(&mut accounts, &alpha.key(), &alpha.key(), true));
        assert!(!move_in(
            &mut accounts,
            &alpha.key(),
            &("missing".into(), "synthetic".into()),
            false
        ));
    }

    #[test]
    fn an_account_from_a_newer_build_is_shown_and_never_overwritten() {
        let root = std::env::temp_dir().join(format!("aurora-newer-accounts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("accounts.json");
        let file = serde_json::json!([
            { "username": "synthetic-kept", "uri": "synthetic-grid", "remember_password": false, "a_newer_field": 1 },
            { "username": "synthetic-newer", "uri": { "a newer kind": "of grid" }, "remember_password": true },
        ]);
        let bytes = serde_json::to_vec(&file).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let (accounts, repaired) = read(&path).expect("catalogue");
        assert!(repaired);
        assert_eq!(accounts.iter().map(|a| a.username.as_str()).collect::<Vec<_>>(), ["synthetic-kept"]);
        // a change would drop the newer account: refused, the file untouched
        assert!(change_at(&path, |accounts| accounts.clear()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_order_uses_latest_metadata_and_keeps_concurrent_additions() {
        let root = std::env::temp_dir().join(format!("aurora-order-{}", uuid::Uuid::new_v4()));
        let path = root.join("accounts.json");
        let mut alpha = Account::from_settings(&crate::settings::Settings::default(), true, None);
        alpha.username = "synthetic-alpha".into();
        let mut beta = alpha.clone();
        beta.username = "synthetic-beta".into();
        change_at(&path, |accounts| accounts.extend([alpha.clone(), beta.clone()])).unwrap();
        let mut gamma = beta.clone();
        gamma.username = "synthetic-gamma".into();
        change_at(&path, |accounts| {
            accounts[1].start_region = "Synthetic updated region".into();
            accounts[0].remember_password = false;
            accounts.push(gamma.clone());
        })
        .unwrap();
        change_at(&path, |accounts| {
            move_in(accounts, &alpha.key(), &beta.key(), true);
        })
        .unwrap();
        let (saved, _) = read(&path).unwrap();
        assert_eq!(
            saved.iter().map(Account::key).collect::<Vec<_>>(),
            vec![beta.key(), alpha.key(), gamma.key()]
        );
        assert_eq!(saved[0].start_region, "Synthetic updated region");
        assert!(!saved[1].remember_password);
        assert!(saved[1].remember_mfa);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_catalogue_defaults_and_grid_changes_preserve_other_profiles() {
        let old = r#"{"username":"synthetic-alpha","uri":"test-grid","remember_password":true}"#;
        let alpha: Account = serde_json::from_str(old).expect("synthetic old profile");
        assert_eq!(alpha.start_location, "last");
        assert!(!alpha.remember_mfa);
        assert!(alpha.last_place.is_none());
        let mut beta = alpha.clone();
        beta.username = "synthetic-beta".into();
        beta.start_location = "home".into();
        let mut accounts = vec![alpha.clone(), beta];
        let mut moved = alpha.clone();
        moved.uri = aurora_net::login::SL_BETA_GRID.into();
        moved.start_location = "region".into();
        moved.start_region = "Synthetic region".into();
        upsert(&mut accounts, &alpha.key(), moved);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[1].start_location, "home");
        let mut settings = crate::settings::Settings::default();
        accounts[0].apply(&mut settings);
        assert_eq!(settings.grid, crate::settings::GridChoice::SecondLifeBeta);
        assert_eq!(settings.start_region, "Synthetic region");
    }
    #[test]
    fn successful_login_preserves_each_accounts_last_place() {
        let mut alpha = Account::from_settings(&crate::settings::Settings::default(), false, Some(uuid::Uuid::from_u128(1)));
        alpha.username = "synthetic-alpha".into();
        alpha.last_place = Some(LastPlace {
            region: "Synthetic coast".into(),
            image: uuid::Uuid::from_u128(2),
        });
        let mut beta = alpha.clone();
        beta.username = "synthetic-beta".into();
        beta.agent = Some(uuid::Uuid::from_u128(3));
        beta.last_place = Some(LastPlace {
            region: "Synthetic forest".into(),
            image: uuid::Uuid::from_u128(4),
        });
        let mut accounts = vec![alpha.clone(), beta.clone()];
        let mut login = alpha.clone();
        login.last_place = None;
        login.remember_password = true;
        upsert(&mut accounts, &alpha.key(), login.clone());
        assert_eq!(accounts[0].last_place, alpha.last_place);
        assert!(accounts[0].remember_password);
        assert_eq!(accounts[1].last_place, beta.last_place);
        login.agent = Some(uuid::Uuid::from_u128(5));
        upsert(&mut accounts, &alpha.key(), login);
        assert!(accounts[0].last_place.is_none());
    }
    #[test]
    fn concurrent_catalogue_changes_keep_every_account() {
        let root = std::env::temp_dir().join(format!("aurora-accounts-{}", uuid::Uuid::new_v4()));
        let path = root.join("accounts.json");
        std::thread::scope(|scope| {
            for i in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    change_at(path, |accounts| {
                        accounts.push(Account {
                            username: format!("synthetic-{i}"),
                            uri: "test-grid".into(),
                            remember_password: i % 2 == 0,
                            remember_mfa: false,
                            start_location: "last".into(),
                            start_region: String::new(),
                            agent: None,
                            last_place: None,
                        })
                    })
                    .expect("catalogue update")
                });
            }
        });
        let (accounts, _) = read(&path).expect("catalogue");
        assert_eq!(accounts.len(), 8);
        change_at(&path, |accounts| accounts.retain(|a| a.username != "synthetic-0")).expect("remove one");
        let (remaining, _) = read(&path).expect("catalogue");
        assert_eq!(remaining.len(), 7);
        assert_eq!(remaining.iter().filter(|a| a.remember_password).count(), 3);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
