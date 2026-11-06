use std::io::Read;

fn temp_tree(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("aurora-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn archive_entries(bytes: Vec<u8>) -> Vec<(String, String)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().unwrap().into_owned();
            let mut contents = String::new();
            file.read_to_string(&mut contents).unwrap();
            (name, contents)
        })
        .collect()
}

#[test]
fn build_bundle_reads_only_filtered_source_artifacts_and_excludes_private_session_files() {
    let root = temp_tree("repro-bundle-e2e");
    let settings_path = root.join("settings.json");
    let log_path = root.join("viewer.log");
    let session = root.join("session");
    std::fs::create_dir_all(&session).unwrap();

    let secrets = [
        "AVATAR_PRIVATE_71a8",
        "ACCOUNT_PRIVATE_c621",
        "PASSWORD_PRIVATE_47dd",
        "TOKEN_PRIVATE_963b",
        "LOGIN_PRIVATE_238f",
        "PATH_PRIVATE_540e",
        "MODULE_PRIVATE_3189",
        "PROCESS_PRIVATE_7f22",
        "POWER_PRIVATE_10c4",
        "FAULT_PRIVATE_8a11",
    ];
    let settings = serde_json::json!({
        "avatar_name": secrets[0],
        "account_name": secrets[1],
        "password": secrets[2],
        "token": secrets[3],
        "login_url": secrets[4],
        "log_path": format!("/home/alice/{}", secrets[5]),
        "draw_distance": 128,
        "shadows": true,
        "diagnostic_environment": {
            "use_preferences": true,
            "contact_variant": "PRIVATE_VARIANT",
            "vram_budget_mb": "4096",
            "present_mode": "fifo"
        }
    });
    std::fs::write(&settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();

    std::fs::write(
        &log_path,
        format!(
            "[2026-11-06T10:00:00.000Z ERROR aurora_render::main_thread] GPU device lost {};\n\
             [2026-11-06T10:00:01.000Z WARN aurora_net] chat {}: {}\n",
            secrets[5], secrets[0], secrets[2]
        ),
    )
    .unwrap();

    let samples = vec![serde_json::json!({
        "at_ms": 1000,
        "values": [16.7, 8.2, null, null, null, null, null, null, null, null, null, null, null, null, null],
        "passes": [
            {"name": "objects", "ms": 2.3},
            {"name": "water", "ms": 1.1},
            {"name": "post", "ms": 3.0}
        ],
        "window": "focused"
    })];
    let events = vec![serde_json::json!({
        "at_ms": 500,
        "kind": "region_changed",
        "from": 0,
        "to": 1
    })];
    let flight = serde_json::json!({
        "schema": 1,
        "trigger": "gpu_loss",
        "end_ms": 1000,
        "samples": samples,
        "events": events
    });
    std::fs::write(session.join("flight-recorder.json"), serde_json::to_vec(&flight).unwrap()).unwrap();
    std::fs::write(session.join("gpu-breadcrumbs.txt"), b"v1 5 1 1 3 100 8 1 0 6 2 1 2\n").unwrap();
    std::fs::write(
        session.join("environment-start.json"),
        serde_json::json!({
            "modules_collected": true,
            "modules": [{"name": secrets[6], "path": format!("/home/alice/{}", secrets[5])}],
            "processes_collected": true,
            "processes": [secrets[7]],
            "displays_collected": true,
            "displays": [{"width": 1920, "height": 1080, "refresh_hz": 60, "hdr": true, "vrr": false}],
            "power_collected": true,
            "power_profile": secrets[8]
        })
        .to_string(),
    )
    .unwrap();
    // Neither an arbitrary private file nor a raw report beside the session
    // artifacts is an input to the bundle builder.
    std::fs::write(session.join("gpu-device-fault-private.txt"), secrets[9]).unwrap();
    std::fs::write(session.join("public-report.md"), secrets[0]).unwrap();

    let entries = archive_entries(crate::deep_diagnostics::build_bundle(&settings_path, &log_path, Some(&session)).unwrap());
    assert_eq!(
        entries.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
        crate::deep_diagnostics::bundle_contents().to_vec()
    );
    let all = entries.iter().map(|(_, contents)| contents.as_str()).collect::<Vec<_>>().join("\n");
    for secret in secrets.into_iter().chain(["PRIVATE_VARIANT"]) {
        assert!(!all.contains(secret), "bundle leaked {secret}:\n{all}");
    }
    assert!(all.contains("GPU device lost"));
    assert!(all.contains("objects"));
    assert!(all.contains("module_count"));
    assert!(all.contains("1920"));
    assert!(all.contains("draw_distance"));
    assert!(all.contains("v1 5 1 1 3 100 8 1 0 6 2 1 2"));
    assert!(!all.contains("gpu-device-fault-private.txt"));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn build_bundle_degrades_malformed_and_oversized_source_files_to_safe_summaries() {
    let root = temp_tree("repro-bundle-bounds-e2e");
    let settings_path = root.join("settings.json");
    let log_path = root.join("viewer.log");
    let session = root.join("session");
    std::fs::create_dir_all(&session).unwrap();
    std::fs::write(&settings_path, b"not JSON").unwrap();
    std::fs::write(&log_path, b"[bad WARN aurora_net] timeout \n").unwrap();
    std::fs::write(session.join("flight-recorder.json"), b"{malformed").unwrap();
    std::fs::write(session.join("gpu-breadcrumbs.txt"), b"v1 PRIVATE_DRIVER_TEXT").unwrap();
    std::fs::write(session.join("environment-start.json"), b"[malformed").unwrap();

    let entries = archive_entries(crate::deep_diagnostics::build_bundle(&settings_path, &log_path, Some(&session)).unwrap());
    let content = entries.iter().map(|(_, contents)| contents.as_str()).collect::<Vec<_>>().join("\n");
    assert!(content.contains("\"status\":\"unavailable\""));
    assert!(content.contains("\"status\": \"unavailable\""));
    assert!(content.contains("timeout"));
    assert!(content.contains("unavailable"));

    std::fs::write(&settings_path, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    std::fs::write(session.join("flight-recorder.json"), vec![b'x'; 512 * 1024 + 1]).unwrap();
    std::fs::write(session.join("gpu-breadcrumbs.txt"), vec![b'x'; 513]).unwrap();
    std::fs::write(session.join("environment-start.json"), vec![b'x'; 128 * 1024 + 1]).unwrap();
    let entries = archive_entries(crate::deep_diagnostics::build_bundle(&settings_path, &log_path, Some(&session)).unwrap());
    let content = entries.iter().map(|(_, contents)| contents.as_str()).collect::<Vec<_>>().join("\n");
    assert!(content.contains("\"status\":\"unavailable\""));
    assert!(content.contains("\"status\": \"unavailable\""));
    assert!(content.len() < 1024 * 1024);

    std::fs::remove_dir_all(root).unwrap();
}
