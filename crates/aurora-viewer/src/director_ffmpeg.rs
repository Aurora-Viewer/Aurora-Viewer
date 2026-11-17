//! Finding the ffmpeg the Director records with, again without restarting
//! the viewer, and the ffmpeg pack of the project (`aurora_updater::
//! ffmpeg_pack`). Aurora's own: the Legacy Viewer records no film (#3897:
//! installed, then no sign of how to go on).
//!
//! The viewer's own PATH is the one it was started with: an ffmpeg
//! installed meanwhile (winget, a folder added to the PATH) is not in it.
//! "Look again" therefore reads the PATH the system has now (on Windows the
//! user's and the machine's, from the registry, as a new terminal would get
//! them) and the folders the usual installers put it in, besides the
//! viewer's own. The search touches the disk: it runs on a thread.

use aurora_updater::ffmpeg_pack::{self, Published};
use aurora_updater::{Cancel, Failure, Progress, feed::Source};
use std::ffi::OsString;
use std::path::PathBuf;

/// `%NAME%` replaced by the variable's value (as Windows writes the
/// folders of a PATH kept in the registry); a name without a value, or a
/// lone `%`, stays as it is.
pub fn expand(text: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match var(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The folders of several PATHs and lists, in order, each once (without
/// case on Windows), the empty ones left out.
pub fn joined(parts: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    parts
        .into_iter()
        .filter(|folder| !folder.as_os_str().is_empty())
        .filter(|folder| {
            let key = folder.to_string_lossy().trim_end_matches(['\\', '/']).to_owned();
            seen.insert(if cfg!(windows) { key.to_lowercase() } else { key })
        })
        .collect()
}

/// The PATH folders the system gives a program started now.
#[cfg(windows)]
fn system_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
    };
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn read(root: HKEY, key: &str) -> Option<String> {
        let (key, name) = (wide(key), wide("Path"));
        let mut buffer = vec![0u16; 32_768];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: both strings end with a NUL, `buffer` holds `bytes`
        // writable bytes, and the type output is not asked (null).
        let status = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    }
    let machine = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    // the machine's folders first, as Windows builds a new process's PATH
    [read(HKEY_LOCAL_MACHINE, machine), read(HKEY_CURRENT_USER, "Environment")]
        .into_iter()
        .flatten()
        .flat_map(|path| {
            let path = expand(&path, |name| std::env::var(name).ok());
            std::env::split_paths(&OsString::from(path)).collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(not(windows))]
fn system_paths() -> Vec<PathBuf> {
    Vec::new()
}

/// Where the usual installers put ffmpeg, beside the PATH (a program
/// started from the macOS Dock does not get Homebrew's PATH, for one).
fn known_folders() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if cfg!(windows) {
        [
            var("LOCALAPPDATA").map(|d| d.join(r"Microsoft\WinGet\Links")),
            var("ProgramData").map(|d| d.join(r"chocolatey\bin")),
            var("USERPROFILE").map(|d| d.join(r"scoop\shims")),
            var("ProgramFiles").map(|d| d.join(r"ffmpeg\bin")),
            Some(PathBuf::from(r"C:\ffmpeg\bin")),
        ]
        .into_iter()
        .flatten()
        .collect()
    } else if cfg!(target_os = "macos") {
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .map(PathBuf::from)
            .to_vec()
    } else {
        let mut folders: Vec<PathBuf> = ["/usr/bin", "/usr/local/bin", "/snap/bin"].map(PathBuf::from).to_vec();
        folders.extend(var("HOME").map(|d| d.join(".local/bin")));
        folders
    }
}

/// Every folder to look for ffmpeg in: the viewer's PATH, the system's as it
/// is now, the usual installers' folders.
pub fn search_folders() -> Vec<PathBuf> {
    let own = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    joined(own.into_iter().chain(system_paths()).chain(known_folders()))
}

/// The ffmpeg programs to try, in order: the one chosen by hand alone when
/// there is one; else the one found in `folders`, then the project's pack.
pub fn candidates(chosen: Option<PathBuf>, folders: &[PathBuf], pack: Option<PathBuf>) -> Vec<PathBuf> {
    if let Some(chosen) = chosen {
        return vec![chosen];
    }
    let path = std::env::join_paths(folders)
        .ok()
        .and_then(|path| crate::recorder::find_on_path(&path));
    path.into_iter().chain(pack).collect()
}

/// Where the ffmpeg pack is installed: beside the media plugins.
pub fn pack_root() -> PathBuf {
    crate::settings::media_plugins_dir().with_file_name("ffmpeg")
}

/// The pack this viewer can download: `None` until the project publishes
/// one (`ffmpeg_pack::PACK` is filled in).
pub fn pack() -> Option<Published> {
    ffmpeg_pack::published(&ffmpeg_pack::PACK)
}

/// The pack's ffmpeg, when it is installed (reads the disk: off the frame).
pub fn pack_program() -> Option<PathBuf> {
    pack().and_then(|pack| ffmpeg_pack::installed(&pack_root(), &pack))
}

/// "Look again": how many times it was pressed, and whether the last look
/// found nothing either (the window then says so, instead of the same
/// lines as before).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LookAgain {
    pressed: u32,
    missing: bool,
}

impl LookAgain {
    pub fn pressed(&mut self) {
        self.pressed += 1;
        self.missing = false;
    }

    /// The look ended: found or not.
    pub fn answered(&mut self, found: bool) {
        self.missing = !found && self.pressed > 0;
        if found {
            self.pressed = 0;
        }
    }

    /// Times it was pressed for nothing, while the last one found nothing.
    pub fn in_vain(&self) -> Option<u32> {
        self.missing.then_some(self.pressed)
    }
}

/// The pack's download, as the window shows it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PackState {
    /// No pack is published for this viewer: no download is offered.
    #[default]
    Unavailable,
    /// It can be downloaded (`size` bytes); nothing happens before the user
    /// agrees.
    Offered {
        size: u64,
    },
    /// "Download ffmpeg" pressed: the question, with the size, waits.
    Asking {
        size: u64,
    },
    Downloading {
        received: u64,
        total: u64,
        preparing: bool,
    },
    Installed,
    Failed,
}

