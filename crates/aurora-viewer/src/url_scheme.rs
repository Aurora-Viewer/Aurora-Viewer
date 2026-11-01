//! The system's `secondlife:` link handler: whether links of web pages open
//! in this viewer, and the choice to make them so. The Legacy Viewer's
//! installer asks and writes the class (CreateUrlRegistryEntries,
//! indra/newview/installers/windows/installer_template.nsi); Aurora has no
//! installer and writes the same class for the user alone
//! (HKCU\Software\Classes\secondlife, which wins over a machine-wide one),
//! from Preferences › Interface. The command is the installer's
//! (`"<exe>" -url "%1"`, MAINT-8305): `cli.rs` reads nothing after the link.
//!
//! On Linux the Legacy Viewer's register_secondlifeprotocol.sh
//! (indra/newview/linux_tools/) writes a hidden desktop entry for the scheme
//! into the user's applications folder and makes it the default with
//! `xdg-mime default`. Aurora writes the same two files itself, per the XDG
//! specifications: `$XDG_DATA_HOME/applications/aurora-viewer-slurl.desktop`
//! and its `x-scheme-handler/<scheme>` defaults in
//! `$XDG_CONFIG_HOME/mimeapps.list`, the rest of which it leaves untouched.

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
    cfg!(any(windows, target_os = "linux"))
}

#[cfg(any(windows, target_os = "linux"))]
const SCHEMES: [&str; 3] = ["secondlife", "hop", "x-grid-location-info"];

#[cfg(windows)]
mod win {
    use super::{Handler, SCHEMES};
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW,
        RegSetKeyValueW,
    };

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

    fn handler_for(scheme: &str) -> Handler {
        // the merged view: the user's class, else the machine's
        let Some(exe) = read(HKEY_CLASSES_ROOT, &format!(r"{scheme}\shell\open\command"), None).and_then(|c| command_exe(&c)) else {
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

    pub fn handler() -> Handler {
        SCHEMES
            .iter()
            .map(|scheme| handler_for(scheme))
            .find(|handler| *handler != Handler::This)
            .unwrap_or(Handler::This)
    }

    pub fn register() -> Result<(), String> {
        for scheme in SCHEMES {
            register_scheme(scheme)?;
        }
        Ok(())
    }

    fn register_scheme(scheme: &str) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.display().to_string();
        let class = format!(r"Software\Classes\{scheme}");
        write(&class, None, "URL:Virtual World")?;
        write(&class, Some("URL Protocol"), "")?;
        write(&format!(r"{class}\DefaultIcon"), None, &format!("\"{exe}\",0"))?;
        write(&format!(r"{class}\shell\open"), Some("FriendlyAppName"), "Aurora Viewer")?;
        write(&format!(r"{class}\shell\open\command"), None, &format!("\"{exe}\" -url \"%1\""))?;
        log::info!("{scheme} links: registered for this user");
        Ok(())
    }

    /// Removes the user's class when it is this viewer's: a machine-wide
    /// handler (an installed viewer) takes the links again.
    pub fn unregister() -> Result<(), String> {
        for scheme in SCHEMES {
            unregister_scheme(scheme)?;
        }
        Ok(())
    }

    fn unregister_scheme(scheme: &str) -> Result<(), String> {
        let class = format!(r"Software\Classes\{scheme}");
        let ours = read(HKEY_CURRENT_USER, &format!(r"{class}\shell\open\command"), None)
            .and_then(|c| command_exe(&c))
            .zip(std::env::current_exe().ok())
            .is_some_and(|(exe, this)| same_file(&exe, &this));
        if !ours {
            return Ok(());
        }
        let key = wide(&class);
        // SAFETY: a NUL-terminated string alive for the call.
        let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
        if status == ERROR_SUCCESS {
            log::info!("{scheme} links: unregistered");
            Ok(())
        } else {
            Err(format!("{class}: error {status}"))
        }
    }

    /// A user class left pointing at an Aurora executable that no longer
    /// exists (the folder moved) is pointed at this one.
    pub fn repair() {
        for scheme in SCHEMES {
            let class = format!(r"Software\Classes\{scheme}");
            let Some(exe) = read(HKEY_CURRENT_USER, &format!(r"{class}\shell\open\command"), None).and_then(|c| command_exe(&c)) else {
                continue;
            };
            let aurora = exe.file_name().is_some_and(|n| n.eq_ignore_ascii_case("aurora-viewer.exe"));
            if aurora && !exe.exists() {
                log::info!("{scheme} links: the registered viewer moved");
                if let Err(e) = register_scheme(scheme) {
                    log::warn!("{scheme} links: {e}");
                }
            }
        }
    }
}

