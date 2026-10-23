use super::*;

#[test]
fn blender_live_installer_selects_native_package_actions_without_downgrading() {
    let mut entry = Installation {
        executable: PathBuf::from("blender"),
        report: Report {
            version: [5, 2, 1],
            ..Default::default()
        },
    };
    assert_eq!(entry.action(), Action::Install);
    entry.report.plugin_version = Some("0.8.6".into());
    assert_eq!(entry.action(), Action::Update);
    entry.report.plugin_version = Some(addon::version().into());
    assert_eq!(entry.action(), Action::Enable);
    entry.report.enabled = true;
    assert_eq!(entry.action(), Action::Reinstall);
    let [major, minor, patch] = semver(addon::version()).expect("bundled version");
    entry.report.plugin_version = Some(format!("{major}.{}.{patch}", minor + 1));
    assert_eq!(entry.action(), Action::Newer);
    assert!(semver("0.10.0") > semver("0.9.0"), "numeric comparison, not lexical");
    entry.report.version = [2, 92, 0];
    assert_eq!(entry.action(), Action::Unsupported);
    entry.report.error = Some("probe failed".into());
    assert_eq!(entry.action(), Action::Failed);
    assert_eq!(semver("0.8.7.1"), None);
    assert_eq!(semver("not a version"), None);
}

#[test]
fn blender_live_installer_discovery_is_bounded_deduplicated_and_accepts_spaces() {
    let temporary = Temporary::new().expect("directory");
    let root = &temporary.0;
    let executable = if cfg!(windows) { "blender.exe" } else { "blender" };
    for name in ["Blender 5.2", "blender-4.3-portable", "Unrelated app", "Blender nested/Blender 4.2"] {
        let path = root.join(name);
        std::fs::create_dir_all(&path).expect("directory");
        std::fs::write(path.join(executable), []).expect("fixture");
    }
    let mut paths = candidates(std::slice::from_ref(root));
    paths.push(root.join("Blender 5.2").join(executable));
    paths.push(root.join("not-present").join(executable));
    let paths = unique_executables(paths);
    assert_eq!(paths.len(), 2, "no unrelated applications or recursive disk traversal");
    assert!(paths.iter().any(|p| p.to_string_lossy().contains("Blender 5.2")));
}

/// Runs the actual compiled viewer packaging/process path in disposable profiles.
/// Example: AURORA_TEST_BLENDER=<absolute blender.exe>; cargo test --workspace
/// blender_live_installer_native -- --ignored --nocapture
#[test]
#[ignore = "requires AURORA_TEST_BLENDER; launches Blender in an isolated profile"]
fn blender_live_installer_native_probe_install_and_reinstall_preserve_previous_files() {
    let executable = PathBuf::from(std::env::var_os("AURORA_TEST_BLENDER").expect("AURORA_TEST_BLENDER"));
    let profile = Temporary::new().expect("isolated profile");
    let config = profile.0.join("profile/config");
    for directory in [&config, &profile.0.join("profile/scripts"), &profile.0.join("profile/extensions")] {
        std::fs::create_dir_all(directory).expect("isolate every Blender user directory before starting it");
    }
    let run = |install| {
        let mut command = command(&executable);
        command
            .env("BLENDER_USER_CONFIG", &config)
            .env("BLENDER_USER_SCRIPTS", profile.0.join("profile/scripts"))
            .env("BLENDER_USER_EXTENSIONS", profile.0.join("profile/extensions"))
            .env("BLENDER_USER_RESOURCES", profile.0.join("profile"));
        bridge(command, install).expect("bridge report")
    };
    let initial = run(false);
    assert!(initial.error.is_none(), "{:?}", initial.error);
    assert!(initial.plugin_version.is_none());
    assert!(!config.join("userpref.blend").exists(), "probe never saves preferences");
    let installed = run(true);
    assert!(installed.error.is_none(), "{:?}", installed.error);
    assert_eq!(installed.plugin_version.as_deref(), Some(addon::version()));
    assert!(installed.enabled);
    assert!(installed.target.join("__init__.py").is_file());
    let preferences = std::fs::read(config.join("userpref.blend")).expect("persisted enabled add-on");
    let fresh_probe = run(false);
    assert!(fresh_probe.enabled, "enabled after restarting Blender");
    std::fs::write(installed.target.join("previous-marker.txt"), b"previous Aurora package").expect("marker");
    let reinstalled = run(true);
    assert!(reinstalled.error.is_none(), "{:?}", reinstalled.error);
    let backup = reinstalled.backup.expect("previous version backup");
    assert_eq!(
        std::fs::read(backup.join("package/previous-marker.txt")).expect("backup"),
        b"previous Aurora package"
    );
    assert_eq!(
        std::fs::read(backup.join("userpref.blend")).expect("preferences backup"),
        preferences
    );
    assert!(run(false).enabled);
    println!(
        "Native Blender {:?}: install, persistent enable, reinstallation and backup passed",
        installed.version
    );
}
