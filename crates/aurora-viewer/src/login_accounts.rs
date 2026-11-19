//! Firestorm LLPanelLogin::populateUserList / onUserListCommit: independent
//! credentials per account and grid. Only non-secret catalogue data is here;
//! hashes remain in the OS credential store. Merge changes under an OS lock.
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

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

/// The catalogue as the disk has it, with the changes still waiting for
/// a write.
pub fn load() -> Vec<Account> {
    let catalogue = catalogue();
    // the login screen's changes still on their way reach the disk first
    crate::saves::wait_for(&catalogue.path, crate::saves::BEFORE_READ);
    match read(&catalogue.path) {
        Ok((mut accounts, _)) => {
            catalogue.owed.lock().iter().for_each(|c| c.apply(&mut accounts));
            accounts
        }
        Err(error) => {
            log::warn!("account catalogue unreadable: {error}");
            Vec::new()
        }
    }
}

/// Why a change of the catalogue is not on the disk.
#[derive(Debug)]
enum Failure {
    /// The catalogue cannot be rewritten: unreadable, or holding accounts
    /// of a newer build. The change is dropped, as it always was.
    Refused(std::io::Error),
    /// The write was refused (the file held by another program, a folder
    /// that refuses writes, a full disk): `accounts` is the list the change
    /// makes, to be written again.
    Kept { accounts: Vec<Account>, error: std::io::Error },
}

fn change_at(path: &Path, update: impl FnOnce(&mut Vec<Account>)) -> Result<Vec<Account>, Failure> {
    // the lock, held to the write; when it cannot be taken the catalogue is
    // still read, for the list the change makes
    let lock = (|| {
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
        Ok::<_, std::io::Error>(lock)
    })();
    let (mut accounts, repaired) = read(path).map_err(Failure::Refused)?;
    if repaired {
        // Rewriting would drop what a newer build saved there (as an
        // unreadable catalogue always was): shown, never overwritten.
        return Err(Failure::Refused(std::io::Error::other(
            "account catalogue holds entries from a newer Aurora, not rewritten",
        )));
    }
    update(&mut accounts);
    let written = match &lock {
        Ok(_) => serde_json::to_vec_pretty(&accounts)
            .map_err(std::io::Error::other)
            .and_then(|bytes| crate::atomic_file::write(path, &bytes)),
        Err(error) => Err(std::io::Error::new(error.kind(), error.to_string())),
    };
    drop(lock);
    match written {
        Ok(()) => Ok(accounts),
        Err(error) => Err(Failure::Kept { accounts, error }),
    }
}

/// A change of the catalogue, kept to be made again on the latest file
/// while its write is refused.
#[derive(Clone, Debug)]
enum Change {
    Upsert {
        previous: (String, String),
        account: Account,
    },
    Remove((String, String)),
    Forget {
        key: (String, String),
        mfa: bool,
    },
    Move {
        from: (String, String),
        target: (String, String),
        after: bool,
    },
    Place {
        account: Account,
        place: LastPlace,
    },
}

impl Change {
    fn apply(&self, accounts: &mut Vec<Account>) {
        match self {
            Change::Upsert { previous, account } => upsert(accounts, previous, account.clone()),
            Change::Remove(key) => accounts.retain(|old| old.key() != *key),
            Change::Forget { key, mfa } => {
                if let Some(current) = accounts.iter_mut().find(|current| current.key() == *key) {
                    if *mfa {
                        current.remember_mfa = false;
                    } else {
                        current.remember_password = false;
                    }
                }
            }
            Change::Move { from, target, after } => {
                move_in(accounts, from, target, *after);
            }
            Change::Place { account, place } => {
                if let Some(current) = accounts
                    .iter_mut()
                    .find(|current| current.key() == account.key() && current.agent == account.agent)
                {
                    current.last_place = Some(place.clone());
                }
            }
        }
    }
}

/// The catalogue file and the changes its writes could not save yet. Each
/// change is made on the latest file after the ones still owed, in their
/// order, until a write takes them all: a refused edit is neither dropped
/// nor undone by the copy on the disk, and another Aurora's changes made
/// meanwhile are kept. The Legacy Viewer keeps its saved logins in memory
/// and writes them once (LLSecAPIBasicHandler::_writeProtectedData,
/// indra/newview/llsechandler_basic.cpp); here they are also tried again
/// (`save_retry`) and once more at exit (`save_before_exit`).
struct Catalogue {
    path: std::path::PathBuf,
    owed: parking_lot::Mutex<Vec<Change>>,
}

