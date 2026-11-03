//! Discover executables and let the selected Blender manage its own profile and
//! native package format. No filesystem walk or child process runs on the frame.
use super::addon;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

const BRIDGE: &str = include_str!("../../../../blender/install_bridge.py");

#[cfg(windows)]
mod windows;

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Report {
    #[serde(default)]
    pub version: [u32; 3],
    pub plugin_version: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub profile: PathBuf,
    #[serde(default)]
    pub target: PathBuf,
    pub backup: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Installation {
    pub executable: PathBuf,
    pub report: Report,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Install,
    Update,
    Enable,
    Reinstall,
    Newer,
    Unsupported,
    Failed,
}

impl Installation {
    pub fn action(&self) -> Action {
        let report = &self.report;
        if report.error.is_some() {
            return Action::Failed;
        }
        if report.version < [2, 93, 0] {
            return Action::Unsupported;
        }
        match report.plugin_version.as_deref().and_then(semver) {
            None => Action::Install,
            Some(v) => match Some(v).cmp(&semver(addon::version())) {
                std::cmp::Ordering::Greater => Action::Newer,
                std::cmp::Ordering::Less => Action::Update,
                std::cmp::Ordering::Equal if !report.enabled => Action::Enable,
                _ => Action::Reinstall,
            },
        }
    }

    pub fn label(&self) -> String {
        let [a, b, c] = self.report.version;
        if a == 0 { "Blender".into() } else { format!("Blender {a}.{b}.{c}") }
    }

    pub fn path_label(&self) -> String {
        let path = self.executable.to_string_lossy();
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

fn semver(value: &str) -> Option<[u32; 3]> {
    let values: Vec<u32> = value.split('.').map(str::parse).collect::<Result<_, _>>().ok()?;
    values.try_into().ok()
}

enum Completed {
    Scan {
        installations: Vec<Installation>,
        manual: Vec<PathBuf>,
    },
    Install {
        executable: PathBuf,
        report: Report,
    },
    Cancelled,
    Error(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Phase {
    #[default]
    Choosing,
    Scanning,
    Checking,
    Installing,
}

#[derive(Default)]
pub(crate) struct Installer {
    pub scanned: bool,
    pub busy: bool,
    pub installations: Vec<Installation>,
    pub error: Option<String>,
    pub success: Option<Report>,
    manual: Vec<PathBuf>,
    result: Arc<Mutex<Option<Completed>>>,
    phase: Arc<Mutex<Phase>>,
}

impl Installer {
    pub(super) fn phase(&self) -> Phase {
        *self.phase.lock()
    }

    pub fn demo(&mut self, mode: &str) {
        let phase = match mode {
            "installer-choose" => Phase::Choosing,
            "installer-check" => Phase::Checking,
            "installer-install" => Phase::Installing,
            "installer-done" | "installer-error" => Phase::Installing,
            _ => return,
        };
        self.begin(phase);
        if matches!(mode, "installer-done" | "installer-error") {
            let report = Report {
                version: [5, 2, 1],
                enabled: mode == "installer-done",
                plugin_version: (mode == "installer-done").then(|| addon::version().into()),
                profile: "Blender/profile".into(),
                target: "Blender/profile/extensions/aurora_live_mesh".into(),
                error: (mode == "installer-error").then(|| aurora_i18n::tr!("blender-live-installer-no-report").into()),
                ..Default::default()
            };
            *self.result.lock() = Some(Completed::Install {
                executable: "Blender/blender.exe".into(),
                report,
            });
            self.poll();
        }
    }

    fn begin(&mut self, phase: Phase) -> bool {
        if self.busy {
            return false;
        }
        self.busy = true;
        self.error = None;
        self.success = None;
        *self.phase.lock() = phase;
        true
    }

    pub fn poll(&mut self) {
        let Some(completed) = self.result.lock().take() else { return };
        self.busy = false;
        match completed {
            Completed::Scan { installations, manual } => {
                self.installations = installations;
                self.manual = manual;
            }
            Completed::Install { executable, report } => {
                if let Some(entry) = self.installations.iter_mut().find(|i| i.executable == executable) {
                    entry.report = report.clone();
                } else {
                    self.installations.push(Installation {
                        executable,
                        report: report.clone(),
                    });
                }
                if let Some(error) = &report.error {
                    self.error = Some(error.clone());
                } else {
                    self.success = Some(report);
                }
            }
            Completed::Error(error) => self.error = Some(error),
            Completed::Cancelled => {}
        }
    }

    pub fn scan(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        if !self.begin(Phase::Scanning) {
            return;
        }
        self.scanned = true;
        let result = self.result.clone();
        let ctx = ctx.clone();
        let mut manual = self.manual.clone();
        jobs.execute(move || {
            if let Some(path) = remembered_paths()
                && let Ok(bytes) = std::fs::read(&path)
            {
                manual.extend(
                    serde_json::from_slice::<Vec<PathBuf>>(&bytes)
                        .unwrap_or_default()
                        .into_iter()
                        .take(64),
                );
            }
            manual = unique_executables(manual);
            *result.lock() = Some(Completed::Scan {
                installations: scan(&manual),
                manual,
            });
            ctx.request_repaint();
        });
    }

    pub fn choose(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        if !self.begin(Phase::Choosing) {
            return;
        }
        let result = self.result.clone();
        let phase = self.phase.clone();
        let ctx = ctx.clone();
        jobs.execute(move || {
            let completed = (|| {
                let picker = rfd::FileDialog::new().set_title(aurora_i18n::tr!("blender-live-choose-executable"));
                #[cfg(windows)]
                let picker = picker.add_filter("Blender", &["exe"]);
                let Some(path) = picker.pick_file() else {
                    return Ok(Completed::Cancelled);
                };
                let Some(executable) = unique_executables(vec![path]).pop() else {
                    return Err(aurora_i18n::tr!("blender-live-invalid-executable").into());
                };
                remember_selected(&executable)?;
                let report = install_selected(&executable, run_bridge, |value| {
                    *phase.lock() = value;
                    ctx.request_repaint();
                });
                Ok(Completed::Install { executable, report })
            })()
            .unwrap_or_else(Completed::Error);
            *result.lock() = Some(completed);
            ctx.request_repaint();
        });
    }

    pub fn install(&mut self, executable: PathBuf, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        if self.busy
            || !self.installations.iter().any(|i| {
                i.executable == executable && matches!(i.action(), Action::Install | Action::Update | Action::Enable | Action::Reinstall)
            })
        {
            return;
        }
        self.begin(Phase::Checking);
        let result = self.result.clone();
        let phase = self.phase.clone();
        let ctx = ctx.clone();
        jobs.execute(move || {
            let report = install_selected(&executable, run_bridge, |value| {
                *phase.lock() = value;
                ctx.request_repaint();
            });
            *result.lock() = Some(Completed::Install { executable, report });
            ctx.request_repaint();
        });
    }
}

/// Check and install only the requested executable, independently of discovery.
fn install_selected(executable: &Path, mut run: impl FnMut(&Path, bool) -> Result<Report, String>, mut phase: impl FnMut(Phase)) -> Report {
    phase(Phase::Checking);
    let mut report = match run(executable, false) {
        Ok(report) => report,
        Err(error) => {
            return Report {
                error: Some(error),
                ..Default::default()
            };
        }
    };
    let action = Installation {
        executable: executable.to_owned(),
        report: report.clone(),
    }
    .action();
    match action {
        Action::Failed => return report,
        Action::Unsupported | Action::Newer => {
            report.error = Some(if action == Action::Unsupported {
                aurora_i18n::tr!("blender-live-unsupported-version").into()
            } else {
                aurora_i18n::tr!("blender-live-newer").into()
            });
            return report;
        }
        _ => {}
    }
    phase(Phase::Installing);
    match run(executable, true) {
        Ok(mut installed) => {
            if installed.error.is_none() && (!installed.enabled || installed.plugin_version.as_deref() != Some(addon::version())) {
                installed.error = Some(aurora_i18n::tr!("blender-live-installer-no-report").into());
            }
            installed
        }
        Err(error) => Report {
            error: Some(error),
            ..report
        },
    }
}

fn remembered_paths() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer").map(|d| d.config_dir().join("blender-installations.json"))
}

fn remember_selected(executable: &Path) -> Result<(), String> {
    let Some(path) = remembered_paths() else { return Ok(()) };
    remember_at(&path, executable)
}

fn remember_at(path: &Path, executable: &Path) -> Result<(), String> {
    let mut paths = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
        .unwrap_or_default();
    // Do not inspect other executables or their drives when choosing one file.
    paths.retain(|p| executable_key(p) != executable_key(executable));
    paths.insert(0, executable.to_owned());
    paths.truncate(64);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(&paths).map_err(|e| e.to_string())?;
    crate::atomic_file::write(path, &bytes).map_err(|e| e.to_string())
}

fn executable_key(path: &Path) -> String {
    if cfg!(windows) {
        path.to_string_lossy().to_lowercase()
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn is_blender(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(if cfg!(windows) { "blender.exe" } else { "blender" }))
        && path.is_file()
}

fn unique_executables(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter_map(|p| p.canonicalize().ok())
        .filter(|p| is_blender(p))
        .filter(|p| seen.insert(executable_key(p)))
        .take(64)
        .collect()
}

/// Inspect only named application directories and one version-directory level.
/// Never recurse through disks, user documents or network shares.
fn candidates(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        paths.push(root.join(if cfg!(windows) { "blender.exe" } else { "blender" }));
        paths.push(root.join("Blender.app/Contents/MacOS/Blender"));
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten().take(256) {
                if !entry.file_name().to_string_lossy().to_ascii_lowercase().starts_with("blender") {
                    continue;
                }
                paths.push(entry.path().join(if cfg!(windows) { "blender.exe" } else { "blender" }));
                paths.push(entry.path().join("Contents/MacOS/Blender"));
            }
        }
    }
    paths
}

fn discovery_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        for key in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(path) = std::env::var_os(key) {
                let path = PathBuf::from(path);
                roots.push(path.join("Blender Foundation"));
                roots.push(path.join("Steam/steamapps/common/Blender"));
            }
        }
        if let Some(path) = std::env::var_os("LOCALAPPDATA") {
            roots.push(PathBuf::from(path).join("Programs/Blender Foundation"));
        }
        // Portable versions commonly live in <drive>/Blender/<version>.
        // GetLogicalDrives excludes absent volumes; remote volumes are excluded.
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
                System::WindowsProgramming::DRIVE_FIXED,
            };
            // SAFETY: no pointers, the return value is only a drive-presence mask.
            let mask = unsafe { GetLogicalDrives() };
            for letter in b'C'..=b'Z' {
                if mask & (1 << (letter - b'A')) == 0 {
                    continue;
                }
                let path = format!("{}:\\", letter as char);
                let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
                // SAFETY: wide is a live, NUL-terminated UTF-16 string.
                if unsafe { GetDriveTypeW(wide.as_ptr()) } != DRIVE_FIXED {
                    continue;
                }
                for folder in ["Blender", "Blender Foundation", "Apps", "Programs", "PortableApps"] {
                    roots.push(PathBuf::from(&path).join(folder));
                }
            }
        }
    } else {
        roots.extend(["/Applications", "/opt", "/usr/bin", "/usr/local/bin", "/snap/blender/current"].map(PathBuf::from));
        if let Some(home) = directories::BaseDirs::new() {
            roots.extend(["Applications", "blender", ".local/bin"].map(|p| home.home_dir().join(p)));
        }
    }
    roots
}

