//! A Legacy Viewer's lists brought into the account, one way and adding
//! only: its files are only read, and what the account has already stays
//! (`HistoryStorage::merge`, `Derender::merge`, `MediaFilter::merge`,
//! `ContactSets::merge_import`). The four files are the same on both sides
//! (LL_PATH_PER_SL_ACCOUNT), so importing again adds nothing, but an entry
//! removed here comes back with it: the import is always asked for, never
//! made at login.
//!
//! `read` runs away from the frame; `World::merge_legacy_lists` then
//! applies what it found.

use super::contact_sets::CONTACT_SETS_FILE;
use super::contact_sets::importing::{EMPTY_MAP, ImportData};
use super::derender::{self, FileEntries};
use super::tphistory::{self, HistoryItem};
use crate::legacy_profile::{Account, account_folders};
use crate::media::filter::{self, Rule};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Large enough for years of history, bounded before parsing.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The lists to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub teleports: bool,
    pub derender: bool,
    pub media: bool,
    pub contact_sets: bool,
}

impl Default for Parts {
    fn default() -> Self {
        Parts {
            teleports: true,
            derender: true,
            media: true,
            contact_sets: true,
        }
    }
}

/// How much each file holds, shown before importing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub teleports: usize,
    pub derender: usize,
    pub media: usize,
    pub contact_sets: usize,
}

impl Counts {
    fn any(&self) -> bool {
        *self != Counts::default()
    }
}

/// An account's folder of a Legacy Viewer.
pub struct Found {
    pub dir: PathBuf,
    pub counts: Counts,
    pub lists: Lists,
}

/// A folder as the import window shows it.
pub struct Folder {
    pub dir: PathBuf,
    pub counts: Counts,
    /// Files in it that could not be read.
    pub unreadable: Vec<&'static str>,
}

impl Found {
    /// What the import window shows of it.
    pub fn folder(&self) -> Folder {
        Folder {
            dir: self.dir.clone(),
            counts: self.counts,
            unreadable: self.lists.unreadable.clone(),
        }
    }
}

/// What a Legacy Viewer's account folder holds.
#[derive(Default)]
pub struct Lists {
    history: Vec<HistoryItem>,
    derender: Option<FileEntries>,
    media: Vec<Rule>,
    contact_sets: Option<ImportData>,
    /// Files that exist and could not be read.
    unreadable: Vec<&'static str>,
}

impl Lists {
    fn counts(&self) -> Counts {
        Counts {
            teleports: self.history.len(),
            derender: self.derender.as_ref().map_or(0, FileEntries::len),
            media: self.media.len(),
            contact_sets: self.contact_sets.as_ref().map_or(0, ImportData::entries),
        }
    }

    /// Drop the lists not asked for.
    pub fn keep(&mut self, parts: Parts) {
        if !parts.teleports {
            self.history.clear();
        }
        if !parts.derender {
            self.derender = None;
        }
        if !parts.media {
            self.media.clear();
        }
        if !parts.contact_sets {
            self.contact_sets = None;
        }
    }
}

/// What an import changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub teleports: usize,
    pub derender: usize,
    pub media: usize,
    pub contact_sets: usize,
    pub unreadable: Vec<&'static str>,
    /// The account's contact sets could not be written.
    pub contact_sets_unsaved: bool,
    /// Lists that could not be written: they stay as they were.
    pub unsaved: Vec<&'static str>,
    /// Our avatar sat on something that left the world.
    pub unseated: bool,
}

impl Report {
    pub fn total(&self) -> usize {
        self.teleports + self.derender + self.media + self.contact_sets
    }
}

/// The file's bytes, None when it is absent. Unreadable or too large: named
/// in `unreadable`.
fn bytes(folder: &Path, name: &'static str, unreadable: &mut Vec<&'static str>) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    let read = std::fs::File::open(folder.join(name)).and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut data));
    match read {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Ok(_) if data.len() as u64 <= MAX_BYTES => Some(data),
        result => {
            log::warn!("Legacy Viewer list {name} not read: {result:?}");
            unreadable.push(name);
            None
        }
    }
}

