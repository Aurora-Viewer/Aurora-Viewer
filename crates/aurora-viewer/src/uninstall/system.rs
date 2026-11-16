//! The two [`System`]s of the removal: the computer itself, and the demo's
//! dry run, which reads and removes nothing and only logs what a real run
//! would do.

use super::{AddonRemoval, Login, System};
use std::path::PathBuf;

/// The system of this run: the dry run in demo mode and in unit tests,
/// never the computer.
pub fn for_run() -> Box<dyn System> {
    if cfg!(test) || std::env::var_os("AURORA_DEMO").is_some() {
        Box::new(DryRun)
    } else {
        Box::new(Computer)
    }
}

/// The registry or desktop files, the credential store and the Blenders of
/// this computer.
struct Computer;

impl System for Computer {
    fn links(&self) -> Vec<String> {
        crate::url_scheme::registrations()
    }

    fn remove_links(&mut self) -> Result<(), String> {
        let done = crate::url_scheme::unregister();
        crate::url_scheme::refresh();
        done
    }

    fn notification_id(&self) -> Option<String> {
        crate::system_notify::app_id_registration()
    }

    fn remove_notification_id(&mut self) -> Result<(), String> {
        crate::system_notify::unregister_app_id()
    }

    fn delete_secret(&mut self, service: &str, account: &str) -> Result<bool, String> {
        // the isolated update tests never reach the user's store
        // (credentials.rs, cache_key.rs)
        if aurora_updater::test_environment().is_some() {
            return Ok(false);
        }
        let entry = keyring::Entry::new(service, account).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) => {
                log::info!("removal from this computer: credential store entry \"{service}\" deleted");
                Ok(true)
            }
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

    fn remove_blender_addon(&mut self) -> Vec<AddonRemoval> {
        crate::blender_live::remove_addon_everywhere()
    }
}

/// The demo: what this system would hold, made up, and nothing touched.
struct DryRun;

impl System for DryRun {
    fn links(&self) -> Vec<String> {
        if cfg!(windows) {
            ["secondlife", "hop", "x-grid-location-info"]
                .map(|scheme| format!(r"HKEY_CURRENT_USER\Software\Classes\{scheme}"))
                .to_vec()
        } else if cfg!(target_os = "linux") {
            vec![
                "~/.local/share/applications/aurora-viewer-slurl.desktop".to_owned(),
                "~/.config/mimeapps.list (x-scheme-handler/secondlife)".to_owned(),
            ]
        } else {
            Vec::new()
        }
    }

    fn remove_links(&mut self) -> Result<(), String> {
        log::info!("removal from this computer (dry run): the link handler would be unregistered");
        Ok(())
    }

    fn notification_id(&self) -> Option<String> {
        cfg!(windows).then(|| r"HKEY_CURRENT_USER\Software\Classes\AppUserModelId\Aurora.Viewer".to_owned())
    }

    fn remove_notification_id(&mut self) -> Result<(), String> {
        log::info!("removal from this computer (dry run): the notification identity would be removed");
        Ok(())
    }

    fn delete_secret(&mut self, service: &str, _account: &str) -> Result<bool, String> {
        log::info!("removal from this computer (dry run): credential store entry \"{service}\" would be deleted");
        Ok(true)
    }

    fn remove_blender_addon(&mut self) -> Vec<AddonRemoval> {
        log::info!("removal from this computer (dry run): no Blender started, one made up");
        let executable = if cfg!(windows) {
            r"C:\Program Files\Blender Foundation\Blender 4.5\blender.exe"
        } else if cfg!(target_os = "macos") {
            "/Applications/Blender.app/Contents/MacOS/Blender"
        } else {
            "/usr/bin/blender"
        };
        vec![AddonRemoval {
            executable: PathBuf::from(executable),
            removed: vec![PathBuf::from("Blender/4.5/extensions/user_default/aurora_live_mesh")],
            backups: None,
            error: None,
        }]
    }

    fn dry_run(&self) -> bool {
        true
    }
}

/// The demo's saved logins.
pub fn demo_logins() -> Vec<Login> {
    vec![
        Login {
            grid: aurora_net::login::SL_MAIN_GRID.to_owned(),
            user: "Demo Resident".to_owned(),
        },
        Login {
            grid: aurora_net::login::SL_BETA_GRID.to_owned(),
            user: "Tester Resident".to_owned(),
        },
    ]
}
