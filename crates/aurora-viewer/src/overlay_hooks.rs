//! Overlay and capture hooks that other programs inject into the viewer.
//!
//! Unlike the Vulkan layers of `aurora_render::vulkan_layers`, which the
//! loader adds and the viewer can keep out, these are DLLs a running program
//! loads into the process at any moment (when its overlay or its capture
//! starts) and that patch the presentation functions in place. Discord's
//! `DiscordHook64.dll`, for example, is started by `DiscordHookHelper64.exe`
//! when its in-game overlay attaches or a Go Live stream captures the
//! window, and hooks `vkQueuePresentKHR`, DXGI and OpenGL; its own drawing
//! code covers DXGI and OpenGL only. The viewer has no way to refuse them
//! (Discord's exclusions are a fixed list of executable names), so it says
//! in the log when one appears: a white or frozen window reported by a
//! tester can then be matched with the hook that arrived at that time. The
//! crash report names the hooks too ("Overlay hooks"), from this fixed list
//! only.

use std::time::Duration;

/// How often the process's modules are listed: the hooks arrive late and
/// at any time. A list of a few hundred modules takes well under a
/// millisecond on a background thread.
const EVERY: Duration = Duration::from_secs(10);

/// One known hook DLL, what it belongs to and what to do if the image is
/// lost while it is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hook {
    pub file: &'static str,
    pub program: &'static str,
    pub advice: &'static str,
}

pub const KNOWN: &[Hook] = &[
    Hook {
        file: "DiscordHook64.dll",
        program: "Discord (in-game overlay or Go Live capture)",
        advice: "if the window turns white while the world's sound plays, turn off Discord's in-game overlay for Aurora (Discord › User Settings › Game Overlay, or the game's entry in Registered Games)",
    },
    Hook {
        file: "GameOverlayRenderer64.dll",
        program: "Steam overlay",
        advice: "if the image is lost, turn off the Steam overlay for Aurora",
    },
    Hook {
        file: "RTSSHooks64.dll",
        program: "RivaTuner Statistics Server (MSI Afterburner's on-screen display)",
        advice: "if the image is lost, add aurora-viewer.exe to RivaTuner with Application detection level None",
    },
    Hook {
        file: "graphics-hook64.dll",
        program: "OBS Studio game capture (or a program built on it)",
        advice: "if the image is lost, capture the window with Window Capture instead of Game Capture",
    },
];

/// The known hook of a loaded module's file name (case does not matter).
pub fn known(file: &str) -> Option<&'static Hook> {
    KNOWN.iter().find(|h| h.file.eq_ignore_ascii_case(file))
}

/// The hooks among `loaded` (module file names) that are not in `seen` yet,
/// once each; `seen` is updated.
pub fn arrived(seen: &mut Vec<&'static str>, loaded: &[String]) -> Vec<&'static Hook> {
    let mut new = Vec::new();
    for name in loaded {
        if let Some(hook) = known(name)
            && !seen.contains(&hook.file)
        {
            seen.push(hook.file);
            new.push(hook);
        }
    }
    new
}

/// `AURORA_DEMO_OVERLAY_HOOK=<file>`, with `AURORA_DEMO` only: that module
/// counts as loaded, to show what the log and the crash report say of it.
#[cfg_attr(not(windows), allow(dead_code))]
fn demo_hook(demo: bool, var: Option<&str>) -> Option<String> {
    let file = var?.trim();
    (demo && !file.is_empty()).then(|| file.to_owned())
}

/// Starts the background watch (Windows only; nothing elsewhere).
pub fn start() {
    #[cfg(windows)]
    {
        let demo = demo_hook(
            std::env::var_os("AURORA_DEMO").is_some(),
            std::env::var("AURORA_DEMO_OVERLAY_HOOK").ok().as_deref(),
        );
        let spawned = std::thread::Builder::new().name("aurora-overlay-hooks".into()).spawn(move || {
            let mut seen = Vec::new();
            loop {
                let mut loaded = modules();
                loaded.extend(demo.clone());
                for hook in arrived(&mut seen, &loaded) {
                    log::warn!(
                        "overlay hook loaded in the viewer: {} ({}); {}",
                        hook.program,
                        hook.file,
                        hook.advice
                    );
                    // the crash report names it (crash/summary.rs)
                    crate::crash::overlay_hook(hook.file);
                }
                std::thread::sleep(EVERY);
            }
        });
        if let Err(e) = spawned {
            log::warn!("overlay hook watch not started: {e}");
        }
    }
}

/// The file names of the modules loaded in this process.
#[cfg(windows)]
fn modules() -> Vec<String> {
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::ProcessStatus::{K32EnumProcessModules, K32GetModuleBaseNameW};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut handles: Vec<HMODULE> = vec![std::ptr::null_mut(); 512];
    let mut names = Vec::new();
    // SAFETY: the pseudo-handle of this process needs no close; the module
    // array and the name buffer are handed with their real sizes in bytes /
    // characters, and only the entries the calls report as written are read.
    unsafe {
        let process = GetCurrentProcess();
        let mut needed = 0u32;
        let bytes = (handles.len() * std::mem::size_of::<HMODULE>()) as u32;
        if K32EnumProcessModules(process, handles.as_mut_ptr(), bytes, &mut needed) == 0 {
            return names;
        }
        let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(handles.len());
        let mut buffer = [0u16; 260];
        for &module in &handles[..count] {
            let len = K32GetModuleBaseNameW(process, module, buffer.as_mut_ptr(), buffer.len() as u32) as usize;
            if len > 0 {
                names.push(String::from_utf16_lossy(&buffer[..len.min(buffer.len())]));
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hooks_are_found_by_file_name_whatever_the_case() {
        assert_eq!(known("discordhook64.DLL").map(|h| h.file), Some("DiscordHook64.dll"));
        assert_eq!(
            known("GameOverlayRenderer64.dll").map(|h| h.file),
            Some("GameOverlayRenderer64.dll")
        );
        for other in ["DiscordHook.dll", "kernel32.dll", "nvoglv64.dll", ""] {
            assert_eq!(known(other), None, "{other}");
        }
    }

    #[test]
    fn the_demo_hook_names_a_module_in_demo_only() {
        assert_eq!(demo_hook(true, Some(" DiscordHook64.dll ")).as_deref(), Some("DiscordHook64.dll"));
        assert_eq!(demo_hook(false, Some("DiscordHook64.dll")), None);
        assert_eq!(demo_hook(true, Some(" ")), None);
        assert_eq!(demo_hook(true, None), None);
    }

    #[test]
    fn each_hook_is_reported_once_when_it_arrives() {
        let mut seen = Vec::new();
        let before: Vec<String> = ["aurora-viewer.exe", "ntdll.dll"].map(String::from).into();
        assert!(arrived(&mut seen, &before).is_empty());
        let mut later = before.clone();
        later.push("DiscordHook64.dll".into());
        let new: Vec<_> = arrived(&mut seen, &later).iter().map(|h| h.file).collect();
        assert_eq!(new, ["DiscordHook64.dll"]);
        assert!(arrived(&mut seen, &later).is_empty());
        later.push("RTSSHooks64.dll".into());
        let new: Vec<_> = arrived(&mut seen, &later).iter().map(|h| h.file).collect();
        assert_eq!(new, ["RTSSHooks64.dll"]);
    }

    #[test]
    fn this_process_lists_its_own_modules() {
        #[cfg(windows)]
        {
            let names = modules();
            assert!(names.iter().any(|n| n.eq_ignore_ascii_case("ntdll.dll")), "{names:?}");
        }
    }
}
