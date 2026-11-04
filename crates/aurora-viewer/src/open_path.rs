//! Files and folders handed to the system's programs: the file manager,
//! the photo viewer, the external script editor. The program (`xdg-open`,
//! `open`, `explorer`, the editor) is waited for on a thread of its own: a
//! child nobody waits for stays a zombie process on Unix until the viewer
//! exits, one per click.

use std::path::Path;
use std::process::Command;

/// Starts `command` without waiting for it.
pub fn spawn(command: &mut Command) -> std::io::Result<()> {
    spawn_waited(command).map(|_| ())
}

/// The child's process id.
fn spawn_waited(command: &mut Command) -> std::io::Result<u32> {
    let mut child = command.spawn()?;
    let id = child.id();
    let reaper = std::thread::Builder::new().name("aurora-open-wait".into()).spawn(move || {
        let _ = child.wait();
    });
    if let Err(e) = reaper {
        // the child already runs; only its exit status stays unread
        log::warn!("open: no thread to wait for the launcher ({e})");
    }
    Ok(id)
}

/// Opens a file or a folder with the system's program for it.
pub fn open(path: &Path) -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    spawn(Command::new(program).arg(path))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    /// Once the launcher exits it is waited for: its process entry goes
    /// (a zombie keeps one, in state Z, until the viewer exits).
    #[test]
    fn launcher_is_waited_for() {
        let id = super::spawn_waited(&mut std::process::Command::new("true")).expect("true runs");
        let stat = format!("/proc/{id}/stat");
        let start = std::time::Instant::now();
        while let Ok(text) = std::fs::read_to_string(&stat) {
            let state = text.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().next());
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "still there, state {state:?}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
