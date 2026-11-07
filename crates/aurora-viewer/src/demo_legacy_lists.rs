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

/// The Legacy Viewer's folder of the scenario, beside the account's.
pub fn root(account_logs: &Path) -> PathBuf {
    account_logs.with_extension("legacy-lists")
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
    std::fs::write(
        dir.join(CONTACT_SETS_FILE),
        format!(
            "<llsd><map><key>Legacy friends</key><map><key>color</key><array><real>0.4</real><real>0.7</real><real>0.9</real></array>\
             <key>friends</key><map><key>{}</key><string></string></map></map></map></llsd>",
            super::chatlog::PEER
        ),
    )
}