/// The account's folders that hold a list, read, a folder shared by two
/// viewers once.
pub fn find(apps: &[PathBuf], account: &Account) -> Vec<Found> {
    let mut seen = Vec::new();
    let mut found = Vec::new();
    for dir in account_folders(apps, account) {
        let real = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if seen.contains(&real) {
            continue;
        }
        seen.push(real);
        let lists = read(&dir);
        let counts = lists.counts();
        if counts.any() || !lists.unreadable.is_empty() {
            found.push(Found { dir, counts, lists });
        }
    }
    found
}

/// Read the lists of an account folder.
pub fn read(folder: &Path) -> Lists {
    let mut lists = Lists::default();
    let mut failed = Vec::new();
    if let Some(data) = bytes(folder, tphistory::TELEPORT_HISTORY_FILE, &mut failed) {
        lists.history = tphistory::places(String::from_utf8_lossy(&data).as_bytes());
    }
    if let Some(data) = bytes(folder, derender::FILE, &mut failed) {
        match FileEntries::from_xml(&data) {
            Ok(file) => lists.derender = Some(file),
            Err(e) => {
                log::warn!("Legacy Viewer asset blacklist unreadable: {e}");
                failed.push(derender::FILE);
            }
        }
    }
    if let Some(data) = bytes(folder, filter::FILE, &mut failed) {
        match filter::parse(&data) {
            Ok(rules) => lists.media = rules,
            Err(e) => {
                log::warn!("Legacy Viewer media list unreadable: {e}");
                failed.push(filter::FILE);
            }
        }
    }
    if let Some(data) = bytes(folder, CONTACT_SETS_FILE, &mut failed) {
        match ImportData::from_xml(&data) {
            Ok(sets) => lists.contact_sets = Some(sets),
            Err(EMPTY_MAP) => {}
            Err(e) => {
                log::warn!("Legacy Viewer contact sets unreadable: {e}");
                failed.push(CONTACT_SETS_FILE);
            }
        }
    }
    lists.unreadable = failed;
    lists
}

