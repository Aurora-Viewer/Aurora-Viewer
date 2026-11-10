//! The favourite landmarks of each account, kept for the login screen's
//! start menu: the inventory is not there before login, so the session
//! writes them down. Port of LLFavoritesOrderStorage::saveFavoritesRecord,
//! removeFavoritesRecordOfUser and showFavoritesOnLoginChanged
//! (indra/newview/llfavoritesbar.cpp) and of
//! FSPanelLogin::addFavoritesToStartLocation (indra/newview/fspanellogin.cpp),
//! originally LGPL 2.1. The Legacy Viewer keeps one `stored_favorites.xml`
//! keyed by "user @ grid" with each favourite's name and SLURL; here
//! `stored_favorites.json` holds one record per grid and account (the key of
//! `login_accounts.rs`) with the item, its name, its region and the position
//! in it, and the per-account "Show my favorite landmarks at login"
//! (ShowFavoritesOnLogin, settings_per_account.xml, on by default). Off, the
//! record keeps only that choice (removeFavoritesRecordOfUser). Writes go to
//! a thread of their own and merge into the latest file under an OS lock, so
//! another Aurora's accounts are kept.

use aurora_net::StartLocation;
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use uuid::Uuid;

pub const FILE: &str = "stored_favorites.json";
/// A file past this size is not read (a few hundred favourites take a few
/// tens of KB).
const MAX_BYTES: usize = 1024 * 1024;

/// One favourite as the login screen offers it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Favorite {
    /// The landmark's inventory item: the choice survives a rename.
    pub item: Uuid,
    /// Its landmark asset: while it stays the same, the stored place
    /// serves the next sessions without asking the landmark again.
    #[serde(default)]
    pub asset: Uuid,
    pub name: String,
    pub region: String,
    /// Position in the region, in metres.
    pub pos: [f32; 3],
}

impl Favorite {
    /// The login's start, the form of a typed location
    /// (LLSLURL::getLoginString: `uri:Region&x&y&z`, rounded).
    pub fn start_location(&self) -> StartLocation {
        let metre = |v: f32| if v.is_finite() { v.round().max(0.0) as u32 } else { 0 };
        StartLocation::Region {
            name: self.region.clone(),
            x: metre(self.pos[0]),
            y: metre(self.pos[1]),
            z: metre(self.pos[2]),
        }
    }
}

/// A grid and an account (`login_accounts::Account::key`).
pub type Key = (String, String);

pub fn key(uri: &str, username: &str) -> Key {
    (uri.trim().to_owned(), username.trim().to_lowercase())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    uri: String,
    username: String,
    #[serde(default = "shown_by_default")]
    shown: bool,
    #[serde(default)]
    favorites: Vec<Favorite>,
}

impl Record {
    fn key(&self) -> Key {
        key(&self.uri, &self.username)
    }
}

/// ShowFavoritesOnLogin's default.
fn shown_by_default() -> bool {
    true
}

enum Change {
    Set(Record),
    Remove(Key),
}