enum Event {
    Progress(Progress),
    Done(Result<PathBuf, Failure>),
}

/// The ffmpeg pack's download, from the question to its folder.
pub struct PackDownload {
    pack: Option<Published>,
    state: PackState,
    tx: std::sync::mpsc::Sender<Event>,
    rx: std::sync::mpsc::Receiver<Event>,
    cancel: Cancel,
}

impl Default for PackDownload {
    fn default() -> Self {
        PackDownload::new(pack(), false)
    }
}

impl PackDownload {
    /// `installed`: the pack's folder was found complete.
    pub fn new(pack: Option<Published>, installed: bool) -> PackDownload {
        let (tx, rx) = std::sync::mpsc::channel();
        let state = match pack {
            None => PackState::Unavailable,
            Some(_) if installed => PackState::Installed,
            Some(pack) => PackState::Offered { size: pack.size },
        };
        PackDownload {
            pack,
            state,
            tx,
            rx,
            cancel: Cancel::default(),
        }
    }

    pub fn state(&self) -> PackState {
        self.state
    }

    /// "Download ffmpeg": the question first, with the size.
    pub fn ask(&mut self) {
        if let (Some(pack), PackState::Offered { .. } | PackState::Failed) = (self.pack, self.state) {
            self.state = PackState::Asking { size: pack.size };
        }
    }

    /// The question answered "No".
    pub fn decline(&mut self) {
        if let (Some(pack), PackState::Asking { .. }) = (self.pack, self.state) {
            self.state = PackState::Offered { size: pack.size };
        }
    }

    /// The question answered "Download": the archive is fetched, checked
    /// and installed by `runtime`.
    pub fn start(&mut self, runtime: &tokio::runtime::Handle) {
        let (Some(pack), PackState::Asking { .. }) = (self.pack, self.state) else {
            return;
        };
        let root = pack_root();
        log::info!(
            "director: downloading the ffmpeg pack {} ({} MB) to {}",
            pack.version,
            pack.size / (1024 * 1024),
            root.display()
        );
        self.state = PackState::Downloading {
            received: 0,
            total: pack.size,
            preparing: false,
        };
        self.cancel = Cancel::default();
        let (cancel, tx) = (self.cancel.clone(), self.tx.clone());
        runtime.spawn(async move {
            let progress = tx.clone();
            let result = ffmpeg_pack::install(&Source::default(), &pack, &root, &cancel, move |p| {
                let _ = progress.send(Event::Progress(p));
            })
            .await;
            let _ = tx.send(Event::Done(result));
        });
    }

    /// Each frame: the ffmpeg program once the pack has just been installed.
    pub fn poll(&mut self) -> Option<PathBuf> {
        let mut installed = None;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Progress(p) => {
                    if matches!(self.state, PackState::Downloading { .. }) {
                        self.state = PackState::Downloading {
                            received: p.received,
                            total: p.total,
                            preparing: p.preparing,
                        };
                    }
                }
                Event::Done(Ok(program)) => {
                    log::info!("director: ffmpeg pack installed, {}", program.display());
                    self.state = PackState::Installed;
                    installed = Some(program);
                }
                Event::Done(Err(failure)) => {
                    log::warn!("director: the ffmpeg pack could not be installed ({failure:?})");
                    self.state = PackState::Failed;
                }
            }
        }
        installed
    }
}

