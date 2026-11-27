//! The import of a Legacy Viewer's lists, from Preferences › Language and
//! updates (`world::legacy_lists`): reading its folders off the frame, what
//! the import window shows, and what a finished import says.
use super::*;
use crate::demo::legacy_lists::{Mode, mode};
use crate::legacy_profile::Account;
use crate::world::legacy_lists::{self, Found, Parts, Report};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

#[derive(Default)]
pub(super) struct LegacyLists {
    /// The folders being read.
    looking: Option<Receiver<Vec<Found>>>,
    /// What they hold, until the window closes on an import.
    found: Vec<Found>,
    /// AURORA_DEMO_LEGACY_LISTS: the folder is written, then imports left.
    demo_started: bool,
    demo_imports: u8,
    /// The demo's Legacy Viewer folder, looked in instead of the PC's.
    demo_apps: Option<Vec<PathBuf>>,
}

impl App {
    /// Who to look for: the account logged in, on the grid of its
    /// transcripts (`chat_log::grid_name`), with or without transcripts.
    pub(super) fn legacy_lists_account(&self) -> Option<Account> {
        let login = self.world.login.as_ref()?;
        let grid = match self.world.chat_log.identity() {
            Some(identity) => identity.grid.clone(),
            None => crate::world::chat_log::grid_name(&self.settings.grid, &self.settings.custom_login_uri, self.demo),
        };
        Some(Account {
            first: login.first_name.clone(),
            last: login.last_name.clone(),
            grid,
        })
    }

    /// The import window opens: read the viewers' folders.
    pub(super) fn legacy_lists_opened(&mut self) {
        self.open_legacy_lists(Parts::default(), None);
    }

    pub(super) fn open_teleport_history_import(&mut self, chosen: Option<PathBuf>) {
        self.panels.settings = true;
        self.options_ui.open(ui::options::Page::System, 0);
        self.open_legacy_lists(
            Parts {
                teleports: true,
                derender: false,
                media: false,
                contact_sets: false,
            },
            chosen,
        );
    }

    fn open_legacy_lists(&mut self, parts: Parts, chosen: Option<PathBuf>) {
        self.options_ui.legacy_lists_status.clear();
        self.options_ui.legacy_lists = Default::default();
        self.options_ui.legacy_lists.open = true;
        self.options_ui.legacy_lists.parts = parts;
        let Some(account) = self.legacy_lists_account() else {
            self.options_ui.legacy_lists.found = Some(Vec::new());
            return;
        };
        self.options_ui.legacy_lists.other_grid = !account.grid_known() || (self.demo && mode() == Some(Mode::OtherGrid));
        let apps = match &self.legacy_lists.demo_apps {
            Some(apps) => apps.clone(),
            None => self.world.chat_log.legacy_apps(),
        };
        let (tx, rx) = channel();
        self.legacy_lists.looking = Some(rx);
        let _ = std::thread::Builder::new().name("aurora-legacy-lists".into()).spawn(move || {
            let found = match chosen {
                Some(dir) => legacy_lists::find_chosen(dir, &account),
                None => legacy_lists::find(&apps, &account),
            };
            let _ = tx.send(found);
        });
    }

    pub(super) fn legacy_lists_start(&mut self, index: usize, parts: Parts) {
        let found = std::mem::take(&mut self.legacy_lists.found);
        let Some(found) = found.into_iter().nth(index) else {
            return;
        };
        let mut lists = found.lists;
        lists.keep(parts);
        let report = self.world.merge_legacy_lists(lists);
        if report.teleports > 0 {
            self.places_ui.history_reordered();
        }
        log::info!("legacy lists imported from {}: {report:?}", found.dir.display());
        if report.unseated {
            // we sat on it: gAgent.standUp()
            self.send(NetCommand::OneShotControl(control::STAND_UP));
        }
        self.options_ui.legacy_lists_status = status(&report);
    }

    /// Logout: what was read stays with its account.
    pub(super) fn legacy_lists_reset(&mut self) {
        self.legacy_lists.looking = None;
        self.legacy_lists.found.clear();
        self.options_ui.legacy_lists = Default::default();
        self.options_ui.legacy_lists_status.clear();
    }

