//! One-session GPU diagnostics and the public-only repro bundle.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_LOG_TAIL: u64 = 256 * 1024;
const MAX_REPORT: u64 = 128 * 1024;
const MAX_ENVIRONMENT: u64 = 128 * 1024;
const MAX_BUNDLE: usize = 1024 * 1024;
const ENV_RESTORE_MARKER: &str = "AURORA_INTERNAL_DIAGNOSTICS_ENV_RESTORE";
const MAX_RESTORE_VALUE: usize = 2048;
const MAX_RESTORE_MARKER: usize = 24 * 1024;
const RESTORED_ENV: [&str; 9] = [
    "AURORA_GPU_VALIDATION",
    "AURORA_CONTACT_VARIANT",
    "AURORA_GPU_RECOVERY",
    "AURORA_VRAM_BUDGET_MB",
    "AURORA_CPU_CULL",
    "AURORA_PRESENT_MODE",
    "WGPU_DEBUG",
    "WGPU_VALIDATION_INDIRECT_CALL",
    "AURORA_GPU_BREADCRUMBS",
];

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

#[derive(Debug, Default)]
pub struct Startup {
    pub enabled: bool,
    pub environment: crate::settings::DiagnosticEnvironment,
}

/// Atomically consumes the opt-in before any worker thread or logger starts.
/// If the settings file is missing or malformed, diagnostics stay disabled.
pub fn consume_next_session(settings_path: &Path) -> std::io::Result<Startup> {
    let Ok(bytes) = std::fs::read(settings_path) else {
        return Ok(Startup::default());
    };
    let Ok(mut settings) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(Startup::default());
    };
    if !settings.is_object() {
        return Ok(Startup::default());
    }
    let environment = serde_json::from_value(settings["diagnostic_environment"].clone()).unwrap_or_default();
    let enabled = settings
        .get("deep_diagnostics_next_session")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Ok(Startup {
            enabled: false,
            environment,
        });
    }
    settings["deep_diagnostics_next_session"] = serde_json::Value::Bool(false);
    let parent = settings_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    crate::atomic_file::write(settings_path, &serde_json::to_vec_pretty(&settings).map_err(std::io::Error::other)?)?;
    Ok(Startup {
        enabled: true,
        environment,
    })
}

fn env_values(environment: &crate::settings::DiagnosticEnvironment) -> [(&'static str, Option<String>); 6] {
    let contact = match environment.contact_variant.as_str() {
        "no-early-out" | "no-shared" | "short" | "skip" => Some(environment.contact_variant.clone()),
        _ => None,
    };
    let budget = environment
        .vram_budget_mb
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|mb| (256..=1_048_576).contains(mb))
        .map(|mb| mb.to_string());
    let present = match environment.present_mode.as_str() {
        "immediate" | "mailbox" | "fifo" | "fifo-relaxed" => Some(environment.present_mode.clone()),
        _ => None,
    };
    [
        ("AURORA_GPU_VALIDATION", environment.gpu_validation.then(|| "1".to_owned())),
        ("AURORA_CONTACT_VARIANT", contact),
        ("AURORA_GPU_RECOVERY", environment.gpu_recovery.then(|| "1".to_owned())),
        ("AURORA_VRAM_BUDGET_MB", budget),
        ("AURORA_CPU_CULL", environment.cpu_cull.then(|| "1".to_owned())),
        ("AURORA_PRESENT_MODE", present),
    ]
}