impl Drop for PackDownload {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registry_path_has_its_variables_replaced() {
        let var = |name: &str| match name {
            "LOCALAPPDATA" => Some(r"C:\Users\u\AppData\Local".to_owned()),
            "SystemRoot" => Some(r"C:\Windows".to_owned()),
            _ => None,
        };
        assert_eq!(
            expand(r"%SystemRoot%\system32;%LOCALAPPDATA%\Microsoft\WinGet\Links", var),
            r"C:\Windows\system32;C:\Users\u\AppData\Local\Microsoft\WinGet\Links"
        );
        assert_eq!(expand("%NOPE%\\bin", var), "%NOPE%\\bin");
        assert_eq!(expand("50% off", var), "50% off");
        assert_eq!(expand("%%", var), "%%");
        assert_eq!(expand("", var), "");
    }

    #[test]
    fn each_folder_is_searched_once() {
        let folders = joined([
            PathBuf::from("/a"),
            PathBuf::from(""),
            PathBuf::from("/b/"),
            PathBuf::from("/a"),
            PathBuf::from("/b"),
        ]);
        assert_eq!(folders, [PathBuf::from("/a"), PathBuf::from("/b/")]);
    }

    #[test]
    fn an_ffmpeg_installed_meanwhile_is_found_by_looking_again() {
        let folder = std::env::temp_dir().join(format!("aurora-ffmpeg-look-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).expect("fixture");
        let folders = [PathBuf::from("/nowhere"), folder.clone()];
        let pack = PathBuf::from("pack/bin/ffmpeg");
        // nothing there yet: the pack only
        assert_eq!(candidates(None, &folders, Some(pack.clone())), std::slice::from_ref(&pack));
        assert!(candidates(None, &folders, None).is_empty());
        let mut look = LookAgain::default();
        look.pressed();
        look.answered(false);
        assert_eq!(look.in_vain(), Some(1), "the window says the look found nothing");
        // installed: the next look finds it, before the pack
        let program = folder.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
        std::fs::write(&program, b"").expect("fixture");
        assert_eq!(candidates(None, &folders, Some(pack.clone())), [program.clone(), pack]);
        look.pressed();
        assert_eq!(look.in_vain(), None, "while it looks");
        look.answered(true);
        assert_eq!(look.in_vain(), None);
        // the one chosen by hand is the only one tried
        assert_eq!(candidates(Some("x".into()), &folders, None), [PathBuf::from("x")]);
        // the first look, before any press, says nothing more
        let mut first = LookAgain::default();
        first.answered(false);
        assert_eq!(first.in_vain(), None);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn no_download_is_offered_before_a_pack_is_published() {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        // this viewer's own entry is empty
        assert_eq!(pack(), None);
        let mut download = PackDownload::new(None, false);
        assert_eq!(download.state(), PackState::Unavailable);
        download.ask();
        download.start(runtime.handle());
        assert_eq!(download.state(), PackState::Unavailable, "nothing to ask, nothing started");
        assert_eq!(download.poll(), None);
    }

    #[test]
    fn a_published_pack_is_asked_first_with_its_size() {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let published = ffmpeg_pack::published(&ffmpeg_pack::FfmpegPack {
            version: "1",
            size: 90_000_000,
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        });
        let mut download = PackDownload::new(published, false);
        assert_eq!(download.state(), PackState::Offered { size: 90_000_000 });
        // never started without the question answered
        download.start(runtime.handle());
        assert_eq!(download.state(), PackState::Offered { size: 90_000_000 });
        download.ask();
        assert_eq!(download.state(), PackState::Asking { size: 90_000_000 });
        download.decline();
        assert_eq!(download.state(), PackState::Offered { size: 90_000_000 });
        // what the worker reports ends in the state (no network here)
        download.state = PackState::Downloading {
            received: 0,
            total: 90_000_000,
            preparing: false,
        };
        download
            .tx
            .send(Event::Progress(Progress {
                received: 45_000_000,
                total: 90_000_000,
                preparing: false,
            }))
            .expect("fixture");
        assert_eq!(download.poll(), None);
        assert!(matches!(download.state(), PackState::Downloading { received: 45_000_000, .. }));
        download.tx.send(Event::Done(Err(Failure::Integrity))).expect("fixture");
        assert_eq!(download.poll(), None);
        assert_eq!(download.state(), PackState::Failed, "a mismatch installs nothing");
        download.ask();
        assert!(matches!(download.state(), PackState::Asking { .. }), "asked again to retry");
        download.tx.send(Event::Done(Ok(PathBuf::from("ffmpeg")))).expect("fixture");
        assert_eq!(download.poll(), Some(PathBuf::from("ffmpeg")));
        assert_eq!(PackDownload::new(published, true).state(), PackState::Installed);
    }
}
