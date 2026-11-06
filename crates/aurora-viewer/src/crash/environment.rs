//! Best-effort process and machine facts captured for crash diagnosis.
//!
//! Paths and command lines are deliberately excluded. Full bounded module
//! basenames and fixed classifications stay in the session folder; the
//! public report reduces them to third-party names and fixed device facts.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[cfg(target_os = "linux")]
const MAX_MODULES: usize = 1024;
#[cfg(target_os = "linux")]
const MAX_PROCESSES: usize = 4096;
static DISPLAYS: std::sync::OnceLock<std::sync::Mutex<Vec<Display>>> = std::sync::OnceLock::new();
static LOSS_CAPTURE_STARTED: AtomicBool = AtomicBool::new(false);
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Module {
    pub name: String,
    pub version: Option<String>,
    pub signer: String,
    pub class: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Display {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: Option<u32>,
    pub hdr: Option<bool>,
    pub vrr: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub modules_collected: bool,
    pub modules: Vec<Module>,
    pub processes_collected: bool,
    pub processes: Vec<String>,
    pub displays_collected: bool,
    pub displays: Vec<Display>,
    pub power_collected: bool,
    pub power_profile: Option<String>,
    pub power_source: Option<String>,
}

pub fn start(dir: &Path) {
    collect(dir, "start", false);
}

pub fn at_gpu_loss() {
    if let Some(session) = super::SESSION.get()
        && take_capture_slot(&LOSS_CAPTURE_STARTED)
    {
        collect(&session.dir, "loss", true);
    }
}

fn take_capture_slot(captured: &AtomicBool) -> bool {
    captured.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok()
}

pub fn note_winit_displays(displays: impl IntoIterator<Item = Display>) {
    let displays: Vec<_> = displays.into_iter().take(32).collect();
    *DISPLAYS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = displays.clone();
    if let Some(session) = super::SESSION.get()
        && let Ok(bytes) = serde_json::to_vec(&displays)
        && let Err(error) = atomic_write(&session.dir.join("environment-displays-start.json"), &bytes)
    {
        log::warn!("winit display snapshot was not published: {error}");
    }
}

fn collect(dir: &Path, phase: &'static str, at_loss: bool) {
    let dir = dir.to_owned();
    let spawned = std::thread::Builder::new()
        .name("aurora-environment-snapshot".into())
        .spawn(move || {
            let mut snapshot = collect_platform();
            if at_loss {
                let fallback = DISPLAYS
                    .get()
                    .and_then(|displays| displays.lock().ok().map(|displays| displays.clone()))
                    .unwrap_or_default();
                snapshot.displays = merge_display_fallback(snapshot.displays, fallback);
                snapshot.displays_collected |= !snapshot.displays.is_empty();
            }
            if let Ok(bytes) = serde_json::to_vec(&snapshot)
                && let Err(error) = atomic_write(&dir.join(format!("environment-{phase}.json")), &bytes)
            {
                log::warn!("environment snapshot was not published: {error}");
            }
        });
    if let Err(error) = spawned {
        log::warn!("environment snapshot was not started: {error}");
    }
}

fn merge_display_fallback(native: Vec<Display>, fallback: Vec<Display>) -> Vec<Display> {
    if native.is_empty() {
        return fallback;
    }
    native
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(bytes)?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(target_os = "linux")]
fn collect_platform() -> Snapshot {
    let mut snapshot = Snapshot::default();
    let maps = std::fs::read_to_string("/proc/self/maps");
    if let Ok(maps) = maps {
        snapshot.modules_collected = true;
        snapshot.modules = parse_linux_modules(&maps);
    }
    if let Some(processes) = linux_overlay_processes() {
        snapshot.processes_collected = true;
        snapshot.processes = processes;
    }
    snapshot.power_profile = linux_power_profile();
    snapshot.power_source = linux_power_source();
    snapshot.power_collected =
        std::path::Path::new("/sys/firmware/acpi/platform_profile").exists() || std::path::Path::new("/sys/class/power_supply").exists();
    snapshot
}

#[cfg(windows)]
fn collect_platform() -> Snapshot {
    super::environment_windows::collect()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn collect_platform() -> Snapshot {
    Snapshot::default()
}

#[cfg(target_os = "linux")]
fn parse_linux_modules(maps: &str) -> Vec<Module> {
    use std::collections::BTreeMap;

    let mut found = BTreeMap::new();
    for line in maps.lines() {
        let Some(path) = line.split_whitespace().nth(5) else {
            continue;
        };
        if !path.starts_with('/') || !is_shared_object(path) {
            continue;
        }
        let Some(name) = Path::new(path.trim_end_matches(" (deleted)"))
            .file_name()
            .and_then(|name| name.to_str())
        else {
            continue;
        };
        let Some(name) = safe_module_name(name) else {
            continue;
        };
        let class = classify_linux_module(path);
        found.entry(name.to_owned()).or_insert(Module {
            name: name.to_owned(),
            version: None,
            signer: "not available on Linux".into(),
            class: class.into(),
        });
        if found.len() >= MAX_MODULES {
            break;
        }
    }
    found.into_values().collect()
}

#[cfg(target_os = "linux")]
fn is_shared_object(path: &str) -> bool {
    path.contains(".so")
}

fn safe_module_name(name: &str) -> Option<&str> {
    (!name.is_empty() && name.len() <= 128 && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte)))
        .then_some(name)
}

