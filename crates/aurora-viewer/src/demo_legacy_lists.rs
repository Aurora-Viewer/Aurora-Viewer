//! AURORA_DEMO_LEGACY_LISTS: a Legacy Viewer's lists for the demo account,
//! made up here, and Preferences › Language and updates on their import.
use crate::media::filter::{self, Action, Edit, MediaFilter};
use crate::world::contact_sets::CONTACT_SETS_FILE;
use crate::world::derender::{self, Derender, Entry, Kind};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The window, waiting for its button.
    Window,
    /// Imported once, the status under the button.
    Run,
    /// Imported twice: the second adds nothing.
    Again,
    /// No folder for the account.
    None,
    /// The account's only list cannot be read.
    Unreadable,
    /// The window as on a grid whose folders cannot be told apart.
    OtherGrid,
}

pub fn mode() -> Option<Mode> {
    Some(match std::env::var("AURORA_DEMO_LEGACY_LISTS").ok()?.as_str() {
        "run" => Mode::Run,
        "again" => Mode::Again,
        "none" => Mode::None,
        "unreadable" => Mode::Unreadable,
        "other-grid" => Mode::OtherGrid,
        _ => Mode::Window,
    })
}

/// The Legacy Viewer's folder of the scenario, in the demo account's own
/// settings folder (`settings::account_dir`, under the demo's folder).
pub fn root(account_dir: &Path) -> PathBuf {
    account_dir.join("legacy-lists")
}

/// Write the folder (each run starts from the same files). `object`: a
/// thing of the demo's world, to see it leave.
pub fn seed(root: &Path, own_name: &str, mode: Mode, object: Option<Uuid>) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(root);
    if mode == Mode::None {
        return Ok(());
    }
    let dir = root.join(own_name.to_lowercase().replace(' ', "_"));
    std::fs::create_dir_all(&dir)?;
    if mode == Mode::Unreadable {
        return std::fs::write(dir.join(filter::FILE), "not a list");
    }
    std::fs::write(
        dir.join("teleport_history.txt"),
        "{'date':d\"2024-03-02T18:40:00Z\",'global_pos':[r256128,r256128,r22],'slurl':'','title':'The Old Pier, Harbor'}\n\
         {'date':d\"2024-03-05T20:15:00Z\",'global_pos':[r512064,r256200,r40],'slurl':'','title':'Lantern Market, Stalls'}\n",
    )?;
    if let Some(id) = object {
        Derender::load(dir.join(derender::FILE)).add(
            id,
            Entry {
                name: "Legacy derender".into(),
                region: "Harbor".into(),
                kind: Kind::Object,
                date: 1_709_400_000,
                permanent: true,
            },
        );
    }
    let mut media = MediaFilter::load(dir.join(filter::FILE));
    media.apply(Edit::Set("radio.example".into(), Action::Allow));
    media.apply(Edit::Set("tracker.example".into(), Action::Deny));
    // both are saved by the writer: on the disk before the window reads them
    for file in [derender::FILE, filter::FILE] {
        crate::saves::wait_for(&dir.join(file), crate::saves::BEFORE_READ);
    }
    std::fs::write(
        dir.join(CONTACT_SETS_FILE),
        format!(
            "<llsd><map><key>Legacy friends</key><map><key>color</key><array><real>0.4</real><real>0.7</real><real>0.9</real></array>\
             <key>friends</key><map><key>{}</key><string></string></map></map></map></llsd>",
            super::chatlog::PEER
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_profile::Account;
    use crate::world::legacy_lists::{Counts, find};

    #[test]
    fn the_window_finds_every_list_the_scenario_writes() {
        let account_dir = std::env::temp_dir().join(format!("aurora-demo-legacy-lists-{}", Uuid::new_v4()));
        let root = root(&account_dir);
        assert!(root.starts_with(&account_dir));
        // the demo's grid (`chat_log::grid_name`)
        let account = Account {
            first: "Demo".into(),
            last: "Resident".into(),
            grid: "demo".into(),
        };
        seed(&root, "Demo Resident", Mode::Window, Some(Uuid::from_u128(7))).unwrap();
        let found = find(std::slice::from_ref(&root), &account);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].counts,
            Counts {
                teleports: 2,
                derender: 1,
                media: 2,
                // the set and its member
                contact_sets: 2,
            }
        );
        seed(&root, "Demo Resident", Mode::Unreadable, None).unwrap();
        let found = find(std::slice::from_ref(&root), &account);
        assert_eq!(found[0].folder().unreadable, [filter::FILE]);
        seed(&root, "Demo Resident", Mode::None, None).unwrap();
        assert!(find(std::slice::from_ref(&root), &account).is_empty());
        std::fs::remove_dir_all(&account_dir).unwrap();
    }
}