fn configured_env_values(environment: &crate::settings::DiagnosticEnvironment) -> Option<[(&'static str, Option<String>); 6]> {
    environment.use_preferences.then(|| env_values(environment))
}

fn encode_restore_values(values: &BTreeMap<String, Option<String>>) -> Option<String> {
    if values
        .iter()
        .any(|(name, value)| !RESTORED_ENV.contains(&name.as_str()) || value.as_ref().is_some_and(|value| value.len() > MAX_RESTORE_VALUE))
    {
        return None;
    }
    let marker = serde_json::to_string(values).ok()?;
    (marker.len() <= MAX_RESTORE_MARKER).then_some(marker)
}

fn decode_restore_values(marker: &str) -> Option<BTreeMap<String, Option<String>>> {
    if marker.len() > MAX_RESTORE_MARKER {
        return None;
    }
    let values = serde_json::from_str::<BTreeMap<String, Option<String>>>(marker).ok()?;
    values
        .iter()
        .all(|(name, value)| RESTORED_ENV.contains(&name.as_str()) && value.as_ref().is_none_or(|value| value.len() <= MAX_RESTORE_VALUE))
        .then_some(values)
}

fn environment_snapshot() -> BTreeMap<String, Option<String>> {
    let mut values = BTreeMap::new();
    for name in RESTORED_ENV {
        match std::env::var_os(name) {
            None => {
                values.insert(name.to_owned(), None);
            }
            Some(value) => {
                let Ok(value) = value.into_string() else { continue };
                if value.len() <= MAX_RESTORE_VALUE {
                    values.insert(name.to_owned(), Some(value));
                }
            }
        }
    }
    values
}

/// Compute the environment passed to renderer initialization and the marker
/// inherited by child viewer launches. Kept pure to test the restart cycle
/// without mutating the test runner's process environment.
fn startup_environment_plan(
    values: &mut BTreeMap<String, Option<String>>,
    inherited_marker: Option<&str>,
    startup: &Startup,
) -> Option<String> {
    if let Some(restored) = inherited_marker.and_then(decode_restore_values) {
        values.extend(restored);
    }
    let configured = configured_env_values(&startup.environment);
    if configured.is_none() && !startup.enabled {
        return None;
    }
    let marker = encode_restore_values(values)?;
    if let Some(configured) = configured {
        for (name, value) in configured {
            if values.contains_key(name) {
                values.insert(name.to_owned(), value);
            }
        }
    }
    if startup.enabled {
        for (name, value) in [
            ("AURORA_GPU_VALIDATION", "1"),
            ("WGPU_DEBUG", "1"),
            ("WGPU_VALIDATION_INDIRECT_CALL", "1"),
            ("AURORA_GPU_BREADCRUMBS", "transfer"),
        ] {
            if values.contains_key(name) {
                values.insert(name.to_owned(), Some(value.to_owned()));
            }
        }
    }
    Some(marker)
}

const BUNDLE_NAMES: &[&str] = &[
    "CONTENTS.txt",
    "viewer-log-summary.txt",
    "last-crash-report.md",
    "flight-recorder.json",
    "gpu-breadcrumbs.txt",
    "environment-snapshot.json",
    "settings-redacted.json",
];

#[cfg(test)]
pub fn bundle_contents() -> &'static [&'static str] {
    BUNDLE_NAMES
}

/// The summary shown before the file picker opens.
pub fn confirmation_text() -> String {
    format!(
        "{}\n\n{}",
        aurora_i18n::tr!("deep-diagnostics-bundle-confirm", files = BUNDLE_NAMES.join(", ")),
        aurora_i18n::tr!("deep-diagnostics-bundle-privacy")
    )
}

/// Build a bounded ZIP from allowlisted public artifacts. The full settings,
/// private crash files and raw GPU fault artifact are intentionally excluded.
pub fn build_bundle(settings_path: &Path, log_path: &Path, session_dir: Option<&Path>) -> std::io::Result<Vec<u8>> {
    let log = read_tail(log_path, MAX_LOG_TAIL).unwrap_or_default();
    let log_summary = crate::crash::privacy::excerpt(&String::from_utf8_lossy(&log));
    let crash_report = crate::crash::latest_public_report().unwrap_or_else(|| "No saved public crash report is available.\n".to_owned());
    let flight = session_dir
        .and_then(|dir| read_bounded(&dir.join("flight-recorder.json"), 512 * 1024).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|json| crate::crash::flight_recorder::canonical_dump_json(&json))
        .unwrap_or_else(|| "{\"status\":\"unavailable\"}".to_owned());
    let breadcrumbs = session_dir
        .and_then(|dir| read_bounded(&dir.join("gpu-breadcrumbs.txt"), 512).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| aurora_render::gpu_diagnostics::Report::decode_public(&text))
        .map(|report| report.encode_public())
        .unwrap_or_else(|| "unavailable\n".to_owned());
    let environment = session_dir
        .map(environment_summary)
        .unwrap_or_else(|| serde_json::json!({ "status": "unavailable" }));
    let settings = redacted_settings(settings_path);
    archive_bundle(log_summary, crash_report, flight, breadcrumbs, environment, settings)
}

