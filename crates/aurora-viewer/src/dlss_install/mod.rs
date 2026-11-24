//! NVIDIA DLSS installed from the viewer (Windows x86-64 and Linux x86-64,
//! a `dlss` build): after the user accepts NVIDIA's licence, the files
//! aurora-render's dlss.rs looks for are put in its folder, without the
//! user running anything outside the viewer. It does what
//! scripts/dlss-setup.ps1 and scripts/dlss-setup.sh do, with the same
//! sources and checks:
//!
//! - Windows: NVIDIA's signed Streamline release (windows.rs). Only the
//!   files the viewer needs are read out of the release's zip, by HTTP
//!   range requests (about 32 MB of its 276 MB), each checked against its
//!   SHA-256 and NVIDIA's signature;
//! - Linux: NVIDIA's DLSS SDK files, and sl-ngx (MIT) built against them
//!   with the system's `g++` (linux.rs).
//!
//! Every file comes from a pinned release or commit and is checked against
//! its SHA-256 before use; nothing of NVIDIA's is shipped with the viewer.
//! Streamline is started with the device, at start only, so DLSS draws
//! after a restart.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
use windows as platform;

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

/// NVIDIA's licence, as the user accepted it, in the folder (the scripts'
/// name for it).
const LICENCE_NAME: &str = "NVIDIA-DLSS-LICENSE.txt";

#[cfg_attr(windows, allow(dead_code))]
/// A file fetched by URL: its path under the release's base URL, where it
/// goes in the work folder, and what it must be.
struct Remote {
    path: &'static str,
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

#[cfg_attr(windows, allow(dead_code))]
const fn remote(path: &'static str, name: &'static str, size: u64, sha256: &'static str) -> Remote {
    Remote { path, name, size, sha256 }
}

/// What the installation is doing, shown by the dialog in the user's language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Starting,
    Downloading,
    /// (Linux: sl-ngx built with g++)
    #[cfg_attr(windows, allow(dead_code))]
    Building,
    Installing,
}

/// Where the installation is, for the Preferences dialog.
#[derive(Clone, Debug, Default)]
pub enum State {
    /// Nothing asked.
    #[default]
    Idle,
    /// The licence is being fetched to be shown.
    Fetching,
    /// NVIDIA's licence, waiting for Accept or Cancel.
    Licence(String),
    /// Downloading or building: which step, and how far (0..1).
    Working(Step, f32),
    /// The files are in place: a restart turns DLSS on.
    Done,
    Failed(String),
}

#[derive(Default)]
pub struct Installer {
    state: Arc<Mutex<State>>,
}

impl Installer {
    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn active(&self) -> bool {
        !matches!(self.state(), State::Idle)
    }

    pub fn close(&self) {
        let mut s = self.state.lock().unwrap();
        if !matches!(*s, State::Working(..) | State::Fetching) {
            *s = State::Idle;
        }
    }

    /// "Install NVIDIA DLSS…": fetches the licence to show.
    pub fn ask(&self) {
        self.set(State::Fetching);
        let state = self.state.clone();
        spawn(
            move || {
                let text = platform::licence(&Net::new()?)?.replace('\r', "");
                *state.lock().unwrap() = State::Licence(text);
                Ok(())
            },
            self.state.clone(),
        );
    }

    /// "Accept and install": downloads, checks and installs into `folder`.
    pub fn install(&self, folder: PathBuf) {
        let licence = match self.state() {
            State::Licence(text) => text,
            _ => return,
        };
        self.set(State::Working(Step::Starting, 0.0));
        let state = self.state.clone();
        spawn(
            move || {
                install(&folder, &licence, &state)?;
                *state.lock().unwrap() = State::Done;
                log::info!("DLSS: installed in {}", folder.display());
                Ok(())
            },
            self.state.clone(),
        );
    }

    fn set(&self, s: State) {
        *self.state.lock().unwrap() = s;
    }
}

fn spawn(work: impl FnOnce() -> Result<(), String> + Send + 'static, state: Arc<Mutex<State>>) {
    std::thread::Builder::new()
        .name("dlss-install".into())
        .spawn(move || {
            if let Err(e) = work() {
                log::warn!("DLSS install: {e}");
                *state.lock().unwrap() = State::Failed(e);
            }
        })
        .expect("thread");
}

/// The installer thread's HTTP client, on a runtime of its own.
struct Net {
    rt: tokio::runtime::Runtime,
    client: reqwest::Client,
}

impl Net {
    fn new() -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("AuroraViewer/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { rt, client })
    }

    #[cfg_attr(windows, allow(dead_code))]
    /// `base`/`r.path`, checked against its size and SHA-256.
    fn download(&self, base: &str, r: &Remote, mut progress: impl FnMut(u64)) -> Result<Vec<u8>, String> {
        let url = format!("{base}/{}", r.path);
        let name = r.path.rsplit('/').next().unwrap_or(r.path);
        let out = self.rt.block_on(async {
            let mut resp = self
                .client
                .get(&url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("{name}: {e}"))?;
            let mut out = Vec::with_capacity(r.size as usize);
            while let Some(chunk) = resp.chunk().await.map_err(|e| format!("{name}: {e}"))? {
                out.extend_from_slice(&chunk);
                if out.len() as u64 > r.size {
                    return Err(format!("{name} is larger than expected"));
                }
                progress(out.len() as u64);
            }
            Ok(out)
        })?;
        if out.len() as u64 != r.size || sha256(&out) != r.sha256 {
            return Err(format!("{name} is not the file expected from {base}"));
        }
        Ok(out)
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What the platform's steps report: the step, and how far (0..1).
type Report<'a> = &'a dyn Fn(Step, f32);

fn install(folder: &Path, licence: &str, state: &Mutex<State>) -> Result<(), String> {
    let report = |step: Step, f: f32| *state.lock().unwrap() = State::Working(step, f);
    // (beside the folder, on its filesystem: each file then moves in one step)
    let work = folder.with_extension("installing");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let result = (|| {
        platform::prepare(&Net::new()?, &work, &report)?;
        std::fs::write(work.join(LICENCE_NAME), licence).map_err(|e| format!("{LICENCE_NAME}: {e}"))?;

        report(Step::Installing, 0.98);
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        for name in platform::INSTALLED.into_iter().chain([LICENCE_NAME]) {
            let to = folder.join(name);
            let _ = std::fs::remove_file(&to);
            std::fs::rename(work.join(name), &to).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        // what an earlier setup left in the folder in their place
        platform::remove_old(folder);
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// A restart asked from the dialog, done as the viewer ends (main.rs).
static RESTART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn request_restart() {
    RESTART.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn restart_requested() -> bool {
    RESTART.load(std::sync::atomic::Ordering::Relaxed)
}