    /// Every frame: what the folders hold.
    pub(super) fn poll_legacy_lists(&mut self) {
        if let Some(result) = self.world.tp_storage.poll_initial_import() {
            match result {
                Ok(added) if added > 0 => {
                    self.places_ui.history_reordered();
                    self.world.system_notice(
                        aurora_i18n::tr!("ui-places-legacy-history-imported-at-login", count = added),
                        crate::world::notice_topics::Topic::Info,
                    );
                }
                Ok(_) => {}
                Err(e) => log::warn!("teleport history: initial import failed ({e})"),
            }
        }
        self.demo_legacy_lists();
        let Some(rx) = &self.legacy_lists.looking else {
            return;
        };
        let found = match rx.try_recv() {
            Ok(found) => found,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            // the reader did not start or panicked: the search ends with
            // nothing found, not "searching" for ever
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                log::warn!("legacy lists: the folders' reader stopped without an answer");
                Vec::new()
            }
        };
        self.legacy_lists.looking = None;
        self.options_ui.legacy_lists.found = Some(found.iter().map(Found::folder).collect());
        self.legacy_lists.found = found;
        if self.legacy_lists.demo_imports > 0 {
            self.legacy_lists.demo_imports -= 1;
            self.options_ui.legacy_lists.open = false;
            self.legacy_lists_start(0, Parts::default());
            if self.legacy_lists.demo_imports > 0 {
                let status = std::mem::take(&mut self.options_ui.legacy_lists_status);
                log::info!("demo legacy lists, first import: {status}");
                self.legacy_lists_opened();
            }
        }
    }

    /// AURORA_DEMO_LEGACY_LISTS: a Legacy Viewer's folder for the demo
    /// account, once it is in the world, and the page with the import.
    fn demo_legacy_lists(&mut self) {
        if !self.demo || self.legacy_lists.demo_started || !self.in_world() {
            return;
        }
        let (Some(mode), Some(login)) = (mode(), self.world.login.as_ref()) else {
            return;
        };
        self.legacy_lists.demo_started = true;
        // in the demo's own settings folder, never beside the transcripts,
        // which can be in a folder of the user's
        let root = crate::demo::legacy_lists::root(&crate::settings::account_dir(&login.agent_id));
        let object = self.world.objects.iter().find(|(_, o)| o.parent_id == 0).map(|(_, o)| o.full_id);
        if let Err(e) = crate::demo::legacy_lists::seed(&root, &self.world.own_name(), mode, object) {
            log::warn!("demo Legacy Viewer lists not written: {e}");
        }
        self.legacy_lists.demo_apps = Some(vec![root]);
        // kept in memory: a run must not change what the next one shows
        self.world.tp_storage = crate::world::tphistory::HistoryStorage::in_memory(Vec::new());
        self.world.derender = Default::default();
        self.world.media_filter = Default::default();
        self.world.contact_sets = Default::default();
        self.legacy_lists.demo_imports = match mode {
            Mode::Window | Mode::None | Mode::Unreadable | Mode::OtherGrid => 0,
            Mode::Run => 1,
            Mode::Again => 2,
        };
        self.panels.settings = true;
        self.options_ui.open(ui::options::Page::System, 0);
        self.legacy_lists_opened();
    }
}

/// The result as the preferences say it. "Nothing new" only when every
/// list was read and saved: a list that failed may well hold something new.
fn status(report: &Report) -> String {
    use aurora_i18n::tr;
    let mut parts = Vec::new();
    if report.total() > 0 {
        parts.push(
            tr!(
                "ui-options-legacy-lists-added",
                teleports = report.teleports,
                derender = report.derender,
                media = report.media,
                sets = report.contact_sets
            )
            .to_owned(),
        );
    } else if report.unreadable.is_empty() && report.unsaved.is_empty() {
        parts.push(tr!("ui-options-legacy-lists-nothing").to_owned());
    }
    if !report.unreadable.is_empty() {
        parts.push(tr!("ui-options-legacy-lists-failed", files = report.unreadable.join(", ")).to_owned());
    }
    if !report.unsaved.is_empty() {
        parts.push(tr!("ui-options-legacy-lists-unsaved", files = report.unsaved.join(", ")).to_owned());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_new_only_when_every_list_was_read_and_saved() {
        let nothing = aurora_i18n::tr!("ui-options-legacy-lists-nothing").to_owned();
        assert_eq!(status(&Report::default()), nothing);
        let unsaved = Report {
            unsaved: vec!["settings_friends_groups.xml"],
            ..Report::default()
        };
        let text = status(&unsaved);
        assert!(!text.contains(&nothing) && text.contains("settings_friends_groups.xml"), "{text}");
        let unreadable = Report {
            unreadable: vec!["medialist.xml"],
            ..Report::default()
        };
        let text = status(&unreadable);
        assert!(!text.contains(&nothing) && text.contains("medialist.xml"), "{text}");
        let added = Report {
            teleports: 2,
            unsaved: vec!["settings_friends_groups.xml"],
            ..Report::default()
        };
        let text = status(&added);
        assert!(
            text.contains('2') && text.contains("settings_friends_groups.xml") && !text.starts_with(' '),
            "{text}"
        );
    }
}