#[cfg(target_os = "linux")]
fn classify_linux_module(path: &str) -> &'static str {
    let path = path.to_ascii_lowercase();
    if path.contains("/aurora-viewer/") || path.contains("/aurora/") {
        "Aurora"
    } else if path.contains("libstd-") || path.contains("libcore-") {
        "Rust standard library"
    } else if path.contains("/nvidia/") || path.contains("/mesa/") || path.contains("libvulkan_") {
        "GPU vendor"
    } else if ["/lib/", "/lib64/", "/usr/lib/", "/usr/lib64/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        "system"
    } else {
        "third-party"
    }
}

#[cfg(target_os = "linux")]
fn linux_overlay_processes() -> Option<Vec<String>> {
    const PROGRAMS: &[(&str, &[&str])] = &[
        ("Discord", &["discord", "discord-canary", "discord-ptb"]),
        ("OBS", &["obs", "obs-studio"]),
        ("Steam", &["steam", "steamwebhelper"]),
        ("RivaTuner", &["rtss"]),
        ("Overwolf", &["overwolf"]),
        ("Medal", &["medal"]),
        ("Xbox Game Bar", &["gamebar"]),
        ("NVIDIA overlay", &["nvidia-overlay", "nvidia-app"]),
        ("AMD overlay", &["radeonsoftware", "amdrssrv"]),
        ("Wallpaper Engine", &["wallpaper-engine"]),
        ("Bandicam", &["bandicam"]),
        ("MangoHud", &["mangohud"]),
    ];
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return None;
    };
    let names = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let pid = name.to_string_lossy();
            pid.bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| std::fs::read_to_string(entry.path().join("comm")).ok())
        })
        .take(MAX_PROCESSES)
        .flatten()
        .map(|name| name.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    Some(
        PROGRAMS
            .iter()
            .filter(|(_, aliases)| names.iter().any(|name| aliases.contains(&name.as_str())))
            .map(|(display, _)| (*display).to_owned())
            .collect(),
    )
}

#[cfg(target_os = "linux")]
fn linux_power_profile() -> Option<String> {
    let text = std::fs::read_to_string("/sys/firmware/acpi/platform_profile").ok()?;
    let profile = text.trim().to_ascii_lowercase();
    ["performance", "balanced", "low-power", "cool", "quiet"]
        .contains(&profile.as_str())
        .then_some(profile)
}

#[cfg(target_os = "linux")]
fn linux_power_source() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut battery = false;
    let mut ac_online = false;
    for entry in entries.flatten() {
        let root = entry.path();
        let kind = std::fs::read_to_string(root.join("type")).unwrap_or_default();
        match kind.trim() {
            "Battery" => battery = true,
            "Mains" | "USB" | "USB_C" => {
                ac_online |= std::fs::read_to_string(root.join("online")).is_ok_and(|v| v.trim() == "1");
            }
            _ => {}
        }
    }
    if battery && !ac_online {
        Some("battery".into())
    } else if ac_online {
        Some("AC".into())
    } else {
        None
    }
}