fn scan(manual: &[PathBuf]) -> Vec<Installation> {
    let mut paths = candidates(&discovery_roots());
    #[cfg(windows)]
    paths.extend(windows::registered());
    paths.extend_from_slice(manual);
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).map(|p| p.join(if cfg!(windows) { "blender.exe" } else { "blender" })));
    }
    let paths = unique_executables(paths);
    log::info!("Blender discovery: probing {} local executables", paths.len());
    // At most two Blender processes, independent of the renderer's worker count.
    let mut installations = Vec::new();
    for pair in paths.chunks(2) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = pair
                .iter()
                .map(|path| {
                    scope.spawn(move || Installation {
                        executable: path.clone(),
                        report: run_bridge(path, false).unwrap_or_else(|error| Report {
                            error: Some(error),
                            ..Default::default()
                        }),
                    })
                })
                .collect();
            for handle in handles {
                if let Ok(entry) = handle.join() {
                    installations.push(entry);
                }
            }
        });
    }
    installations.sort_by(|a, b| b.report.version.cmp(&a.report.version).then(a.executable.cmp(&b.executable)));
    log::info!("Blender discovery: {} installations ready", installations.len());
    installations
}

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("aurora-blender-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Serialize)]
struct Request<'a> {
    operation: &'a str,
    plugin_version: &'a str,
    extension_zip: PathBuf,
    legacy_zip: PathBuf,
}

fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}

fn run_bridge(executable: &Path, install: bool) -> Result<Report, String> {
    bridge(command(executable), install)
}

fn bridge(mut command: Command, install: bool) -> Result<Report, String> {
    let temporary = Temporary::new()?;
    let root = &temporary.0;
    std::fs::write(root.join("bridge.py"), BRIDGE).map_err(|e| e.to_string())?;
    let request = Request {
        operation: if install { "install" } else { "probe" },
        plugin_version: addon::version(),
        extension_zip: root.join("extension.zip"),
        legacy_zip: root.join("legacy.zip"),
    };
    if install {
        std::fs::write(&request.extension_zip, addon::archive(false)?).map_err(|e| e.to_string())?;
        std::fs::write(&request.legacy_zip, addon::archive(true)?).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    std::fs::write(root.join("request.json"), bytes).map_err(|e| e.to_string())?;
    command
        .args(["--background", "--disable-autoexec", "--python-exit-code", "1", "--python"])
        .arg(root.join("bridge.py"))
        .arg("--")
        .arg(root.join("request.json"))
        .arg(root.join("report.json"));
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(if install { 180 } else { 45 });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match other {
                    Err(error) => error.to_string(),
                    _ => aurora_i18n::tr!("blender-live-installer-timeout").into(),
                });
            }
        }
    };
    let bytes = std::fs::read(root.join("report.json")).map_err(|_| aurora_i18n::tr!("blender-live-installer-no-report").to_owned())?;
    let report: Report = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !status.success() && report.error.is_none() {
        return Err(aurora_i18n::tr!("blender-live-installer-no-report").into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