fn read(path: &Path) -> std::io::Result<Vec<Record>> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= MAX_BYTES => {
            crate::disk_io::read(crate::disk_io::Origin::Other, bytes.len());
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)
        }
        Ok(_) => Err(std::io::Error::other("stored favourites too large")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn apply(records: &mut Vec<Record>, change: Change) {
    match change {
        Change::Set(record) => {
            let key = record.key();
            match records.iter_mut().find(|r| r.key() == key) {
                Some(old) => *old = record,
                None => records.push(record),
            }
        }
        Change::Remove(key) => records.retain(|r| r.key() != key),
    }
}

/// The latest file, these changes merged in, written back under its lock.
fn write_changes(path: &Path, changes: Vec<Change>) -> std::io::Result<()> {
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
    // an unreadable file is replaced rather than blocking every save
    let mut records = read(path).unwrap_or_else(|error| {
        log::warn!("stored favourites unreadable, written again: {error}");
        Vec::new()
    });
    for change in changes {
        apply(&mut records, change);
    }
    let bytes = serde_json::to_vec_pretty(&records).map_err(std::io::Error::other)?;
    crate::atomic_file::write(path, &bytes)
}

struct Writer {
    sender: Option<Sender<Change>>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    fn start(path: PathBuf) -> Option<Self> {
        let (sender, changes): (Sender<Change>, Receiver<Change>) = crossbeam_channel::unbounded();
        let worker = std::thread::Builder::new().name("aurora-login-favorites".into()).spawn(move || {
            while let Ok(first) = changes.recv() {
                let mut batch = vec![first];
                batch.extend(changes.try_iter());
                if let Err(error) = write_changes(&path, batch) {
                    log::warn!("stored favourites not saved: {error}");
                }
            }
        });
        match worker {
            Ok(worker) => Some(Self {
                sender: Some(sender),
                worker: Some(worker),
            }),
            Err(error) => {
                log::warn!("Cannot start the stored favourites' writer: {error}");
                None
            }
        }
    }
}

impl Drop for Writer {
    /// The last changes reach the disk before the viewer closes.
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Every account's stored favourites. Without a file (the demo, the tests
/// that do not open one) it lives in memory only.
#[derive(Default)]
pub struct Store {
    records: Vec<Record>,
    path: Option<PathBuf>,
    writer: Option<Writer>,
}

impl Store {
    /// Read (again) the file: a small catalogue, read with the account
    /// catalogue on the login screen.
    pub fn load(&mut self, path: PathBuf) {
        self.records = read(&path).unwrap_or_else(|error| {
            log::warn!("stored favourites unreadable: {error}");
            Vec::new()
        });
        if self.path.as_ref() != Some(&path) {
            self.writer = None;
            self.path = Some(path);
        }
    }

    /// The file, read unless it already is (a login that skipped the
    /// login screen's catalogue).
    pub fn open(&mut self, path: PathBuf) {
        if self.path.is_none() {
            self.load(path);
        }
    }

    /// A store in memory with these favourites (the demo).
    pub fn seeded(entries: impl IntoIterator<Item = (Key, Vec<Favorite>)>) -> Self {
        Self {
            records: entries
                .into_iter()
                .map(|((uri, username), favorites)| Record {
                    uri,
                    username,
                    shown: true,
                    favorites,
                })
                .collect(),
            ..Self::default()
        }
    }

    fn record(&self, key: &Key) -> Option<&Record> {
        self.records.iter().find(|r| r.key() == *key)
    }

    /// The account's favourites for the start menu: none for an account
    /// never logged in here, or that keeps them off.
    pub fn favorites(&self, key: &Key) -> &[Favorite] {
        self.record(key).filter(|r| r.shown).map_or(&[], |r| r.favorites.as_slice())
    }

    /// This landmark of the account is stored with its place already.
    pub fn knows(&self, key: &Key, item: Uuid, asset: Uuid) -> bool {
        self.favorites(key).iter().any(|f| f.item == item && f.asset == asset)
    }

    /// The account's "Show my favorite landmarks at login".
    pub fn shown(&self, key: &Key) -> bool {
        self.record(key).is_none_or(|r| r.shown)
    }

    /// showFavoritesOnLoginChanged: off, the favourites are removed and
    /// only the choice stays; on, the session writes them again.
    pub fn set_shown(&mut self, key: &Key, shown: bool) {
        if self.shown(key) == shown && self.record(key).is_some() {
            return;
        }
        let favorites = if shown {
            self.record(key).map(|r| r.favorites.clone()).unwrap_or_default()
        } else {
            Vec::new()
        };
        self.set(Record {
            uri: key.0.clone(),
            username: key.1.clone(),
            shown,
            favorites,
        });
    }

    /// saveFavoritesRecord: the session's favourites, written when they
    /// differ from the stored ones. Nothing while the account keeps them off.
    pub fn update(&mut self, key: &Key, favorites: Vec<Favorite>) -> bool {
        if !self.shown(key) || self.record(key).is_some_and(|r| r.favorites == favorites) {
            return false;
        }
        self.set(Record {
            uri: key.0.clone(),
            username: key.1.clone(),
            shown: true,
            favorites,
        });
        true
    }

    /// The account left the catalogue (LLFloaterForgetUser): its record goes
    /// with it.
    pub fn forget(&mut self, key: &Key) {
        if self.record(key).is_none() {
            return;
        }
        apply(&mut self.records, Change::Remove(key.clone()));
        self.send(Change::Remove(key.clone()));
    }

    fn set(&mut self, record: Record) {
        apply(&mut self.records, Change::Set(record.clone()));
        self.send(Change::Set(record));
    }

    fn send(&mut self, change: Change) {
        let Some(path) = &self.path else {
            return;
        };
        if self.writer.is_none() {
            self.writer = Writer::start(path.clone());
        }
        if let Some(sender) = self.writer.as_ref().and_then(|w| w.sender.as_ref()) {
            let _ = sender.send(change);
        }
    }

    /// Wait for the writes sent so far (the viewer closing, the tests); the
    /// next change starts the writer again.
    pub fn flush(&mut self) {
        self.writer = None;
    }
}

/// One favourite of the session: its item, its name now, and where it
/// lies when its landmark and its region are known this session.
pub struct Current {
    pub item: Uuid,
    pub asset: Uuid,
    pub name: String,
    pub place: Option<(String, [f32; 3])>,
}

/// The favourites to store, in the session's order: a favourite whose
/// landmark or region is not known yet this session keeps its stored
/// place (with its name of now) while its landmark stays the same, so a
/// login does not empty the menu while the landmarks load; one never placed
/// is left out (the Legacy Viewer stores it with an empty SLURL, which the
/// login panel skips).
pub fn merge(stored: &[Favorite], current: Vec<Current>) -> Vec<Favorite> {
    current
        .into_iter()
        .filter_map(|c| {
            let (region, pos) = match c.place {
                Some(place) => place,
                None => stored
                    .iter()
                    .find(|f| f.item == c.item && f.asset == c.asset)
                    .map(|f| (f.region.clone(), f.pos))?,
            };
            Some(Favorite {
                item: c.item,
                asset: c.asset,
                name: c.name,
                region,
                pos,
            })
        })
        .collect()
}

/// AURORA_DEMO_LOGIN_FAVORITES=1|open|pick, with AURORA_DEMO_LOGIN=cards:
/// stored favourites for the second (selected) and third demo accounts (the first
/// was never logged in here: Home and Last only); `open` opens the
/// selected card's start menu once, `pick` chooses its second favourite
/// (Log in then writes the start URI to the log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demo {
    Shown,
    Open,
    Pick,
}

pub fn demo() -> Option<Demo> {
    static DEMO: std::sync::OnceLock<Option<Demo>> = std::sync::OnceLock::new();
    *DEMO.get_or_init(|| {
        let value = std::env::var("AURORA_DEMO_LOGIN_FAVORITES").ok()?;
        match value.trim() {
            "" | "0" | "off" => None,
            "open" => Some(Demo::Open),
            "pick" => Some(Demo::Pick),
            _ => Some(Demo::Shown),
        }
    })
}

/// The demo's stored favourites (made-up places of the demo map), for the
/// demo accounts named "Example n" on `uri`.
pub fn demo_store(uri: &str) -> Store {
    use aurora_i18n::tr;
    let fav = |n: u128, name: &str, region: &str, pos: [f32; 3]| Favorite {
        item: Uuid::from_u128(0x10f0_0000 + n),
        asset: Uuid::from_u128(0x10f1_0000 + n),
        name: name.to_owned(),
        region: region.to_owned(),
        pos,
    };
    Store::seeded([
        (
            key(uri, "Example 2"),
            vec![
                fav(1, tr!("common-aurora-plaza"), tr!("common-aurora-demo"), [128.0, 128.0, 25.0]),
                fav(2, tr!("demo-place-aurora-lagoon"), tr!("common-boreal-lagoon"), [60.0, 200.0, 22.0]),
                fav(3, tr!("common-pine-grove"), tr!("common-pine-grove"), [200.0, 30.0, 24.0]),
                fav(4, tr!("demo-place-nordheim-hall"), "Nordheim", [128.0, 64.0, 40.0]),
                fav(5, tr!("demo-fav-fireflies-beach"), tr!("common-sandbox"), [90.5, 140.2, 21.7]),
            ],
        ),
        (
            key(uri, "Example 3"),
            vec![fav(6, tr!("common-aurora-plaza"), tr!("common-aurora-demo"), [128.0, 128.0, 25.0])],
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fav(n: u128, name: &str) -> Favorite {
        Favorite {
            item: Uuid::from_u128(n),
            asset: Uuid::from_u128(100 + n),
            name: name.into(),
            region: format!("Synthetic region {n}"),
            pos: [10.0 * n as f32, 20.0, 30.0],
        }
    }

    fn temp_path(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("aurora-{tag}-{}", Uuid::new_v4()));
        let path = root.join(FILE);
        (root, path)
    }

    #[test]
    fn the_start_uri_is_the_typed_location_form() {
        let mut f = fav(1, "Synthetic");
        f.region = "Synthetic Bay".into();
        f.pos = [127.6, 3.2, 22.5];
        assert_eq!(f.start_location().to_login_string(), "uri:Synthetic Bay&128&3&23");
        f.pos = [-4.0, f32::NAN, 4096.4];
        assert_eq!(f.start_location().to_login_string(), "uri:Synthetic Bay&0&0&4096");
    }

    #[test]
    fn an_account_never_logged_in_has_none_and_shows_them_by_default() {
        let store = Store::default();
        let k = key("synthetic-grid", "Synthetic Alpha");
        assert!(store.favorites(&k).is_empty());
        assert!(store.shown(&k));
    }

    #[test]
    fn favorites_survive_a_restart_per_grid_and_account() {
        let (root, path) = temp_path("login-favorites");
        let alpha = key("synthetic-grid", " Synthetic Alpha ");
        let alpha_beta_grid = key("synthetic-other-grid", "synthetic alpha");
        let beta = key("synthetic-grid", "synthetic beta");
        {
            let mut store = Store::default();
            store.load(path.clone());
            assert!(store.update(&alpha, vec![fav(1, "One"), fav(2, "Two")]));
            assert!(!store.update(&alpha, vec![fav(1, "One"), fav(2, "Two")]), "unchanged: not written");
            assert!(store.update(&alpha_beta_grid, vec![fav(3, "Three")]));
            assert!(store.update(&beta, vec![fav(4, "Four")]));
        }
        let mut store = Store::default();
        store.load(path.clone());
        // the catalogue's key: trimmed, any case
        assert_eq!(
            store.favorites(&key("synthetic-grid", "SYNTHETIC ALPHA")),
            [fav(1, "One"), fav(2, "Two")]
        );
        assert_eq!(store.favorites(&alpha_beta_grid), [fav(3, "Three")]);
        store.forget(&beta);
        store.flush();
        let mut again = Store::default();
        again.load(path);
        assert!(again.favorites(&beta).is_empty());
        assert_eq!(again.favorites(&alpha).len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn switching_off_removes_them_and_keeps_the_choice() {
        let (root, path) = temp_path("login-favorites-off");
        let alpha = key("synthetic-grid", "synthetic alpha");
        let mut store = Store::default();
        store.load(path.clone());
        store.update(&alpha, vec![fav(1, "One")]);
        store.set_shown(&alpha, false);
        assert!(store.favorites(&alpha).is_empty());
        assert!(!store.update(&alpha, vec![fav(1, "One")]), "off: the session writes nothing");
        store.flush();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("One"), "no favourite left on disk");
        let mut restored = Store::default();
        restored.load(path.clone());
        assert!(!restored.shown(&alpha));
        restored.set_shown(&alpha, true);
        assert!(restored.update(&alpha, vec![fav(2, "Two")]));
        assert_eq!(restored.favorites(&alpha), [fav(2, "Two")]);
        drop(restored);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_save_keeps_what_another_viewer_wrote_meanwhile() {
        let (root, path) = temp_path("login-favorites-merge");
        let alpha = key("synthetic-grid", "synthetic alpha");
        let other = key("synthetic-grid", "synthetic other");
        let mut ours = Store::default();
        ours.load(path.clone());
        {
            let mut theirs = Store::default();
            theirs.load(path.clone());
            theirs.update(&other, vec![fav(9, "Theirs")]);
        }
        ours.update(&alpha, vec![fav(1, "Ours")]);
        ours.flush();
        let mut read_back = Store::default();
        read_back.load(path);
        assert_eq!(read_back.favorites(&other), [fav(9, "Theirs")]);
        assert_eq!(read_back.favorites(&alpha), [fav(1, "Ours")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unreadable_file_reads_as_empty_and_is_replaced() {
        let (root, path) = temp_path("login-favorites-bad");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        let alpha = key("synthetic-grid", "synthetic alpha");
        let mut store = Store::default();
        store.load(path.clone());
        assert!(store.favorites(&alpha).is_empty());
        store.update(&alpha, vec![fav(1, "One")]);
        store.flush();
        let mut again = Store::default();
        again.load(path);
        assert_eq!(again.favorites(&alpha), [fav(1, "One")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unplaced_favorites_keep_their_stored_place_and_new_name() {
        let stored = vec![fav(1, "Old name"), fav(2, "Two")];
        let current = |n: u128, asset: u128, name: &str, place: Option<(String, [f32; 3])>| Current {
            item: Uuid::from_u128(n),
            asset: Uuid::from_u128(asset),
            name: name.into(),
            place,
        };
        let merged = merge(
            &stored,
            vec![
                current(2, 102, "Two", Some(("Synthetic moved".into(), [1.0, 2.0, 3.0]))),
                current(1, 101, "New name", None),
                current(3, 103, "Never placed", None),
            ],
        );
        assert_eq!(merged.len(), 2, "the never placed one waits");
        assert_eq!((merged[0].item, merged[0].region.as_str()), (Uuid::from_u128(2), "Synthetic moved"));
        assert_eq!(
            (merged[1].name.as_str(), merged[1].region.as_str()),
            ("New name", "Synthetic region 1")
        );
        // another landmark behind the same item: its old place is not kept
        let merged = merge(&stored, vec![current(1, 999, "Relinked", None)]);
        assert!(merged.is_empty());
    }

    #[test]
    fn the_demo_store_has_favorites_for_the_selected_and_the_third_demo_accounts() {
        let store = demo_store("synthetic-grid");
        assert!(store.favorites(&key("synthetic-grid", "Example 1")).is_empty());
        assert_eq!(store.favorites(&key("synthetic-grid", "Example 2")).len(), 5);
        assert_eq!(store.favorites(&key("synthetic-grid", "Example 3")).len(), 1);
    }
}
