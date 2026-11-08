use super::*;

#[test]
fn blender_live_installer_selected_executable_checks_installs_and_reports_its_steps() {
    let executable = Path::new("chosen/blender.exe");
    let mut calls = Vec::new();
    let mut phases = Vec::new();
    let report = install_selected(
        executable,
        |path, install| {
            calls.push((path.to_owned(), install));
            Ok(Report {
                version: [5, 2, 1],
                plugin_version: install.then(|| addon::version().into()),
                enabled: install,
                ..Default::default()
            })
        },
        |phase| phases.push(phase),
    );
    assert!(report.error.is_none() && report.enabled);
    assert_eq!(report.plugin_version.as_deref(), Some(addon::version()));
    assert_eq!(calls, [(executable.to_owned(), false), (executable.to_owned(), true)]);
    assert_eq!(phases, [Phase::Checking, Phase::Installing]);
}

#[test]
fn blender_live_installer_selected_executable_rejects_unsupported_newer_and_failed_probes() {
    let [major, minor, patch] = semver(addon::version()).expect("bundled version");
    for initial in [
        Report {
            version: [2, 92, 0],
            ..Default::default()
        },
        Report {
            version: [5, 2, 1],
            plugin_version: Some(format!("{major}.{}.{patch}", minor + 1)),
            ..Default::default()
        },
        Report {
            error: Some("Probe failed".into()),
            ..Default::default()
        },
    ] {
        let mut calls = 0;
        let report = install_selected(
            Path::new("chosen/blender.exe"),
            |_, install| {
                assert!(!install, "a rejected probe must never install");
                calls += 1;
                Ok(initial.clone())
            },
            |_| {},
        );
        assert!(report.error.is_some());
        assert_eq!(calls, 1);
    }
}

#[test]
fn blender_live_installer_selected_executable_requires_verified_activation() {
    for installed in [
        Ok(Report {
            version: [5, 2, 1],
            ..Default::default()
        }),
        Ok(Report {
            version: [5, 2, 1],
            enabled: true,
            plugin_version: Some("0.1.0".into()),
            ..Default::default()
        }),
        Err("Installation failed".to_owned()),
    ] {
        let report = install_selected(
            Path::new("chosen/blender.exe"),
            |_, install| {
                if install {
                    installed.clone()
                } else {
                    Ok(Report {
                        version: [5, 2, 1],
                        ..Default::default()
                    })
                }
            },
            |_| {},
        );
        assert!(report.error.is_some(), "incomplete installation must never report success");
    }
}

#[test]
fn blender_live_installer_chosen_result_preserves_other_rows_and_cancellation_is_idle() {
    let mut installer = Installer::default();
    installer.installations.push(Installation {
        executable: "other/blender.exe".into(),
        report: Report {
            version: [4, 3, 0],
            ..Default::default()
        },
    });
    installer.success = Some(Report::default());
    assert!(installer.begin(Phase::Choosing));
    assert!(installer.success.is_none());
    assert!(!installer.scanned);
    assert!(
        !installer.begin(Phase::Scanning),
        "a second operation must not replace the active installation"
    );
    assert_eq!(installer.phase(), Phase::Choosing);
    *installer.result.lock() = Some(Completed::Install {
        executable: "chosen/blender.exe".into(),
        report: Report {
            version: [5, 2, 1],
            enabled: true,
            plugin_version: Some(addon::version().into()),
            ..Default::default()
        },
    });
    installer.poll();
    assert!(!installer.busy && !installer.scanned);
    assert_eq!(installer.installations.len(), 2);
    assert_eq!(installer.installations[0].report.version, [4, 3, 0]);
    assert_eq!(installer.success.as_ref().expect("completed installation").version, [5, 2, 1]);
    assert!(installer.begin(Phase::Choosing));
    *installer.result.lock() = Some(Completed::Cancelled);
    installer.poll();
    assert!(!installer.busy && !installer.scanned);
    assert_eq!(installer.installations.len(), 2);
    assert!(installer.error.is_none() && installer.success.is_none());
}

#[test]
fn blender_live_installer_remembers_only_the_selection_without_inspecting_other_paths() {
    let temporary = Temporary::new().expect("directory");
    let file = temporary.0.join("remembered.json");
    let unavailable = PathBuf::from("unavailable-drive/Blender/blender.exe");
    std::fs::write(&file, serde_json::to_vec(&vec![unavailable.clone()]).expect("paths")).expect("fixture");
    let chosen = Path::new("selected/Blender/blender.exe");
    remember_at(&file, chosen).expect("remember");
    remember_at(&file, chosen).expect("remember again");
    let paths: Vec<PathBuf> = serde_json::from_slice(&std::fs::read(file).expect("saved paths")).expect("paths");
    assert_eq!(
        paths,
        [chosen.to_owned(), unavailable],
        "other paths are retained without validation or discovery"
    );
}

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

