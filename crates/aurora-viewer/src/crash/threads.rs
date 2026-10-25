//! A panic in a long-lived background thread used to leave the viewer
//! running without it: the network's tokio runtime catches a task's panic
//! and goes on without the session loop (a frozen world), a dead cache or
//! texture page thread stops loading, a dead chat log thread stops saving,
//! and nothing said so. The panic hook names the thread here; when it is
//! one the session cannot do without, the app ends the session cleanly at
//! the next frame (`app.rs`, beside the render thread's own check) and the
//! report says which thread died, with the panic's stack. Threads whose
//! panic already ends the viewer are not listed: the main thread (the
//! panic leaves `main`), the render thread (`Renderer::failed`), the frame
//! workers (`frame_par` resumes the panic on the main thread) and the job
//! pool (rayon aborts the process on an uncaught panic).

use std::sync::OnceLock;

/// The threads the session cannot go on without (their names as spawned).
const CRITICAL: &[&str] = &[
    // the network runtime's workers: the session loop and its tasks
    "aurora-net",
    // GPU texture pages (aurora-render/src/textures.rs)
    "aurora-pages",
    // what the frame hands over to be freed (scene/jobs.rs)
    "aurora-trash",
    // the disk caches' writers
    "aurora-cache",
    "aurora-texcache",
    // the chat history's writers: without them, conversations are lost
    "aurora-chatlog",
    "aurora-transcript",
];

static DIED: OnceLock<String> = OnceLock::new();

pub fn is_critical(name: &str) -> bool {
    CRITICAL.contains(&name)
}

/// From the panic hook, on the panicking thread. The first critical death
/// is kept; the session folder gets `thread.txt` for the report.
pub fn on_panic(dir: &std::path::Path) {
    let current = std::thread::current();
    let Some(name) = current.name() else { return };
    if is_critical(name) && DIED.set(name.to_owned()).is_ok() {
        let _ = std::fs::write(dir.join("thread.txt"), name);
    }
}

/// The critical thread that died, if one did.
pub fn died() -> Option<&'static str> {
    DIED.get().map(String::as_str)
}

/// A name written by `on_panic`, checked again before it is published.
pub fn published_name(text: &str) -> Option<&'static str> {
    let name = text.trim();
    CRITICAL.iter().copied().find(|known| *known == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_threads_end_the_session_and_only_they_are_published() {
        for name in CRITICAL {
            assert!(is_critical(name));
            assert_eq!(published_name(&format!(" {name}\n")), Some(*name));
        }
        // handled elsewhere, or disposable
        for name in ["main", "aurora-render", "aurora-jobs-3", "aurora-worker-0", "aurora-photo-save", ""] {
            assert!(!is_critical(name), "{name}");
        }
        assert_eq!(published_name("Alice Resident"), None);
    }
}
