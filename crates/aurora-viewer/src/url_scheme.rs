//! The system's `secondlife:` link handler: whether links of web pages open
//! in this viewer, and the choice to make them so. The Legacy Viewer's
//! installer asks and writes the class (CreateUrlRegistryEntries,
//! indra/newview/installers/windows/installer_template.nsi); Aurora has no
//! installer and writes the same class for the user alone
//! (HKCU\Software\Classes\secondlife, which wins over a machine-wide one),
//! from Preferences › Interface. The command is the installer's
//! (`"<exe>" -url "%1"`, MAINT-8305): `cli.rs` reads nothing after the link.

/// Who opens `secondlife:` links.
#[derive(Debug, Clone, PartialEq)]
pub enum Handler {
    /// This viewer's executable.
    This,
    /// Another program, by its file name.
    Other(String),
    None,
}

/// Whether this system lets Aurora register itself.
pub fn supported() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
mod win {
    use super::Handler;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW,
        RegSetKeyValueW,
    };

    const USER_CLASS: &str = r"Software\Classes\secondlife";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A string value (`None` name: the key's default value).
    fn read(root: HKEY, key: &str, name: Option<&str>) -> Option<String> {
        let key = wide(key);
        let name = name.map(wide);
        let name_ptr = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
        let mut bytes = 0u32;
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
        // SAFETY: NUL-terminated strings alive for the call; a null buffer
        // asks for the size only.
        let status = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                name_ptr,
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS || bytes == 0 {
            return None;
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: `buf` holds `bytes` bytes, as the first call asked.
        let status = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                name_ptr,
                flags,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    fn write(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        let key_w = wide(key);
        let name_w = name.map(wide);
        let value_w = wide(value);
        // SAFETY: NUL-terminated strings alive for the call; the size is
        // the value's in bytes, its NUL included.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key_w.as_ptr(),
                name_w.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                REG_SZ,
                value_w.as_ptr().cast(),
                (value_w.len() * 2) as u32,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("{key}: error {status}"))
        }
    }

    /// The program of a `"<exe>" -url "%1"` command.
    pub(super) fn command_exe(command: &str) -> Option<std::path::PathBuf> {
        let command = command.trim();
        let exe = match command.strip_prefix('"') {
            Some(rest) => rest.split('"').next()?,
            None => command.split_whitespace().next()?,
        };
        (!exe.is_empty()).then(|| std::path::PathBuf::from(exe))
    }

    fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
        }
    }

    pub fn handler() -> Handler {
        // the merged view: the user's class, else the machine's
        let Some(exe) = read(HKEY_CLASSES_ROOT, r"secondlife\shell\open\command", None).and_then(|c| command_exe(&c)) else {
            return Handler::None;
        };
        match std::env::current_exe() {
            Ok(this) if same_file(&exe, &this) => Handler::This,
            _ => Handler::Other(
                exe.file_name()
                    .map_or_else(|| exe.display().to_string(), |name| name.to_string_lossy().into_owned()),
            ),
        }
    }

    pub fn register() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.display().to_string();
        write(USER_CLASS, None, "URL:Second Life")?;
        write(USER_CLASS, Some("URL Protocol"), "")?;
        write(&format!(r"{USER_CLASS}\DefaultIcon"), None, &format!("\"{exe}\",0"))?;
        write(&format!(r"{USER_CLASS}\shell\open"), Some("FriendlyAppName"), "Aurora Viewer")?;
        write(
            &format!(r"{USER_CLASS}\shell\open\command"),
            None,
            &format!("\"{exe}\" -url \"%1\""),
        )?;
        log::info!("secondlife links: registered for this user");
        Ok(())
    }

    /// Removes the user's class when it is this viewer's: a machine-wide
    /// handler (an installed viewer) takes the links again.
    pub fn unregister() -> Result<(), String> {
        let ours = read(HKEY_CURRENT_USER, &format!(r"{USER_CLASS}\shell\open\command"), None)
            .and_then(|c| command_exe(&c))
            .zip(std::env::current_exe().ok())
            .is_some_and(|(exe, this)| same_file(&exe, &this));
        if !ours {
            return Ok(());
        }
        let key = wide(USER_CLASS);
        // SAFETY: a NUL-terminated string alive for the call.
        let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
        if status == ERROR_SUCCESS {
            log::info!("secondlife links: unregistered");
            Ok(())
        } else {
            Err(format!("{USER_CLASS}: error {status}"))
        }
    }

    /// A user class left pointing at an Aurora executable that no longer
    /// exists (the folder moved) is pointed at this one.
    pub fn repair() {
        let Some(exe) = read(HKEY_CURRENT_USER, &format!(r"{USER_CLASS}\shell\open\command"), None).and_then(|c| command_exe(&c)) else {
            return;
        };
        let aurora = exe.file_name().is_some_and(|n| n.eq_ignore_ascii_case("aurora-viewer.exe"));
        if aurora && !exe.exists() {
            log::info!("secondlife links: the registered viewer moved");
            if let Err(e) = register() {
                log::warn!("secondlife links: {e}");
            }
        }
    }
}

#[cfg(windows)]
pub use win::{register, unregister};

#[cfg(not(windows))]
pub fn register() -> Result<(), String> {
    Err("not supported on this system".into())
}

#[cfg(not(windows))]
pub fn unregister() -> Result<(), String> {
    Ok(())
}

static CACHE: std::sync::Mutex<Option<(std::time::Instant, Handler)>> = std::sync::Mutex::new(None);

/// The handler, read again at most every 2 s (the preferences draw it at
/// every frame; another program may take the links meanwhile).
pub fn handler() -> Handler {
    let Ok(mut cache) = CACHE.lock() else {
        return Handler::None;
    };
    if let Some((at, handler)) = cache.as_ref()
        && at.elapsed() < std::time::Duration::from_secs(2)
    {
        return handler.clone();
    }
    #[cfg(windows)]
    let handler = win::handler();
    #[cfg(not(windows))]
    let handler = Handler::None;
    *cache = Some((std::time::Instant::now(), handler.clone()));
    handler
}

/// Forgets the cached handler (after a change made here).
pub fn refresh() {
    if let Ok(mut cache) = CACHE.lock() {
        *cache = None;
    }
}

/// At start, published builds outside demo mode only: a registration left
/// on a moved Aurora is pointed at this one (test builds never take it).
pub fn repair_at_start() {
    #[cfg(windows)]
    if env!("AURORA_BUILD_PROFILE") == "release" && std::env::var_os("AURORA_DEMO").is_none() {
        win::repair();
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn command_program() {
        assert_eq!(
            super::win::command_exe(r#""C:\Viewers\Aurora\aurora-viewer.exe" -url "%1""#),
            Some(std::path::PathBuf::from(r"C:\Viewers\Aurora\aurora-viewer.exe"))
        );
        assert_eq!(
            super::win::command_exe(r#"C:\Viewer\viewer.exe -url "%1""#),
            Some(std::path::PathBuf::from(r"C:\Viewer\viewer.exe"))
        );
        assert_eq!(super::win::command_exe("  "), None);
    }
}