#[cfg(windows)]
pub use win::{register, unregister};

#[cfg(target_os = "linux")]
mod xdg {
    use super::{Handler, SCHEMES};
    use std::path::{Path, PathBuf};

    /// Aurora's own entry, hidden from the menus (the shipped
    /// `aurora-viewer.desktop` claims no scheme: registering stays a choice).
    const DESKTOP_ID: &str = "aurora-viewer-slurl.desktop";
    const DEFAULTS: &str = "Default Applications";

    fn home_dir(var: &str, fallback: &str) -> Option<PathBuf> {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(fallback)))
    }

    fn dirs(var: &str, fallback: &str) -> Vec<PathBuf> {
        let list = std::env::var(var).ok().filter(|v| !v.is_empty());
        list.as_deref()
            .unwrap_or(fallback)
            .split(':')
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect()
    }

    fn config_home() -> Option<PathBuf> {
        home_dir("XDG_CONFIG_HOME", ".config")
    }

    fn data_home() -> Option<PathBuf> {
        home_dir("XDG_DATA_HOME", ".local/share")
    }

    /// The `applications` folders, the user's first.
    fn application_dirs() -> Vec<PathBuf> {
        data_home()
            .into_iter()
            .chain(dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share"))
            .map(|d| d.join("applications"))
            .collect()
    }

    /// The `mimeapps.list` files in the order the specification reads them
    /// (Association between MIME types and applications 1.0.1): per folder,
    /// the current desktops' own lists first.
    fn mimeapps_lists() -> Vec<PathBuf> {
        let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|d| !d.is_empty())
            .map(str::to_lowercase)
            .collect();
        let folders = config_home()
            .into_iter()
            .chain(dirs("XDG_CONFIG_DIRS", "/etc/xdg"))
            .chain(application_dirs());
        folders
            .flat_map(|folder| {
                desktops
                    .iter()
                    .map(|d| format!("{d}-mimeapps.list"))
                    .chain(std::iter::once("mimeapps.list".to_owned()))
                    .map(move |name| folder.join(name))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The values of `key` in `[section]` of a desktop-entry style file.
    pub(super) fn value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
        let mut inside = false;
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                inside = name == section;
            } else if inside
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return Some(v.trim());
            }
        }
        None
    }

    /// `text` with `key` of `[section]` set to `value`, or removed (`None`);
    /// every other line is kept as it was.
    pub(super) fn with_value(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut inside = false;
        let mut found = false;
        // where a new key goes: after the section's last non-blank line
        let mut insert_at = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(name) = trimmed.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                inside = name == section;
                found |= inside;
            } else if inside && trimmed.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
                continue;
            }
            out.push(line.to_owned());
            if inside && !trimmed.is_empty() {
                insert_at = Some(out.len());
            }
        }
        if let Some(value) = value {
            let entry = format!("{key}={value}");
            match insert_at {
                Some(at) => out.insert(at, entry),
                None => {
                    if !found {
                        if out.last().is_some_and(|l| !l.trim().is_empty()) {
                            out.push(String::new());
                        }
                        out.push(format!("[{section}]"));
                    }
                    out.push(entry);
                }
            }
        }
        let mut text = out.join("\n");
        text.push('\n');
        text
    }

    /// The program of an `Exec` value as the file holds it, its escapes and
    /// quoting undone (Desktop Entry Specification, "The Exec key").
    pub(super) fn exec_program(exec: &str) -> Option<String> {
        let exec = exec.replace("\\\\", "\\").replace("%%", "%");
        let exec = exec.trim();
        let program = match exec.strip_prefix('"') {
            Some(rest) => {
                let mut out = String::new();
                let mut chars = rest.chars();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => out.push(chars.next()?),
                        c => out.push(c),
                    }
                }
                out
            }
            None => exec.split_whitespace().next()?.to_owned(),
        };
        (!program.is_empty()).then_some(program)
    }

    /// The `Exec` line that opens a link with `exe`. A plain path stays bare:
    /// xdg-utils' xdg-open and xdg-mime take the first word of `Exec` as the
    /// program, quotes included, so they ignore a quoted entry and fall back
    /// to another handler. A path with a reserved character must be quoted
    /// (escaped once for the quoting and once more for the file's string
    /// values); only the desktops' own openers (GLib, KDE) read it then.
    pub(super) fn exec_line(exe: &str) -> String {
        const RESERVED: &str = " \t\n\"'\\><~|&;$*?#()`";
        if !exe.is_empty() && !exe.contains(|c| RESERVED.contains(c)) {
            return format!("{} -url %u", exe.replace('%', "%%"));
        }
        let mut quoted = String::from("\"");
        for c in exe.chars() {
            match c {
                '"' | '`' | '$' => {
                    quoted.push_str("\\\\");
                    quoted.push(c);
                }
                '\\' => quoted.push_str("\\\\\\\\"),
                '%' => quoted.push_str("%%"),
                c => quoted.push(c),
            }
        }
        quoted.push('"');
        format!("{quoted} -url %u")
    }

    pub(super) fn desktop_entry(exe: &str) -> String {
        let mimes: String = SCHEMES.iter().map(|s| format!("x-scheme-handler/{s};")).collect();
        format!(
            "[Desktop Entry]\nType=Application\nName=Aurora Viewer\nComment=Opens Second Life and Hypergrid links\n\
             Exec={}\nIcon=aurora-viewer\nTerminal=false\nNoDisplay=true\nMimeType={mimes}\n",
            exec_line(exe)
        )
    }

    /// The program found on the PATH, as a launcher would run it.
    fn resolve(program: &str) -> Option<PathBuf> {
        if program.contains('/') {
            return Some(PathBuf::from(program));
        }
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(program))
            .find(|p| p.is_file())
    }

    fn same_file(a: &Path, b: &Path) -> bool {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == b,
        }
    }

    fn desktop_file(id: &str) -> Option<PathBuf> {
        application_dirs().into_iter().map(|d| d.join(id)).find(|p| p.is_file())
    }

    /// The executable's path. Once the file is replaced under a running
    /// viewer (a rebuild, a package upgrade), Linux reports it with
    /// " (deleted)" appended: the new file at the same path is the one a
    /// link must start.
    pub(super) fn live_exe(exe: PathBuf) -> PathBuf {
        if exe.exists() {
            return exe;
        }
        match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
            Some(path) => PathBuf::from(path),
            None => exe,
        }
    }

    fn this_exe() -> Option<PathBuf> {
        std::env::current_exe().ok().map(live_exe)
    }

    fn is_this(program: &str) -> bool {
        resolve(program).zip(this_exe()).is_some_and(|(exe, this)| same_file(&exe, &this))
    }

    /// The desktop entry that opens `scheme`: the first default of the
    /// lists that is installed, else the first entry claiming it in a
    /// `mimeinfo.cache` (what `xdg-open` and GLib fall back on).
    fn handler_id(scheme: &str) -> Option<String> {
        let key = format!("x-scheme-handler/{scheme}");
        let installed = |ids: &str| {
            ids.split(';')
                .map(str::trim)
                .find(|id| !id.is_empty() && desktop_file(id).is_some())
                .map(str::to_owned)
        };
        let from_lists = mimeapps_lists()
            .iter()
            .filter_map(|list| std::fs::read_to_string(list).ok())
            .find_map(|text| value(&text, DEFAULTS, &key).and_then(installed));
        from_lists.or_else(|| {
            application_dirs()
                .iter()
                .filter_map(|d| std::fs::read_to_string(d.join("mimeinfo.cache")).ok())
                .find_map(|text| value(&text, "MIME Cache", &key).and_then(installed))
        })
    }

    fn handler_for(scheme: &str) -> Handler {
        let Some(id) = handler_id(scheme) else {
            return Handler::None;
        };
        let text = desktop_file(&id).and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let entry = "Desktop Entry";
        if value(&text, entry, "Exec").and_then(exec_program).is_some_and(|p| is_this(&p)) {
            return Handler::This;
        }
        let name = value(&text, entry, "Name").filter(|n| !n.is_empty());
        Handler::Other(name.map_or(id.trim_end_matches(".desktop").to_owned(), str::to_owned))
    }

    pub fn handler() -> Handler {
        SCHEMES
            .iter()
            .map(|scheme| handler_for(scheme))
            .find(|handler| *handler != Handler::This)
            .unwrap_or(Handler::This)
    }

    /// Writes beside the target, then renames: a reader never sees half a
    /// file. A symbolic link (a dotfiles folder) keeps pointing at it, and
    /// the file keeps its permissions (a private `mimeapps.list` stays so).
    fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("aurora-tmp");
        let kept = std::fs::metadata(&path).ok().map(|m| m.permissions());
        std::fs::write(&tmp, text)
            .and_then(|()| kept.map_or(Ok(()), |mode| std::fs::set_permissions(&tmp, mode)))
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("{}: {e}", path.display())
            })
    }

    /// The ids of a default list (`a.desktop;b.desktop;`), in order.
    fn ids(list: Option<&str>) -> impl Iterator<Item = &str> {
        list.unwrap_or_default().split(';').map(str::trim).filter(|id| !id.is_empty())
    }

    /// Aurora's entry first, then the handlers that were there before it
    /// (a Legacy Viewer's, set by `xdg-mime default`): turning the links
    /// off gives them back.
    pub(super) fn ours_first(list: Option<&str>) -> String {
        std::iter::once(DESKTOP_ID)
            .chain(ids(list).filter(|id| *id != DESKTOP_ID))
            .map(|id| format!("{id};"))
            .collect()
    }

    /// The list without Aurora's entry; None when nothing is left.
    pub(super) fn without_ours(list: Option<&str>) -> Option<String> {
        let rest: String = ids(list).filter(|id| *id != DESKTOP_ID).map(|id| format!("{id};")).collect();
        (!rest.is_empty()).then_some(rest)
    }

    fn own_entry() -> Result<PathBuf, String> {
        data_home()
            .map(|d| d.join("applications").join(DESKTOP_ID))
            .ok_or_else(|| "no home folder".into())
    }

    fn mimeapps() -> Result<PathBuf, String> {
        config_home()
            .map(|d| d.join("mimeapps.list"))
            .ok_or_else(|| "no home folder".into())
    }

    fn write_entry() -> Result<(), String> {
        let exe = live_exe(std::env::current_exe().map_err(|e| e.to_string())?);
        write_atomic(&own_entry()?, &desktop_entry(&exe.to_string_lossy()))
    }

    pub fn register() -> Result<(), String> {
        write_entry()?;
        let list = mimeapps()?;
        // a list that exists but cannot be read is left alone: rewriting it
        // from nothing would drop every other default of the user
        let mut text = match std::fs::read_to_string(&list) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", list.display())),
        };
        for scheme in SCHEMES {
            let key = format!("x-scheme-handler/{scheme}");
            let list = ours_first(value(&text, DEFAULTS, &key));
            text = with_value(&text, DEFAULTS, &key, Some(&list));
        }
        write_atomic(&list, &text)?;
        log::info!("secondlife links: registered for this user");
        Ok(())
    }

    /// Removes this viewer's defaults and entry; another handler of the
    /// lists or of the system takes the links again. An entry left by
    /// another Aurora executable stays.
    pub fn unregister() -> Result<(), String> {
        let entry = own_entry()?;
        let text = std::fs::read_to_string(&entry).unwrap_or_default();
        let ours = value(&text, "Desktop Entry", "Exec")
            .and_then(exec_program)
            .is_some_and(|p| is_this(&p));
        if !ours {
            return Ok(());
        }
        let list = mimeapps()?;
        if let Ok(old) = std::fs::read_to_string(&list) {
            let mut text = old.clone();
            for scheme in SCHEMES {
                let key = format!("x-scheme-handler/{scheme}");
                let kept = without_ours(value(&text, DEFAULTS, &key));
                if kept.as_deref() != value(&text, DEFAULTS, &key) {
                    text = with_value(&text, DEFAULTS, &key, kept.as_deref());
                }
            }
            if text != old {
                write_atomic(&list, &text)?;
            }
        }
        std::fs::remove_file(&entry).map_err(|e| format!("{}: {e}", entry.display()))?;
        log::info!("secondlife links: unregistered");
        Ok(())
    }

    /// Aurora's entry left pointing at an executable that no longer exists
    /// (moved or rebuilt elsewhere) is pointed at this one.
    pub fn repair() {
        let Ok(entry) = own_entry() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&entry) else {
            return;
        };
        let gone = value(&text, "Desktop Entry", "Exec")
            .and_then(exec_program)
            .is_some_and(|p| Path::new(&p).is_absolute() && !Path::new(&p).exists());
        if gone {
            log::info!("secondlife links: the registered viewer moved");
            if let Err(e) = write_entry() {
                log::warn!("secondlife links: {e}");
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use xdg::{register, unregister};

#[cfg(not(any(windows, target_os = "linux")))]
pub fn register() -> Result<(), String> {
    Err("not supported on this system".into())
}

#[cfg(not(any(windows, target_os = "linux")))]
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
    #[cfg(target_os = "linux")]
    let handler = xdg::handler();
    #[cfg(not(any(windows, target_os = "linux")))]
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
    if env!("AURORA_BUILD_PROFILE") == "release" && std::env::var_os("AURORA_DEMO").is_none() {
        #[cfg(windows)]
        win::repair();
        #[cfg(target_os = "linux")]
        xdg::repair();
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

#[cfg(all(test, target_os = "linux"))]
mod xdg_tests {
    use super::xdg::{desktop_entry, exec_line, exec_program, live_exe, ours_first, value, with_value, without_ours};
    use std::path::PathBuf;

    #[test]
    fn a_replaced_executable_is_registered_at_its_path() {
        let dir = std::env::temp_dir().join(format!("aurora-live-exe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let exe = dir.join("aurora-viewer");
        std::fs::write(&exe, "").expect("temp file");
        let deleted = PathBuf::from(format!("{} (deleted)", exe.display()));
        assert_eq!(live_exe(deleted), exe);
        assert_eq!(live_exe(exe.clone()), exe);
        // removed and not replaced: still its path, never the suffix
        assert_eq!(live_exe(dir.join("gone (deleted)")), dir.join("gone"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    const LIST: &str = "[Added Associations]\ntext/html=firefox.desktop;\n\n[Default Applications]\n\
        x-scheme-handler/secondlife=firestorm.desktop;\ntext/plain=kate.desktop\n\n# kept\n";

    #[test]
    fn mimeapps_defaults_are_edited_in_place() {
        let key = "x-scheme-handler/secondlife";
        let set = with_value(LIST, "Default Applications", key, Some("aurora-viewer-slurl.desktop"));
        assert_eq!(value(&set, "Default Applications", key), Some("aurora-viewer-slurl.desktop"));
        // the other sections, keys and the comment stay
        assert_eq!(value(&set, "Added Associations", "text/html"), Some("firefox.desktop;"));
        assert_eq!(value(&set, "Default Applications", "text/plain"), Some("kate.desktop"));
        assert!(set.contains("# kept"));
        assert_eq!(set.matches(key).count(), 1);
        let removed = with_value(&set, "Default Applications", key, None);
        assert_eq!(value(&removed, "Default Applications", key), None);
        assert_eq!(value(&removed, "Default Applications", "text/plain"), Some("kate.desktop"));
        // no list yet, or no defaults section
        let fresh = with_value("", "Default Applications", "x-scheme-handler/hop", Some("a.desktop"));
        assert_eq!(fresh, "[Default Applications]\nx-scheme-handler/hop=a.desktop\n");
        let added = with_value("[Added Associations]\nx=y\n", "Default Applications", "k", Some("v"));
        assert_eq!(added, "[Added Associations]\nx=y\n\n[Default Applications]\nk=v\n");
    }

    #[test]
    fn turning_links_on_then_off_gives_the_earlier_handler_back() {
        let key = "x-scheme-handler/secondlife";
        let earlier = value(LIST, "Default Applications", key);
        let on = ours_first(earlier);
        assert_eq!(
            on,
            format!("aurora-viewer-slurl.desktop;{}", earlier.expect("the fixture's default"))
        );
        assert_eq!(without_ours(Some(&on)).as_deref(), earlier);
        // registered twice, or already in the list: listed once, first
        assert_eq!(ours_first(Some(&on)), on);
        assert_eq!(
            ours_first(Some("a.desktop; aurora-viewer-slurl.desktop;b.desktop")),
            "aurora-viewer-slurl.desktop;a.desktop;b.desktop;"
        );
        // no earlier handler: Aurora alone, and nothing left once off
        assert_eq!(ours_first(None), "aurora-viewer-slurl.desktop;");
        assert_eq!(without_ours(Some("aurora-viewer-slurl.desktop;")), None);
        assert_eq!(without_ours(None), None);
    }

    #[test]
    fn exec_lines_round_trip() {
        for exe in [
            "/usr/bin/aurora-viewer",
            "/home/me/My Viewers/aurora-viewer",
            "/opt/100%/aurora-viewer",
            r#"/odd/"q"$x`b\\%/aurora"#,
        ] {
            let line = exec_line(exe);
            assert!(line.ends_with(" -url %u"), "{line}");
            assert_eq!(exec_program(&line).as_deref(), Some(exe), "{line}");
        }
        assert_eq!(exec_program("firestorm %u").as_deref(), Some("firestorm"));
        assert_eq!(exec_program("  "), None);
    }

    #[test]
    fn desktop_entry_claims_the_three_schemes() {
        let entry = desktop_entry("/usr/bin/aurora-viewer");
        assert_eq!(value(&entry, "Desktop Entry", "NoDisplay"), Some("true"));
        assert_eq!(
            value(&entry, "Desktop Entry", "MimeType"),
            Some("x-scheme-handler/secondlife;x-scheme-handler/hop;x-scheme-handler/x-grid-location-info;")
        );
        // bare, or xdg-utils skips the entry
        assert_eq!(value(&entry, "Desktop Entry", "Exec"), Some("/usr/bin/aurora-viewer -url %u"));
        assert_eq!(
            value(&desktop_entry("/opt/My Viewers/aurora-viewer"), "Desktop Entry", "Exec"),
            Some(r#""/opt/My Viewers/aurora-viewer" -url %u"#)
        );
    }
}