fn archive_bundle(
    log_summary: String,
    crash_report: String,
    flight: String,
    breadcrumbs: String,
    environment: serde_json::Value,
    settings: serde_json::Value,
) -> std::io::Result<Vec<u8>> {
    let contents = format!(
        "{}\n\n{}",
        aurora_i18n::tr!(
            "deep-diagnostics-bundle-manifest",
            files = BUNDLE_NAMES.iter().skip(1).copied().collect::<Vec<_>>().join(", ")
        ),
        aurora_i18n::tr!("deep-diagnostics-bundle-privacy")
    );
    let files = [
        ("CONTENTS.txt", contents.into_bytes()),
        ("viewer-log-summary.txt", log_summary.into_bytes()),
        ("last-crash-report.md", truncate_text(crash_report, MAX_REPORT as usize)),
        ("flight-recorder.json", truncate_text(flight, 512 * 1024)),
        ("gpu-breadcrumbs.txt", truncate_text(breadcrumbs, 512)),
        (
            "environment-snapshot.json",
            serde_json::to_vec_pretty(&environment).map_err(std::io::Error::other)?,
        ),
        (
            "settings-redacted.json",
            serde_json::to_vec_pretty(&settings).map_err(std::io::Error::other)?,
        ),
    ];
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        writer.start_file(name, options).map_err(std::io::Error::other)?;
        writer.write_all(&bytes)?;
    }
    let bytes = writer.finish().map_err(std::io::Error::other)?.into_inner();
    if bytes.len() > MAX_BUNDLE {
        return Err(std::io::Error::other("repro bundle exceeds the size limit"));
    }
    Ok(bytes)
}

fn truncate_text(text: String, limit: usize) -> Vec<u8> {
    let mut bytes = text.into_bytes();
    bytes.truncate(limit);
    bytes
}

fn read_tail(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(limit)))?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    if len > limit
        && let Some(end) = bytes.iter().position(|byte| *byte == b'\n')
    {
        bytes.drain(..=end);
    }
    Ok(bytes)
}

fn read_bounded(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(std::io::Error::other("diagnostic artifact exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other("diagnostic artifact grew beyond the size limit"));
    }
    Ok(bytes)
}

fn redacted_settings(path: &Path) -> serde_json::Value {
    let Ok(raw) = read_bounded(path, 2 * 1024 * 1024) else {
        return serde_json::json!({ "status": "unavailable" });
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return serde_json::json!({ "status": "unavailable" });
    };
    let environment =
        serde_json::from_value::<crate::settings::DiagnosticEnvironment>(value["diagnostic_environment"].clone()).unwrap_or_default();
    let switches = env_values(&environment)
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.map_or(serde_json::Value::Null, serde_json::Value::String)))
        .collect::<serde_json::Map<_, _>>();
    let graphics = [
        "draw_distance",
        "time_of_day",
        "vsync",
        "shadows",
        "shadow_quality",
        "exposure",
        "glow",
        "bloom",
        "bloom_strength",
        "occlusion_culling",
        "lod_factor",
        "texture_budget_auto",
        "texture_budget_mb",
        "max_texture_resolution",
        "contact_shadows",
        "projector_shadows",
        "antialiasing",
        "render_scale_percent",
        "ao_method",
        "reflection_probes",
        "ssr",
    ]
    .into_iter()
    .filter_map(|key| {
        let value = &value[key];
        match value {
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some((key.to_owned(), value.clone())),
            _ => None,
        }
    })
    .collect::<serde_json::Map<_, _>>();
    serde_json::json!({
        "deep_diagnostics_next_session": false,
        "diagnostic_environment": {
            "use_preferences": environment.use_preferences,
            "switches": switches
        },
        "graphics": graphics
    })
}