impl Catalogue {
    fn new(path: std::path::PathBuf) -> Self {
        Catalogue {
            path,
            owed: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// The list `change` makes, saved now or kept to be saved: true when it
    /// still waits for a write. An error when the catalogue cannot be
    /// rewritten; the changes owed are dropped then.
    fn change(&self, change: Change) -> Result<(Vec<Account>, bool), std::io::Error> {
        let mut owed = self.owed.lock();
        owed.push(change);
        match change_at(&self.path, |accounts| owed.iter().for_each(|c| c.apply(accounts))) {
            Ok(accounts) => {
                owed.clear();
                Ok((accounts, false))
            }
            Err(Failure::Kept { accounts, error }) => {
                log::warn!("account catalogue not saved ({error}); kept, tried again later");
                Ok((accounts, true))
            }
            Err(Failure::Refused(error)) => {
                owed.clear();
                Err(error)
            }
        }
    }

    /// Writes what is owed (the retry, the exit): an error while it stays
    /// owed.
    fn flush(&self) -> std::io::Result<()> {
        let mut owed = self.owed.lock();
        if owed.is_empty() {
            return Ok(());
        }
        match change_at(&self.path, |accounts| owed.iter().for_each(|c| c.apply(accounts))) {
            Ok(_) => {
                owed.clear();
                Ok(())
            }
            Err(Failure::Kept { error, .. }) => Err(error),
            Err(Failure::Refused(error)) => {
                log::warn!("account catalogue changes dropped: {error}");
                owed.clear();
                Ok(())
            }
        }
    }
}

/// The user's catalogue.
fn catalogue() -> &'static Catalogue {
    static CATALOGUE: std::sync::OnceLock<Catalogue> = std::sync::OnceLock::new();
    CATALOGUE.get_or_init(|| Catalogue::new(crate::settings::config_dir().join("login_accounts.json")))
}

/// The thread that tries again what the user's catalogue owes.
static RETRY: crate::save_retry::OnDemand = crate::save_retry::OnDemand::new("aurora-login-accounts", "account catalogue");

/// A change of the user's catalogue: the list it makes, saved now or later.
fn change(change: Change) -> Result<Vec<Account>, std::io::Error> {
    let (accounts, owed) = catalogue().change(change)?;
    if owed {
        RETRY.ask(|| catalogue().flush());
    }
    Ok(accounts)
}

/// The viewer closing: what the catalogue still owes is tried once more,
/// and waited for.
pub fn save_before_exit() {
    RETRY.finish();
}

/// The catalogue's file: the key of its saves' queue (saves.rs).
pub(crate) fn catalogue_path() -> std::path::PathBuf {
    catalogue().path.clone()
}

/// The changes asked from the login screen: each is made at once on the
/// list it shows, and in the catalogue by a save thread (saves.rs), in
/// their order: the frame no longer waits for the disk. A refused write
/// keeps the change owed ([`Catalogue::change`]). `ISSUED` numbers them,
/// and `ANSWER` keeps what the last one left in the catalogue (with what
/// another viewer changed meanwhile), for [`answer`].
static ISSUED: AtomicU64 = AtomicU64::new(0);
static ANSWER: Mutex<Option<(u64, Answer)>> = Mutex::new(None);

/// What the catalogue said of the login screen's last change.
#[derive(Debug)]
pub enum Answer {
    /// The catalogue as it is now, the changes still owed included.
    Saved(Vec<Account>),
    /// Dropped, the catalogue cannot be rewritten (logged); `reorder` when
    /// it was a move of the order.
    Failed { reorder: bool },
}

/// `edit` made on `accounts` now, and in the catalogue by its save thread.
fn queue(what: &'static str, accounts: &mut Vec<Account>, edit: Change) {
    edit.apply(accounts);
    let reorder = matches!(edit, Change::Move { .. });
    let number = ISSUED.fetch_add(1, Ordering::Relaxed) + 1;
    crate::saves::change(catalogue_path(), move || {
        let answer = match change(edit) {
            Ok(accounts) => Answer::Saved(accounts),
            Err(error) => {
                log::warn!("{what}: {error}");
                Answer::Failed { reorder }
            }
        };
        *ANSWER.lock().unwrap_or_else(PoisonError::into_inner) = Some((number, answer));
    });
}

/// The catalogue's answer to the last change asked, once it is made; none
/// while a later change is still on its way (its own answer follows).
pub fn answer() -> Option<Answer> {
    let mut answer = ANSWER.lock().unwrap_or_else(PoisonError::into_inner);
    match answer.take() {
        Some((number, answer)) if number == ISSUED.load(Ordering::Relaxed) => Some(answer),
        _ => None,
    }
}

pub fn save(accounts: &mut Vec<Account>, account: Account) {
    replace(accounts, &account.key(), account);
}

/// A card can switch grids without deleting credentials for its previous grid.
pub fn replace(accounts: &mut Vec<Account>, previous: &(String, String), account: Account) {
    let previous = previous.clone();
    queue("account catalogue not saved", accounts, Change::Upsert { previous, account });
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
    change(Change::Place {
        account: account.clone(),
        place: place.clone(),
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

pub fn remove(accounts: &mut Vec<Account>, account: &Account) {
    queue("account not removed", accounts, Change::Remove(account.key()));
}

/// Move identities in the latest catalogue without replacing any profile
/// data. A refused write keeps the new order and tries again.
pub fn move_relative(accounts: &mut Vec<Account>, from: &(String, String), target: &(String, String), after: bool) {
    let (from, target) = (from.clone(), target.clone());
    queue("account order not saved", accounts, Change::Move { from, target, after });
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
pub fn forget_preference(accounts: &mut Vec<Account>, account: &Account, mfa: bool) {
    queue(
        "account preference not forgotten",
        accounts,
        Change::Forget { key: account.key(), mfa },
    );
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
    fn synthetic(name: &str) -> Account {
        let mut account = Account::from_settings(&crate::settings::Settings::default(), true, None);
        account.username = name.into();
        account
    }

    /// A refused write (here a folder stands where the lock file goes: the
    /// catalogue reads, the write cannot be made) keeps the edit: the list
    /// returned has it, a later edit adds to it, and the next write puts
    /// both on the disk.
    #[test]
    fn a_refused_write_keeps_the_edit_until_a_write_takes_it() {
        let root = std::env::temp_dir().join(format!("aurora-accounts-refused-{}", uuid::Uuid::new_v4()));
        let path = root.join("accounts.json");
        let alpha = synthetic("synthetic-alpha");
        let beta = synthetic("synthetic-beta");
        change_at(&path, |accounts| accounts.extend([alpha.clone(), beta.clone()])).unwrap();
        let catalogue = Catalogue::new(path.clone());
        std::fs::remove_file(path.with_extension("lock")).unwrap();
        std::fs::create_dir(path.with_extension("lock")).unwrap();
        let (accounts, owed) = catalogue
            .change(Change::Forget {
                key: alpha.key(),
                mfa: false,
            })
            .unwrap();
        assert!(owed, "the write was refused");
        assert!(!accounts[0].remember_password, "the list shows the edit");
        let (accounts, owed) = catalogue.change(Change::Remove(beta.key())).unwrap();
        assert!(owed);
        assert_eq!(accounts.len(), 1);
        assert!(!accounts[0].remember_password, "the first edit is still there");
        assert!(catalogue.flush().is_err(), "still refused");
        assert_eq!(catalogue.owed.lock().len(), 2, "nothing dropped");
        let (on_disk, _) = read(&path).unwrap();
        assert_eq!(on_disk.len(), 2, "the disk still has the old list");
        std::fs::remove_dir(path.with_extension("lock")).unwrap();
        catalogue.flush().unwrap();
        assert!(catalogue.owed.lock().is_empty());
        let (saved, _) = read(&path).unwrap();
        assert_eq!(saved.iter().map(Account::key).collect::<Vec<_>>(), vec![alpha.key()]);
        assert!(!saved[0].remember_password);
        std::fs::remove_dir_all(root).unwrap();
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