impl super::World {
    /// Add what `read` found to the account's lists.
    pub fn merge_legacy_lists(&mut self, lists: Lists) -> Report {
        let mut report = Report {
            unreadable: lists.unreadable,
            ..Report::default()
        };
        match self.tp_storage.merge(lists.history) {
            Ok(changed) => report.teleports = changed,
            Err(e) => {
                log::warn!("teleport history not saved: {e}");
                report.unsaved.push(tphistory::TELEPORT_HISTORY_FILE);
            }
        }
        match self.media_filter.merge(lists.media) {
            Ok(added) => report.media = added,
            Err(e) => {
                log::warn!("media filter list not saved: {e}");
                report.unsaved.push(filter::FILE);
            }
        }
        if let Some(file) = lists.derender {
            (report.derender, report.unseated) = self.merge_derender(file);
        }
        if let Some(data) = lists.contact_sets {
            let adds = data.adds(&self.contact_sets);
            if adds > 0 {
                let social = &self.social;
                match self.contact_sets.merge_import(data, |id| social.is_friend(id)) {
                    Ok(_) => report.contact_sets = adds,
                    Err(e) => {
                        log::warn!("contact sets not saved: {e}");
                        report.contact_sets_unsaved = true;
                    }
                }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::super::derender::tests_support::demo_world;
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-legacy-lists-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn write_lists(dir: &Path) {
        // as the Legacy Viewer writes them
        std::fs::write(
            dir.join(tphistory::TELEPORT_HISTORY_FILE),
            "{'date':d\"2017-06-21T03:13:59.28Z\",'global_pos':[r462388,r303008,r2402.53],'slurl':'','title':'A place, A region'}\n",
        )
        .expect("history");
        std::fs::write(
            dir.join(filter::FILE),
            "<llsd><array><map><key>action</key><string>deny</string><key>domain</key><string>tracker.example</string></map></array></llsd>",
        )
        .expect("media");
        std::fs::write(
            dir.join(derender::FILE),
            "<llsd><map><key>7d4581af-c4ca-6003-e2a4-3038efea2a25</key><map><key>asset_name</key><string>A thing</string>\
             <key>asset_type</key><integer>6</integer><key>asset_date</key><string>2024-01-02 03:04:05</string></map></map></llsd>",
        )
        .expect("derender");
        std::fs::write(
            dir.join(CONTACT_SETS_FILE),
            "<llsd><map><key>Friends</key><map><key>color</key><array><real>0.5</real><real>0.5</real><real>0.5</real></array>\
             <key>friends</key><map><key>00000000-0000-0000-0000-000000000042</key><string></string></map></map></map></llsd>",
        )
        .expect("contact sets");
    }

    #[test]
    fn find_takes_the_account_s_folder_once_and_the_ticked_lists() {
        let root = folder("find");
        let dir = root.join("mira_resident");
        std::fs::create_dir_all(&dir).expect("account");
        std::fs::create_dir_all(root.join("someone_else")).expect("other");
        write_lists(&dir);
        write_lists(&root.join("someone_else"));
        let account = Account {
            first: "Mira".into(),
            last: "Resident".into(),
            grid: "agni".into(),
        };
        let mut found = find(&[root.clone(), root.clone()], &account);
        assert_eq!(found.len(), 1);
        let Found {
            dir: at,
            counts,
            mut lists,
        } = found.remove(0);
        assert_eq!(at, dir);
        assert_eq!(
            counts,
            Counts {
                teleports: 1,
                derender: 1,
                media: 1,
                contact_sets: 1
            }
        );
        lists.keep(Parts {
            media: false,
            contact_sets: false,
            derender: false,
            ..Parts::default()
        });
        let (mut world, _) = demo_world();
        let report = world.merge_legacy_lists(lists);
        assert_eq!((report.teleports, report.total()), (1, 1));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_contact_file_with_nicknames_alone_counts() {
        let dir = folder("nicknames");
        std::fs::write(
            dir.join(CONTACT_SETS_FILE),
            "<llsd><map><key>Pseudonyms</key><map><key>00000000-0000-0000-0000-000000000042</key><string>Nick</string></map></map></llsd>",
        )
        .expect("contact sets");
        assert_eq!(read(&dir).counts().contact_sets, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_list_comes_in_once() {
        let dir = folder("once");
        write_lists(&dir);
        let (mut world, _) = demo_world();
        let report = world.merge_legacy_lists(read(&dir));
        assert_eq!(
            (report.teleports, report.derender, report.media, report.contact_sets),
            (1, 1, 1, 2),
            "the set and its member"
        );
        assert!(report.unreadable.is_empty());
        assert_eq!(world.merge_legacy_lists(read(&dir)), Report::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contact_sets_that_cannot_be_saved_are_not_counted() {
        let dir = folder("unsaved");
        write_lists(&dir);
        let (mut world, _) = demo_world();
        // a file where the account's folder should be
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "").expect("blocker");
        world.contact_sets = crate::world::contact_sets::ContactSets::load(blocker.join(CONTACT_SETS_FILE));
        let report = world.merge_legacy_lists(read(&dir));
        assert!(report.contact_sets_unsaved);
        assert_eq!(report.contact_sets, 0);
        assert_eq!(
            (report.teleports, report.derender, report.media),
            (1, 1, 1),
            "the other lists still come in"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_without_lists_adds_nothing_and_a_broken_file_is_named() {
        let dir = folder("broken");
        let (mut world, _) = demo_world();
        assert_eq!(world.merge_legacy_lists(read(&dir)), Report::default());

        std::fs::write(dir.join(filter::FILE), "not xml").expect("media");
        std::fs::write(dir.join(CONTACT_SETS_FILE), "<llsd><map></map></llsd>").expect("sets");
        let report = world.merge_legacy_lists(read(&dir));
        assert_eq!(report.unreadable, [filter::FILE], "an empty contact sets file is not an error");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
