use super::*;

/// A computer in memory: what it holds, and what was asked of it.
#[derive(Default)]
struct Fake {
    links: Vec<String>,
    links_fail: Option<String>,
    notification: Option<String>,
    /// (service, account) entries held.
    secrets: Vec<(String, String)>,
    secret_fail: Option<String>,
    blender: Vec<AddonRemoval>,
    calls: Vec<String>,
}

impl System for Fake {
    fn links(&self) -> Vec<String> {
        self.links.clone()
    }

    fn remove_links(&mut self) -> Result<(), String> {
        self.calls.push("links".into());
        match self.links_fail.clone() {
            Some(e) => Err(e),
            None => {
                self.links.clear();
                Ok(())
            }
        }
    }

    fn notification_id(&self) -> Option<String> {
        self.notification.clone()
    }

    fn remove_notification_id(&mut self) -> Result<(), String> {
        self.calls.push("notification".into());
        self.notification = None;
        Ok(())
    }

    fn delete_secret(&mut self, service: &str, account: &str) -> Result<bool, String> {
        self.calls.push(format!("secret {service} / {account}"));
        if let Some(e) = &self.secret_fail
            && service == crate::cache_key::STORE_ENTRY.0
        {
            return Err(e.clone());
        }
        let before = self.secrets.len();
        self.secrets.retain(|(s, a)| !(s == service && a == account));
        Ok(self.secrets.len() < before)
    }

    fn remove_blender_addon(&mut self) -> Vec<AddonRemoval> {
        self.calls.push("blender".into());
        std::mem::take(&mut self.blender)
    }
}

fn login(grid: &str, user: &str) -> Login {
    Login {
        grid: grid.into(),
        user: user.into(),
    }
}

const AGNI: &str = aurora_net::login::SL_MAIN_GRID;
const ADITI: &str = aurora_net::login::SL_BETA_GRID;

#[test]
fn the_plan_lists_only_what_is_ours_and_each_login_once() {
    let fake = Fake {
        links: vec![r"HKEY_CURRENT_USER\Software\Classes\secondlife".into()],
        notification: Some("AppUserModelId".into()),
        ..Default::default()
    };
    let logins = [
        login(AGNI, "Jane Resident"),
        // the same account, as the store names it: once
        login(AGNI, " jane resident "),
        login(ADITI, "Jane Resident"),
        login(AGNI, "  "),
    ];
    let plan = plan(&fake, &logins);
    assert_eq!(
        plan.parts,
        [
            Part::Links(vec![r"HKEY_CURRENT_USER\Software\Classes\secondlife".into()]),
            Part::NotificationId("AppUserModelId".into()),
            Part::Secret(Secret::CacheKey),
            Part::Secret(Secret::Password(login(AGNI, "Jane Resident"))),
            Part::Secret(Secret::SecondFactor(login(AGNI, "Jane Resident"))),
            Part::Secret(Secret::Password(login(ADITI, "Jane Resident"))),
            Part::Secret(Secret::SecondFactor(login(ADITI, "Jane Resident"))),
            Part::Secret(Secret::CrashReports),
            Part::BlenderAddon,
        ]
    );
    assert!(fake.calls.is_empty(), "planning removes nothing");
}

#[test]
fn another_programs_link_handler_is_never_planned() {
    // `links` lists only the classes that open this executable: none here
    let plan = plan(&Fake::default(), &[]);
    assert!(!plan.parts.iter().any(|p| matches!(p, Part::Links(_) | Part::NotificationId(_))));
    let (steps, _) = run(&plan, &mut Fake::default());
    assert!(steps.iter().all(|(part, _)| !matches!(part, Part::Links(_))));
}

#[test]
fn the_store_entries_are_the_ones_the_viewer_writes() {
    let jane = login(AGNI, "Jane Resident");
    let [password, mfa] = crate::credentials::store_entries(AGNI, "Jane Resident");
    assert_eq!(Secret::Password(jane.clone()).entry(), password);
    assert_eq!(Secret::SecondFactor(jane).entry(), mfa);
    assert_eq!(password.0, "Aurora Viewer");
    assert_eq!(mfa.0, "Aurora Viewer MFA");
    assert_eq!(password.1, format!("{AGNI}|jane resident"));
    assert_eq!(Secret::CacheKey.entry(), ("Aurora Viewer cache", "disk cache key v1".to_owned()));
    assert_eq!(Secret::CrashReports.entry(), ("Aurora Viewer GitHub", "crash-reports".to_owned()));
}

#[test]
fn the_run_removes_every_part_and_reports_absent_and_failed_ones() {
    let jane = login(AGNI, "Jane Resident");
    let [password, _] = crate::credentials::store_entries(AGNI, "Jane Resident");
    let mut fake = Fake {
        links: vec!["secondlife".into()],
        links_fail: Some("access denied".into()),
        notification: Some("AppUserModelId".into()),
        secrets: vec![(password.0.to_owned(), password.1.clone())],
        secret_fail: Some("store locked".into()),
        blender: vec![AddonRemoval {
            executable: "blender".into(),
            removed: vec!["profile/aurora_live_mesh".into()],
            backups: None,
            error: None,
        }],
        ..Default::default()
    };
    let plan = plan(&fake, std::slice::from_ref(&jane));
    let (steps, blender) = run(&plan, &mut fake);
    let done: Vec<&Done> = steps.iter().map(|(_, d)| d).collect();
    assert_eq!(
        done,
        [
            // a failure does not stop the rest
            &Done::Failed("access denied".into()),
            &Done::Removed,
            &Done::Failed("store locked".into()),
            &Done::Removed,
            // never saved
            &Done::Absent,
            &Done::Absent,
            &Done::Removed,
        ]
    );
    assert_eq!(blender.len(), 1);
    assert!(fake.secrets.is_empty() && fake.notification.is_none());
    assert_eq!(fake.calls.first().map(String::as_str), Some("links"));
    assert_eq!(fake.calls.last().map(String::as_str), Some("blender"));
}

