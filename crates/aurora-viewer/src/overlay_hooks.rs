//! Overlay and capture hooks that other programs inject into the viewer.
//!
//! Unlike the Vulkan layers of `aurora_render::vulkan_layers`, which the
//! loader adds and the viewer can keep out, these are DLLs a running program
//! loads into the process at any moment (when its overlay or capture starts).
//! `DiscordHook64.dll` is a known legacy Discord game-hook module. Discord
//! described its redesigned overlay as no longer using the former direct game
//! hooking approach (https://discord.com/blog/player-release-q12025). A known
//! module is observational evidence, not proof that its program caused a
//! display loss. The crash report names hooks from this fixed list only.

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
        program: "Discord legacy game hook",
        advice: "if the image is lost, test with Discord's game overlay disabled for Aurora; this legacy hook does not identify Go Live capture state",
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
        // Medal documents game-process injection for its overlay/capture, but
        // does not publish this DLL name. The exact module is observed in
        // crash reports (with an OBS Graphics Hook PDB):
        // https://support.medal.tv/support/solutions/articles/48001166446-how-to-whitelist-medal-in-your-anti-virus
        // https://us.forums.blizzard.com/en/overwatch/t/dev-look-pls-constant-crashes-and-unrecoverable-errors/721037
        file: "medal-hook64.dll",
        program: "Medal game capture",
        advice: "if the image is lost, turn off Medal's in-game overlay or use window capture",
    },
    Hook {
        // NVIDIA's symbol server lists this module and a D3D12 load is logged
        // in its developer forum: https://driver-symbols.nvidia.com/nvspcap64.dll/
        // and https://forums.developer.nvidia.com/t/crash-in-d3d12createdevice-when-custom-debugger-is-attached/275346.
        file: "nvspcap64.dll",
        program: "NVIDIA Share capture overlay",
        advice: "if the image is lost, turn off the in-game overlay in NVIDIA App",
    },
    Hook {
        // FPS Monitor's developer confirms game-rendering injection and the
        // fpsmon64.dll/fps-mon64.dll names; the exact hyphenated token is also
        // present in its current package listing:
        // https://store.steampowered.com/news/posts/?enddate=1640812878&feed=steam_community_announcements
        // https://steamdb.info/depot/966611/
        file: "fps-mon64.dll",
        program: "FPS Monitor overlay",
        advice: "if the image is lost, turn off FPS Monitor's in-game overlay",
    },
    Hook {
        // OBS Studio's Windows blocklist names Bandicam's game hook and says
        // it does not unhook cleanly: https://github.com/obsproject/obs-studio/blob/master/frontend/utility/win-dll-blocklist.c.
        file: "bdcam64.dll",
        program: "Bandicam game capture",
        advice: "if the image is lost, switch Bandicam to screen recording or turn off game capture",
    },
    Hook {
        file: "we-graphics-hook64.dll",
        program: "Wallpaper Engine's capture hook (a copy of OBS Studio's game capture)",
        advice: "if the image is lost, close Wallpaper Engine or turn off its capture while Aurora runs",
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
    fn known_hooks_include_only_the_confirmed_capture_module_names() {
        for (file, program) in [
            ("medal-hook64.dll", "Medal game capture"),
            ("nvspcap64.dll", "NVIDIA Share capture overlay"),
            ("fps-mon64.dll", "FPS Monitor overlay"),
            ("bdcam64.dll", "Bandicam game capture"),
        ] {
            assert_eq!(known(file).map(|hook| hook.program), Some(program), "{file}");
        }
        for unsupported in [
            "GameBarPresenceWriter.dll",
            "GameBarOverlay.dll",
            "owclient.dll",
            "DiscordOverlay64.dll",
            "atiadlxx.dll",
            "amdlvr64.dll",
        ] {
            assert_eq!(known(unsupported), None, "{unsupported}");
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
        later.extend([
            "DiscordHook64.dll".into(),
            "medal-hook64.dll".into(),
            "nvspcap64.dll".into(),
            "fps-mon64.dll".into(),
            "bdcam64.dll".into(),
        ]);
        let new: Vec<_> = arrived(&mut seen, &later).iter().map(|h| h.file).collect();
        assert_eq!(
            new,
            [
                "DiscordHook64.dll",
                "medal-hook64.dll",
                "nvspcap64.dll",
                "fps-mon64.dll",
                "bdcam64.dll"
            ]
        );
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