fn environment_summary(dir: &Path) -> serde_json::Value {
    let mut snapshots = serde_json::Map::new();
    for phase in ["start", "loss"] {
        let path = dir.join(format!("environment-{phase}.json"));
        let Ok(bytes) = read_bounded(&path, MAX_ENVIRONMENT) else {
            continue;
        };
        let Ok(snapshot) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let displays = snapshot["displays"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|display| {
                let width = display["width"].as_u64()?;
                let height = display["height"].as_u64()?;
                Some(serde_json::json!({
                    "width": width,
                    "height": height,
                    "refresh_hz": display["refresh_hz"].as_u64(),
                    "hdr": display["hdr"].as_bool(),
                    "vrr": display["vrr"].as_bool()
                }))
            })
            .collect::<Vec<_>>();
        snapshots.insert(
            phase.to_owned(),
            serde_json::json!({
                "modules_collected": snapshot["modules_collected"].as_bool().unwrap_or(false),
                "module_count": snapshot["modules"].as_array().map_or(0, Vec::len),
                "processes_collected": snapshot["processes_collected"].as_bool().unwrap_or(false),
                "process_count": snapshot["processes"].as_array().map_or(0, Vec::len),
                "displays_collected": snapshot["displays_collected"].as_bool().unwrap_or(false),
                "displays": displays,
                "power_collected": snapshot["power_collected"].as_bool().unwrap_or(false)
            }),
        );
    }
    if snapshots.is_empty() {
        serde_json::json!({ "status": "unavailable" })
    } else {
        serde_json::Value::Object(snapshots)
    }
}