pub fn public_rows(snapshot: &Snapshot) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    let third_party = snapshot
        .modules
        .iter()
        .filter(|module| module.class == "third-party" && safe_public_module(module))
        .map(|module| module.name.as_str())
        .take(13)
        .collect::<Vec<_>>();
    let module_row = if !snapshot.modules_collected {
        "not collected: module list unavailable".to_owned()
    } else if third_party.is_empty() {
        "collected: no third-party modules identified".to_owned()
    } else {
        let extra = snapshot
            .modules
            .iter()
            .filter(|module| module.class == "third-party" && safe_public_module(module))
            .count()
            .saturating_sub(12);
        let names = third_party.into_iter().take(12).collect::<Vec<_>>().join(", ");
        if extra == 0 {
            format!("collected: {names}")
        } else {
            format!("collected: {names}, and {extra} more")
        }
    };
    rows.push(("Third-party modules in process", module_row));

    let programs = snapshot
        .processes
        .iter()
        .filter(|name| known_program(name))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let process_row = if !snapshot.processes_collected {
        "not collected: process names unavailable".to_owned()
    } else if programs.is_empty() {
        "collected: no known overlay programs found".to_owned()
    } else {
        format!("collected: {}", programs.into_iter().collect::<Vec<_>>().join(", "))
    };
    rows.push(("Overlay programs running", process_row));

    rows.push(("Displays", display_summary(&snapshot.displays, snapshot.displays_collected)));
    let power = if !snapshot.power_collected {
        "not collected: profile and power source unavailable".to_owned()
    } else {
        let profile = snapshot
            .power_profile
            .as_deref()
            .filter(|profile| known_power_profile(profile))
            .unwrap_or("not reported");
        let source = snapshot
            .power_source
            .as_deref()
            .filter(|source| matches!(*source, "AC" | "battery"))
            .unwrap_or("not reported");
        format!("collected: profile {profile}; power source {source}")
    };
    rows.push(("Power", power));
    rows
}

fn display_summary(displays: &[Display], collected: bool) -> String {
    if !collected {
        return "not collected: display information unavailable".to_owned();
    }
    if displays.is_empty() {
        return "collected: no displays reported".to_owned();
    }
    let displays = displays.iter().filter(|display| safe_display(display)).take(8).collect::<Vec<_>>();
    if displays.is_empty() {
        return "collected: no validated display modes".to_owned();
    }
    let modes = displays
        .iter()
        .enumerate()
        .map(|(index, display)| {
            let class = resolution_class(display.width, display.height);
            let refresh = match display.refresh_hz.filter(|hz| (20..=1000).contains(hz)) {
                Some(hz) => format!("{hz} Hz"),
                None => "refresh not reported".to_owned(),
            };
            let hdr = display_bool_word(display.hdr);
            let vrr = display_bool_word(display.vrr);
            format!("Display {} {class} {refresh} HDR {hdr} VRR {vrr}", index + 1)
        })
        .collect::<Vec<_>>();
    let rates = displays
        .iter()
        .filter_map(|display| display.refresh_hz.filter(|hz| (20..=1000).contains(hz)))
        .collect::<std::collections::BTreeSet<_>>();
    let mixed = if displays
        .iter()
        .any(|display| display.refresh_hz.filter(|hz| (20..=1000).contains(hz)).is_none())
    {
        "unknown"
    } else if rates.len() > 1 {
        "yes"
    } else {
        "no"
    };
    format!(
        "collected: {} displays ({}); mixed refresh {}",
        displays.len(),
        modes.join(", "),
        mixed
    )
}

fn display_bool_word(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "not reported",
    }
}

fn safe_display(display: &Display) -> bool {
    (640..=32_768).contains(&display.width) && (360..=16_384).contains(&display.height)
}

fn resolution_class(width: u32, height: u32) -> &'static str {
    if width >= 3840 || height >= 2160 {
        "4K or higher"
    } else if height >= 1440 {
        "1440p class"
    } else if height >= 1080 {
        "1080p class"
    } else {
        "below 1080p"
    }
}

fn known_program(name: &str) -> bool {
    matches!(
        name,
        "Discord"
            | "OBS"
            | "RivaTuner"
            | "MSI Afterburner"
            | "Overwolf"
            | "Medal"
            | "Xbox Game Bar"
            | "Steam"
            | "NVIDIA overlay"
            | "AMD overlay"
            | "Wallpaper Engine"
            | "Bandicam"
            | "FPS Monitor"
            | "ReShade"
            | "MangoHud"
    )
}

fn known_power_profile(profile: &str) -> bool {
    matches!(
        profile,
        "performance"
            | "balanced"
            | "low-power"
            | "cool"
            | "quiet"
            | "high performance"
            | "power saver"
            | "ultimate performance"
            | "custom/other power plan"
    )
}

