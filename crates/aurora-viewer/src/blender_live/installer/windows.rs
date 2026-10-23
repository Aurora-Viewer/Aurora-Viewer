//! Read only the OS application registry, including nonstandard install paths.
use std::{path::PathBuf, ptr};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW,
        RegGetValueW, RegOpenKeyExW,
    },
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let key = wide(key);
    let name = wide(name);
    let mut buffer = vec![0u16; 32768];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: all strings are NUL-terminated and buffer holds `bytes` writable bytes.
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn children(root: HKEY, key: &str) -> Vec<String> {
    let name = wide(key);
    let mut handle = ptr::null_mut();
    // SAFETY: the input string and output handle are valid for the call.
    if unsafe { RegOpenKeyExW(root, name.as_ptr(), 0, KEY_READ, &mut handle) } != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut keys = Vec::new();
    for index in 0..4096 {
        let mut buffer = [0u16; 256];
        let mut len = buffer.len() as u32;
        // SAFETY: handle is open, buffer/len are writable; optional outputs are null.
        let status = unsafe {
            RegEnumKeyExW(
                handle,
                index,
                buffer.as_mut_ptr(),
                &mut len,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        keys.push(String::from_utf16_lossy(&buffer[..len as usize]));
    }
    // SAFETY: close the handle returned by the successful open exactly once.
    unsafe { RegCloseKey(handle) };
    keys
}

pub(super) fn registered() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        if let Some(path) = string(root, r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\blender.exe", "") {
            paths.push(PathBuf::from(path.trim_matches('"')));
        }
        for key in [
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ] {
            for child in children(root, key) {
                let key = format!("{key}\\{child}");
                if !string(root, &key, "DisplayName").is_some_and(|n| n.to_ascii_lowercase().contains("blender")) {
                    continue;
                }
                if let Some(path) = string(root, &key, "InstallLocation") {
                    paths.push(PathBuf::from(path.trim_matches('"')).join("blender.exe"));
                }
                if let Some(icon) = string(root, &key, "DisplayIcon") {
                    let executable = icon
                        .rsplit_once(',')
                        .map_or(icon.as_str(), |(path, _)| path)
                        .trim()
                        .trim_matches('"');
                    paths.push(PathBuf::from(executable));
                }
            }
        }
    }
    paths
}