/// Apply preferences before threads start: these flags are read once by the
/// renderer or at module initialization.
pub fn apply_environment(startup: &Startup) {
    ACTIVE.store(startup.enabled, Ordering::Relaxed);
    let mut values = environment_snapshot();
    let inherited_marker = std::env::var_os(ENV_RESTORE_MARKER).and_then(|marker| marker.into_string().ok());
    let restore_marker = startup_environment_plan(&mut values, inherited_marker.as_deref(), startup);
    // SAFETY: called from the first lines of `main`, before any application
    // thread is started. The renderer reads these variables at initialization.
    unsafe {
        std::env::remove_var(ENV_RESTORE_MARKER);
        if let Some(marker) = restore_marker {
            std::env::set_var(ENV_RESTORE_MARKER, marker);
        }
        for (name, value) in values {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}

#[cfg(test)]
#[path = "deep_diagnostics/tests.rs"]
mod source_bundle_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_diagnostics_is_consumed_for_one_session_only() {
        let root = std::env::temp_dir().join(format!("aurora-deep-diagnostics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        std::fs::write(&path, br#"{"deep_diagnostics_next_session":true,"keep":"value"}"#).unwrap();

        assert!(consume_next_session(&path).unwrap().enabled);
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["deep_diagnostics_next_session"], false);
        assert_eq!(saved["keep"], "value");
        assert!(!consume_next_session(&path).unwrap().enabled);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_or_invalid_settings_do_not_enable_deep_diagnostics() {
        let root = std::env::temp_dir().join(format!("aurora-deep-diagnostics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        assert!(!consume_next_session(&path).unwrap().enabled);
        std::fs::write(&path, b"not json").unwrap();
        assert!(!consume_next_session(&path).unwrap().enabled);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diagnostic_preferences_map_to_the_existing_environment_switches() {
        let env = crate::settings::DiagnosticEnvironment {
            gpu_validation: true,
            contact_variant: "no-shared".into(),
            gpu_recovery: true,
            vram_budget_mb: "4096".into(),
            cpu_cull: true,
            present_mode: "fifo-relaxed".into(),
            ..Default::default()
        };
        assert_eq!(
            env_values(&env),
            [
                ("AURORA_GPU_VALIDATION", Some("1".into())),
                ("AURORA_CONTACT_VARIANT", Some("no-shared".into())),
                ("AURORA_GPU_RECOVERY", Some("1".into())),
                ("AURORA_VRAM_BUDGET_MB", Some("4096".into())),
                ("AURORA_CPU_CULL", Some("1".into())),
                ("AURORA_PRESENT_MODE", Some("fifo-relaxed".into())),
            ]
        );
    }

    #[test]
    fn diagnostic_environment_rejects_unknown_or_unbounded_values() {
        let env = crate::settings::DiagnosticEnvironment {
            contact_variant: "PRIVATE_PATH".into(),
            vram_budget_mb: "999999999999999999999".into(),
            present_mode: "PRIVATE_MODE".into(),
            ..Default::default()
        };
        let values = env_values(&env);
        assert!(values[1].1.is_none());
        assert!(values[3].1.is_none());
        assert!(values[5].1.is_none());
        assert!(!format!("{values:?}").contains("PRIVATE"));
        let too_small = crate::settings::DiagnosticEnvironment {
            vram_budget_mb: "64".into(),
            ..Default::default()
        };
        assert!(env_values(&too_small)[3].1.is_none());
    }

    #[test]
    fn unset_preferences_leave_inherited_environment_switches_untouched() {
        let inherited = crate::settings::DiagnosticEnvironment::default();
        assert!(configured_env_values(&inherited).is_none());
        let mut explicit = inherited;
        explicit.use_preferences = true;
        let values = configured_env_values(&explicit).unwrap();
        assert!(values.iter().all(|(_, value)| value.is_none()));
    }

    #[test]
    fn relaunch_restore_marker_preserves_only_bounded_known_environment_values() {
        let values = BTreeMap::from([
            ("AURORA_GPU_VALIDATION".to_owned(), None),
            ("WGPU_DEBUG".to_owned(), Some("wgpu_core=trace".to_owned())),
            ("AURORA_GPU_BREADCRUMBS".to_owned(), Some("auto".to_owned())),
        ]);
        let marker = encode_restore_values(&values).unwrap();
        assert_eq!(decode_restore_values(&marker), Some(values));

        let untrusted = BTreeMap::from([("API_TOKEN".to_owned(), Some("secret".to_owned()))]);
        assert!(encode_restore_values(&untrusted).is_none());
        let oversized = BTreeMap::from([("WGPU_DEBUG".to_owned(), Some("x".repeat(MAX_RESTORE_VALUE + 1)))]);
        assert!(encode_restore_values(&oversized).is_none());
        assert!(decode_restore_values(&"x".repeat(MAX_RESTORE_MARKER + 1)).is_none());
    }

    #[test]
    fn child_start_restores_original_environment_before_disabling_one_session_flags() {
        let original = BTreeMap::from([
            ("AURORA_GPU_VALIDATION".to_owned(), None),
            ("AURORA_CONTACT_VARIANT".to_owned(), Some("no-early-out".to_owned())),
            ("AURORA_GPU_RECOVERY".to_owned(), None),
            ("AURORA_VRAM_BUDGET_MB".to_owned(), None),
            ("AURORA_CPU_CULL".to_owned(), None),
            ("AURORA_PRESENT_MODE".to_owned(), None),
            ("WGPU_DEBUG".to_owned(), Some("wgpu_core=warn".to_owned())),
            ("WGPU_VALIDATION_INDIRECT_CALL".to_owned(), None),
            ("AURORA_GPU_BREADCRUMBS".to_owned(), Some("auto".to_owned())),
        ]);
        let mut inherited = original.clone();
        let first = Startup {
            enabled: true,
            ..Default::default()
        };
        let marker = startup_environment_plan(&mut inherited, None, &first).unwrap();
        assert_eq!(inherited["AURORA_GPU_VALIDATION"].as_deref(), Some("1"));
        assert_eq!(inherited["WGPU_DEBUG"].as_deref(), Some("1"));
        assert_eq!(inherited["AURORA_GPU_BREADCRUMBS"].as_deref(), Some("transfer"));

        let child = Startup::default();
        assert!(!child.enabled);
        let child_marker = startup_environment_plan(&mut inherited, Some(&marker), &child);
        assert!(child_marker.is_none());
        assert_eq!(inherited, original);
    }

    #[test]
    fn repro_bundle_has_listed_files_and_strips_personal_data_and_private_faults() {
        let root = std::env::temp_dir().join(format!("aurora-repro-bundle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let settings_path = root.join("settings.json");
        std::fs::write(
            &settings_path,
            br#"{"avatar_name":"AVATAR_SECRET","account_name":"ACCOUNT_SECRET","password":"PASSWORD_SECRET","token":"TOKEN_SECRET","login_url":"LOGIN_SECRET","log_path":"/home/USER_SECRET","gpu_adapter":"USER_SECRET","draw_distance":128,"shadows":true,"diagnostic_environment":{"use_preferences":true,"contact_variant":"PRIVATE_VARIANT","vram_budget_mb":"4096","present_mode":"fifo"}}"#,
        )
        .unwrap();
        let session = root.join("session");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(
            session.join("environment-start.json"),
            br#"{"modules_collected":true,"modules":[{"name":"MODULE_SECRET","path":"/private/MODULE_SECRET"}],"processes_collected":true,"processes":["PROCESS_SECRET"],"displays_collected":true,"displays":[{"width":1920,"height":1080,"refresh_hz":60,"hdr":true,"vrr":false}],"power_collected":true,"power_profile":"USER_SECRET"}"#,
        )
        .unwrap();
        std::fs::write(session.join("gpu-device-fault-private.txt"), "PRIVATE_GPU_FAULT_SECRET").unwrap();

        let settings = redacted_settings(&settings_path);
        assert_eq!(settings["graphics"]["draw_distance"], 128);
        assert_eq!(settings["graphics"]["shadows"], true);
        let environment = environment_summary(&session);
        let raw_log = concat!(
            "[2026-11-06T10:00:00.000Z ERROR aurora_render::main_thread] GPU device lost /home/USER_SECRET TOKEN_SECRET\n",
            "[2026-11-06T10:00:01.000Z WARN aurora_net] chat AVATAR_SECRET: PASSWORD_SECRET\n"
        );
        let bytes = archive_bundle(
            crate::crash::privacy::excerpt(raw_log),
            "Public crash summary: GPU device lost".into(),
            r#"{"schema":1,"trigger":"gpu_loss","end_ms":0,"samples":[],"events":[]}"#.into(),
            "unavailable\n".into(),
            environment,
            settings,
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().unwrap().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, BUNDLE_NAMES);
        let mut all = String::new();
        for name in names {
            let mut file = archive.by_name(&name).unwrap();
            file.read_to_string(&mut all).unwrap();
        }
        for secret in [
            "AVATAR_SECRET",
            "ACCOUNT_SECRET",
            "PASSWORD_SECRET",
            "TOKEN_SECRET",
            "LOGIN_SECRET",
            "USER_SECRET",
            "MODULE_SECRET",
            "PROCESS_SECRET",
            "PRIVATE_VARIANT",
            "PRIVATE_GPU_FAULT_SECRET",
            "gpu-device-fault-private.txt",
        ] {
            assert!(!all.contains(secret), "bundle leaked {secret}");
        }
        assert!(all.contains("module_count"));
        assert!(all.contains("1920"));
        assert!(all.contains("GPU device lost"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_inputs_are_bounded_before_parsing() {
        let root = std::env::temp_dir().join(format!("aurora-repro-bounds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("oversized.json");
        std::fs::write(&path, vec![b'x'; MAX_ENVIRONMENT as usize + 1]).unwrap();
        assert!(read_bounded(&path, MAX_ENVIRONMENT).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
