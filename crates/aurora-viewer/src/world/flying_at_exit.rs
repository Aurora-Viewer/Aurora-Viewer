//! Flying when the last session ended, so the next login flies again: the
//! Legacy Viewer's `FlyingAtExit`, written in `LLAppViewer::disconnectViewer`
//! (indra/newview/llappviewer.cpp) and read in `LLAgent::init`
//! (indra/newview/llagent.cpp), which sets the FLY control before the first
//! AgentUpdate leaves (originally LGPL 2.1). Kept per account here: the next
//! login on the same avatar flies, not every avatar of the PC.
//!
//! The rest follows the same rules as a flight started with the key: the
//! arrival in a region that blocks flying lands the avatar
//! (`process_agent_movement_complete`, `process_region_info`), and a login
//! seated on an object stops the flight (`LLVOAvatar::sitOnObject`).
use std::path::Path;

pub const FILE: &str = "flying_at_exit.json";

/// The states whose write was refused, the latest per account: read before
/// the file at the next login, tried again (`save_retry`) and once more at
/// exit. The Legacy Viewer's FlyingAtExit is a setting, saved with the
/// others when the viewer closes.
static OWED: crate::save_retry::OwedFiles = crate::save_retry::OwedFiles::new("aurora-flying-at-exit", "flying state at logout");

/// The saved state; false when there is none or it cannot be read.
pub fn load(path: &Path) -> bool {
    match OWED.owed(path).map_or_else(|| std::fs::read(path), Ok) {
        Ok(bytes) => serde_json::from_slice::<bool>(&bytes).unwrap_or_else(|e| {
            log::warn!("cannot read the flying state at the last logout: {e}");
            false
        }),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("cannot read the flying state at the last logout: {e}");
            }
            false
        }
    }
}

/// Remember whether the session that ends was flying.
pub fn save(path: &Path, flying: bool) {
    let bytes = if flying { b"true".to_vec() } else { b"false".to_vec() };
    if let Err(e) = OWED.write(path, bytes) {
        log::warn!("cannot save the flying state at logout ({e}); kept, tried again later");
    }
}

/// The viewer closing: a state still owed is tried once more.
pub fn save_before_exit() {
    OWED.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flight_at_logout_comes_back_for_the_same_account_only() {
        let root = std::env::temp_dir().join(format!("aurora-flying-{}", uuid::Uuid::new_v4()));
        let first = root.join("first").join(FILE);
        let second = root.join("second").join(FILE);
        // nothing saved yet: walking
        assert!(!load(&first));
        save(&first, true);
        assert!(load(&first));
        assert!(!load(&second), "another account keeps its own state");
        // logged out on foot this time
        save(&first, false);
        assert!(!load(&first));
        // a damaged file walks
        std::fs::write(&first, b"not a state").unwrap();
        assert!(!load(&first));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The account's folder refuses the write (a file stands in its
    /// place): the next login still flies, and the state reaches the disk
    /// once the folder takes writes.
    #[test]
    fn a_refused_write_keeps_the_flight_for_the_next_login() {
        let root = std::env::temp_dir().join(format!("aurora-flying-refused-{}", uuid::Uuid::new_v4()));
        let account = root.join("account");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&account, b"in the way").unwrap();
        let path = account.join(FILE);
        save(&path, true);
        assert!(load(&path), "kept in memory");
        std::fs::remove_file(&account).unwrap();
        // the retry's run
        OWED.retry_now();
        assert_eq!(OWED.owed(&path), None, "nothing owed any more");
        assert_eq!(std::fs::read(&path).unwrap(), b"true");
        std::fs::remove_dir_all(root).unwrap();
    }
}