#[cfg(any(windows, test))]
pub fn power_profile_from_guid(guid: &str) -> &'static str {
    match guid.to_ascii_lowercase().as_str() {
        "381b4222-f694-41f0-9685-ff5bb260df2e" => "balanced",
        "a1841308-3541-4fab-bc81-f71556f20b4a" => "power saver",
        "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c" => "high performance",
        "e9a42b02-d5df-448d-aa00-03f14749eb61" => "ultimate performance",
        _ => "custom/other power plan",
    }
}

/// Pure vocabulary and module-token checks used by the report and tests.
pub fn safe_public_module(module: &Module) -> bool {
    let known_labels = matches!(
        module.class.as_str(),
        "Aurora" | "GPU vendor" | "Microsoft" | "system" | "Rust standard library" | "third-party"
    ) && matches!(
        module.signer.as_str(),
        "Microsoft" | "GPU vendor" | "Aurora" | "Rust standard library" | "not available on Linux" | "unsigned" | "unknown" | "other"
    );
    let consistent = match module.signer.as_str() {
        "Microsoft" => module.class == "Microsoft",
        "GPU vendor" => module.class == "GPU vendor",
        "Aurora" => module.class == "Aurora",
        "Rust standard library" => module.class == "Rust standard library",
        "other" | "unsigned" => module.class == "third-party",
        // The Linux collector has no signer API. On Windows, catalog-signed
        // modules can lack an embedded signer, so the fixed path/name class
        // remains the best available evidence for an unknown signer.
        "not available on Linux" | "unknown" => true,
        _ => false,
    };
    safe_module_name(&module.name).is_some() && known_labels && consistent
}