#[test]
fn blender_live_removal_runs_only_where_aurora_is_installed_and_keeps_backups() {
    let temporary = Temporary::new().expect("directory");
    let with_backups = temporary.0.join("profile-a");
    std::fs::create_dir_all(with_backups.join("aurora-backups")).expect("backups");
    let entry = |name: &str, plugin: Option<&str>, enabled: bool, error: Option<&str>, profile: PathBuf| Installation {
        executable: PathBuf::from(name),
        report: Report {
            version: [5, 2, 1],
            plugin_version: plugin.map(str::to_owned),
            enabled,
            profile,
            error: error.map(str::to_owned),
            ..Default::default()
        },
    };
    let installations = vec![
        entry("installed", Some(addon::version()), true, None, with_backups.clone()),
        entry("disabled", Some("0.8.6"), false, None, PathBuf::new()),
        entry("enabled-only", None, true, None, temporary.0.join("profile-c")),
        entry("absent", None, false, None, temporary.0.join("profile-d")),
        entry("unreadable", Some(addon::version()), true, Some("probe failed"), PathBuf::new()),
        entry("failing", Some(addon::version()), true, None, PathBuf::new()),
    ];
    let mut ran = Vec::new();
    let removals = remove_from(installations, |path| {
        ran.push(path.to_owned());
        if path == Path::new("failing") {
            return Err("timed out".into());
        }
        Ok(Report {
            removed: vec![PathBuf::from(path).join("aurora_live_mesh")],
            ..Default::default()
        })
    });
    assert_eq!(
        ran,
        ["installed", "disabled", "enabled-only", "failing"].map(PathBuf::from),
        "never a Blender without Aurora's add-on, nor one whose profile could not be read"
    );
    assert_eq!(removals[0].removed, [PathBuf::from("installed/aurora_live_mesh")]);
    assert_eq!(removals[0].backups.as_deref(), Some(with_backups.join("aurora-backups").as_path()));
    assert!(removals[1..].iter().all(|r| r.backups.is_none()));
    assert_eq!(removals[3].error.as_deref(), Some("timed out"));
    assert!(removals[3].removed.is_empty());
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
        bridge(command, Operation::install(install)).expect("bridge report")
    };
    let initial = run(false);
    assert!(initial.error.is_none(), "{:?}", initial.error);
    assert!(initial.plugin_version.is_none());
    assert!(!config.join("userpref.blend").exists(), "probe never saves preferences");
    let mut phases = Vec::new();
    let installed = install_selected(
        &executable,
        |path, install| {
            assert_eq!(path, executable, "only the selected executable runs");
            Ok(run(install))
        },
        |phase| phases.push(phase),
    );
    assert_eq!(phases, [Phase::Checking, Phase::Installing]);
    assert!(installed.error.is_none(), "{:?}", installed.error);
    assert_eq!(installed.plugin_version.as_deref(), Some(addon::version()));
    assert!(installed.enabled);
    assert!(installed.target.join("__init__.py").is_file());
    // the removal below deletes this folder: never one of the user's own profile
    let inside = |path: &Path| {
        path.canonicalize()
            .ok()
            .zip(profile.0.canonicalize().ok())
            .is_some_and(|(path, root)| path.starts_with(root))
    };
    assert!(inside(&installed.target), "{:?} is outside the isolated profile", installed.target);
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
    let mut command = command(&executable);
    command
        .env("BLENDER_USER_CONFIG", &config)
        .env("BLENDER_USER_SCRIPTS", profile.0.join("profile/scripts"))
        .env("BLENDER_USER_EXTENSIONS", profile.0.join("profile/extensions"))
        .env("BLENDER_USER_RESOURCES", profile.0.join("profile"));
    let removed = bridge(command, Operation::Uninstall).expect("uninstall report");
    assert!(removed.error.is_none(), "{:?}", removed.error);
    assert_eq!(removed.removed, std::slice::from_ref(&installed.target));
    assert!(!installed.target.exists(), "the package folder is deleted");
    assert!(backup.join("userpref.blend").is_file(), "the install's backups stay");
    let after = run(false);
    assert!(after.plugin_version.is_none() && !after.enabled, "gone after restarting Blender");
    println!(
        "Native Blender {:?}: install, persistent enable, reinstallation, backup and removal passed",
        installed.version
    );
}