#[test]
fn the_addon_step_follows_the_blenders() {
    let removal = |removed: bool, error: Option<&str>| AddonRemoval {
        executable: "blender".into(),
        removed: if removed { vec!["a".into()] } else { Vec::new() },
        backups: None,
        error: error.map(str::to_owned),
    };
    assert_eq!(addon_done(&[]), Done::Absent);
    assert_eq!(addon_done(&[removal(true, None)]), Done::Removed);
    assert_eq!(addon_done(&[removal(false, None)]), Done::Absent);
    assert_eq!(
        addon_done(&[removal(true, None), removal(false, Some("timed out"))]),
        Done::Failed("blender: timed out".into())
    );
}

#[test]
fn the_remaining_places_exist_and_nest_once() {
    let root = PathBuf::from("profile");
    let candidates = vec![
        (Kind::Settings, root.join("support")),
        (Kind::Cache, root.join("caches")),
        // macOS: the logs and the media plugins live in the settings folder
        (Kind::Logs, root.join("support/logs")),
        (Kind::MediaPlugins, root.join("support/media-plugins")),
        (Kind::ChosenCache, PathBuf::from("D:/cache/AuroraViewerCache")),
        (Kind::Transcripts, PathBuf::from("gone")),
        (Kind::Viewer, PathBuf::new()),
    ];
    let left = remaining(candidates, |p| p != Path::new("gone"));
    assert_eq!(
        left,
        [
            Remaining {
                kinds: vec![Kind::Settings, Kind::Logs, Kind::MediaPlugins],
                path: root.join("support"),
            },
            Remaining {
                kinds: vec![Kind::Cache],
                path: root.join("caches"),
            },
            Remaining {
                kinds: vec![Kind::ChosenCache],
                path: PathBuf::from("D:/cache/AuroraViewerCache"),
            },
        ]
    );
    // an outer folder after its inner ones takes their place
    let left = remaining(
        vec![
            (Kind::Logs, root.join("data/logs")),
            (Kind::Cache, root.join("cache")),
            (Kind::MediaPlugins, root.join("data/media-plugins")),
            (Kind::Settings, root.join("data")),
        ],
        |_| true,
    );
    assert_eq!(left.len(), 2);
    assert_eq!(left[0].path, root.join("data"));
    assert_eq!(left[0].kinds, [Kind::Settings, Kind::Logs, Kind::MediaPlugins]);
    assert_eq!(left[1].kinds, [Kind::Cache]);
}

#[test]
fn a_file_opens_the_folder_that_holds_it() {
    let place = Remaining {
        kinds: vec![Kind::Viewer],
        path: PathBuf::from("viewers/aurora-viewer.exe"),
    };
    assert_eq!(place.folder(|_| false), PathBuf::from("viewers"));
    assert_eq!(place.folder(|_| true), PathBuf::from("viewers/aurora-viewer.exe"));
}

#[test]
fn the_viewer_is_its_bundle_on_macos() {
    assert_eq!(
        viewer_location(Path::new("/Applications/Aurora Viewer.app/Contents/MacOS/aurora-viewer")),
        PathBuf::from("/Applications/Aurora Viewer.app")
    );
    assert_eq!(
        viewer_location(Path::new("viewers/aurora-viewer.exe")),
        PathBuf::from("viewers/aurora-viewer.exe")
    );
}

#[test]
fn paths_are_shown_without_the_user_name() {
    let home = PathBuf::from("people").join("jane");
    let local = home.join("AppData").join("Local");
    let bases = [
        ("%LOCALAPPDATA%", local.clone()),
        ("%USERPROFILE%", home.clone()),
        ("%APPDATA%", PathBuf::new()),
    ];
    let sep = std::path::MAIN_SEPARATOR;
    // the longest base wins
    assert_eq!(
        shown(&local.join("Aurora").join("AuroraViewer"), &bases),
        format!("%LOCALAPPDATA%{sep}{}", Path::new("Aurora").join("AuroraViewer").display())
    );
    assert_eq!(shown(&home.join("Desktop"), &bases), format!("%USERPROFILE%{sep}Desktop"));
    assert_eq!(shown(&home, &bases), "%USERPROFILE%");
    // elsewhere, and an unknown (empty) base matching nothing
    assert_eq!(shown(Path::new("other"), &bases), "other");
}

#[test]
fn the_demo_and_the_tests_never_get_the_computer() {
    assert!(system::for_run().dry_run());
}

#[test]
fn the_printed_report_names_every_part_and_place() {
    let jane = login(AGNI, "Jane Resident");
    let mut fake = Fake::default();
    let plan = plan(&fake, std::slice::from_ref(&jane));
    let (steps, blender) = run(&plan, &mut fake);
    let report = Report {
        steps,
        blender,
        remaining: vec![Remaining {
            kinds: vec![Kind::Settings, Kind::Logs],
            path: PathBuf::from("support"),
        }],
        dry_run: true,
    };
    let text = printed(&report);
    assert!(text.contains(&part_label(&Part::Secret(Secret::Password(jane)))), "{text}");
    assert!(
        text.contains(&format!("{}: support", kinds_label(&[Kind::Settings, Kind::Logs]))),
        "{text}"
    );
    assert!(text.contains(aurora_i18n::tr!("uninstall-dry-run")), "{text}");
    assert!(!report.failed());
}