#[cfg(any(windows, test))]
pub(super) fn module_class_for_signer(signer: &str, path_class: &'static str) -> &'static str {
    match signer {
        "Microsoft" => "Microsoft",
        "GPU vendor" => "GPU vendor",
        "Aurora" => "Aurora",
        "Rust standard library" => "Rust standard library",
        "other" | "unsigned" => "third-party",
        // Catalog-signed Windows files and system files may not have an
        // embedded signer to inspect. Keep path/name classification only for
        // those unknowns; a verified non-platform signer is always third-party.
        "unknown" => path_class,
        _ => "third-party",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_names_are_basenames_only_and_reject_paths_or_control_text() {
        assert_eq!(safe_module_name("libexample.so"), Some("libexample.so"));
        for bad in ["", "C:\\Users\\person\\private.dll", "module\nname.so", "name with spaces.so"] {
            assert_eq!(safe_module_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn public_module_classifier_accepts_only_known_class_and_signer_words() {
        let module = Module {
            name: "third-party.so".into(),
            version: None,
            signer: "unknown".into(),
            class: "third-party".into(),
        };
        assert!(safe_public_module(&module));
        let mut unsafe_module = module.clone();
        unsafe_module.class = "user-controlled path".into();
        assert!(!safe_public_module(&unsafe_module));
        unsafe_module = module.clone();
        unsafe_module.signer = "other".into();
        unsafe_module.class = "GPU vendor".into();
        assert!(!safe_public_module(&unsafe_module));
    }

    #[test]
    fn public_environment_rows_keep_only_third_party_names_fixed_programs_and_valid_modes() {
        let snapshot = Snapshot {
            modules_collected: true,
            modules: vec![
                Module {
                    name: "thirdparty.dll".into(),
                    version: None,
                    signer: "unknown".into(),
                    class: "third-party".into(),
                },
                Module {
                    name: "kernel32.dll".into(),
                    version: None,
                    signer: "unknown".into(),
                    class: "system".into(),
                },
            ],
            processes_collected: true,
            processes: vec!["Discord".into(), "PRIVATE_USER".into()],
            displays_collected: true,
            displays: vec![
                Display {
                    width: 1920,
                    height: 1080,
                    refresh_hz: Some(60),
                    hdr: Some(false),
                    vrr: Some(true),
                },
                Display {
                    width: 2560,
                    height: 1440,
                    refresh_hz: Some(144),
                    hdr: Some(true),
                    vrr: Some(true),
                },
                Display {
                    width: u32::MAX,
                    height: 1080,
                    refresh_hz: Some(u32::MAX),
                    hdr: None,
                    vrr: None,
                },
            ],
            power_collected: true,
            power_profile: Some("balanced".into()),
            power_source: Some("AC".into()),
        };
        let rows = public_rows(&snapshot);
        let joined = rows.iter().map(|(_, value)| value.as_str()).collect::<Vec<_>>().join(" | ");
        assert!(joined.contains("thirdparty.dll"));
        assert!(joined.contains("Discord"));
        assert!(joined.contains("1080p class"));
        assert!(joined.contains("mixed refresh yes"));
        assert!(joined.contains("profile balanced; power source AC"));
        assert!(!joined.contains("kernel32.dll"));
        assert!(!joined.contains("PRIVATE_USER"));
        assert!(!joined.contains("4294967295"));
    }

    #[test]
    fn mixed_refresh_is_unknown_when_any_display_rate_is_missing() {
        let snapshot = Snapshot {
            displays_collected: true,
            displays: vec![
                Display {
                    width: 1920,
                    height: 1080,
                    refresh_hz: Some(60),
                    hdr: None,
                    vrr: None,
                },
                Display {
                    width: 2560,
                    height: 1440,
                    refresh_hz: None,
                    hdr: None,
                    vrr: None,
                },
            ],
            ..Default::default()
        };
        let rows = public_rows(&snapshot);
        assert!(
            rows.iter()
                .any(|(name, value)| *name == "Displays" && value.ends_with("mixed refresh unknown"))
        );
    }

    #[test]
    fn native_displays_are_retained_instead_of_replaced_by_unmatched_winit_modes() {
        let merged = merge_display_fallback(
            vec![
                Display {
                    width: 3840,
                    height: 2160,
                    refresh_hz: Some(144),
                    hdr: Some(true),
                    vrr: Some(false),
                },
                Display {
                    width: 1920,
                    height: 1080,
                    refresh_hz: None,
                    hdr: Some(false),
                    vrr: None,
                },
            ],
            vec![
                Display {
                    width: 3840,
                    height: 2160,
                    refresh_hz: Some(60),
                    hdr: None,
                    vrr: None,
                },
                Display {
                    width: 1920,
                    height: 1080,
                    refresh_hz: Some(75),
                    hdr: None,
                    vrr: None,
                },
            ],
        );
        assert_eq!(merged[0].refresh_hz, Some(144));
        assert_eq!(merged[0].hdr, Some(true));
        assert_eq!(merged[0].vrr, Some(false));
        assert_eq!(merged[1].refresh_hz, None);
        assert_eq!(merged[1].hdr, Some(false));
    }

    #[test]
    fn concurrent_atomic_snapshot_writes_publish_one_complete_file() {
        let directory = std::env::temp_dir().join(format!("aurora-env-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("environment-loss.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let writers = ["a".repeat(16_384), "b".repeat(16_384)].map(|contents| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                atomic_write(&path, contents.as_bytes()).unwrap();
                contents
            })
        });
        barrier.wait();
        let contents = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            // The writers may not have published yet; join them before reading.
            String::new()
        });
        let expected = writers.map(|writer| writer.join().unwrap());
        let contents = if contents.is_empty() {
            std::fs::read_to_string(&path).unwrap()
        } else {
            contents
        };
        assert!(expected.contains(&contents), "snapshot was partial or mixed");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_the_first_confirmed_loss_claims_snapshot_capture() {
        let captured = AtomicBool::new(false);
        assert!(take_capture_slot(&captured));
        assert!(!take_capture_slot(&captured));
    }

    #[test]
    fn verified_module_signer_overrides_conflicting_path_classification() {
        assert_eq!(module_class_for_signer("Microsoft", "third-party"), "Microsoft");
        assert_eq!(module_class_for_signer("GPU vendor", "third-party"), "GPU vendor");
        assert_eq!(module_class_for_signer("other", "GPU vendor"), "third-party");
        assert_eq!(module_class_for_signer("unknown", "system"), "system");
    }

    #[test]
    fn known_windows_power_schemes_reduce_to_fixed_profile_labels() {
        assert_eq!(power_profile_from_guid("381B4222-F694-41F0-9685-FF5BB260DF2E"), "balanced");
        assert_eq!(power_profile_from_guid("user-created-guid"), "custom/other power plan");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_maps_keep_only_unique_shared_object_basenames() {
        let maps = "7f0 rw-p 0 00:00 0 /usr/lib/libc.so.6\n7f1 rw-p 0 00:00 0 /opt/custom/plugin.so (deleted)\n7f2 rw-p 0 00:00 0 /home/PRIVATE_USER/extra.so\n";
        let modules = parse_linux_modules(maps);
        assert_eq!(modules.len(), 3);
        assert!(
            modules
                .iter()
                .all(|module| !module.name.contains('/') && !module.name.contains("PRIVATE_USER"))
        );
        assert!(
            modules
                .iter()
                .any(|module| module.name == "plugin.so" && module.class == "third-party")
        );
    }
}
