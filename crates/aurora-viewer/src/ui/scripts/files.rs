//! LLScriptEdCore::loadScriptText / writeToFile / openInExternalEditor and
//! LLLiveLSLFile (Firestorm, LGPL 2.1). Dialogs and file IO run off the UI thread.

use aurora_net::scripts::MAX_SOURCE;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

pub(super) enum Done {
    Opened(String),
    Saved,
    External(Watch),
    Failed(String),
    Cancelled,
}
pub(super) struct Job(pub mpsc::Receiver<Done>);

fn failure(error: impl std::fmt::Display) -> Done {
    Done::Failed(aurora_i18n::tr!("script-file-error", reason = error.to_string()))
}

pub(super) fn read(path: &Path, convert_tabs: bool) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_SOURCE as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SOURCE {
        return Err(aurora_i18n::tr!("script-source-too-large").into());
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| aurora_i18n::tr!("script-file-utf8").to_owned())?;
    let source = if convert_tabs { text.replace('\t', "    ") } else { text.into() };
    if source.len() > MAX_SOURCE {
        return Err(aurora_i18n::tr!("script-source-too-large").into());
    }
    Ok(source)
}

pub(super) fn open(convert_tabs: bool) -> Job {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // LLFilePicker's script filter: LSL and SLua (*.lsl; *.lua; *.luau)
        let done = match rfd::FileDialog::new()
            .add_filter("LSL / SLua", &["lsl", "luau", "lua", "txt"])
            .pick_file()
        {
            None => Done::Cancelled,
            Some(path) => match read(&path, convert_tabs) {
                Ok(source) => Done::Opened(source),
                Err(e) => failure(e),
            },
        };
        let _ = tx.send(done);
    });
    Job(rx)
}

/// The file extension of a script: `.luau` for SLua (LLPreviewLSL /
/// LLLiveLSLEditor::onSaveToFile's script_extension).
fn extension(lua: bool) -> &'static str {
    if lua { "luau" } else { "lsl" }
}

pub(super) fn save(name: String, source: String, lua: bool) -> Job {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let done = match rfd::FileDialog::new()
            .set_file_name(format!("{}.{}", safe_name(&name), extension(lua)))
            .add_filter(if lua { "SLua" } else { "LSL" }, &[extension(lua)])
            .save_file()
        {
            None => Done::Cancelled,
            Some(path) => match fs::write(path, source) {
                Ok(()) => Done::Saved,
                Err(e) => failure(e),
            },
        };
        let _ = tx.send(done);
    });
    Job(rx)
}

fn safe_name(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"\\/|?*".contains(*c))
        .take(80)
        .collect();
    let name = name.trim_matches([' ', '.']);
    if name.is_empty() { "script".into() } else { name.into() }
}

/// Parse an executable and arguments, without invoking a command shell.
/// Quotes protect spaces; Windows backslashes are preserved except before ".
fn command(text: &str, path: &Path) -> Result<(String, Vec<String>), String> {
    let mut args = Vec::new();
    let mut arg = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        if c == '"' {
            quoted = !quoted;
            started = true;
        } else if c == '\\' {
            let mut count = 1;
            while chars.peek() == Some(&'\\') {
                chars.next();
                count += 1;
            }
            if chars.peek() == Some(&'"') {
                arg.push_str(&"\\".repeat(count / 2));
                if count % 2 == 1 {
                    chars.next();
                    arg.push('"');
                }
            } else {
                arg.push_str(&"\\".repeat(count));
            }
            started = true;
        } else if c.is_whitespace() && !quoted {
            if started {
                args.push(std::mem::take(&mut arg));
                started = false;
            }
        } else {
            arg.push(c);
            started = true;
        }
    }
    if quoted {
        return Err(aurora_i18n::tr!("script-external-command-error").into());
    }
    if started {
        args.push(arg);
    }
    if args.is_empty() || args[0].is_empty() {
        return Err(aurora_i18n::tr!("script-external-not-set").into());
    }
    let executable = args.remove(0);
    let file = path.to_string_lossy();
    let mut inserted = false;
    for arg in &mut args {
        if arg.contains("%s") {
            *arg = arg.replace("%s", &file);
            inserted = true;
        }
    }
    if !inserted {
        args.push(file.into_owned());
    }
    Ok((executable, args))
}

/// The folder of the scripts sent to the external editor: one per run of
/// the viewer, created new in the temp folder and readable by its owner
/// only (the shared temp folder of Linux and macOS is open to every
/// account, and a script can hold a key or a private URL). The scripts
/// share it, each under the name the editors of the editor sync recognise
/// (`crate::editor_sync`), and `script.list` names it as the Legacy Viewer
/// names its temp folder. The folder's 0700 is what keeps a file private
/// once an editor saves by writing a new file (with the umask's 0644) and
/// renaming it over ours. Windows: a protected DACL for the user, which the
/// files inherit, as a temp folder may be shared (a `C:\Temp` policy).
static LIVE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The folder [`LIVE`] holds, if a script was sent to the editor this run.
pub(crate) fn live_folder() -> Option<PathBuf> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A folder that did not exist, readable by its owner only: one already
/// there (planted by another account) is refused.
fn create_private_dir(directory: &Path) -> std::io::Result<()> {
    aurora_editor_sync::challenge::create_user_only_dir(directory)
}

/// Writes a script into the run's folder ([`LIVE`]), created on first use
/// and again, under a new name, if something removed it. The file is
/// claimed for the new watch before it is written: the watch it replaces
/// can no longer remove it.
fn write_live(file_name: &str, source: &str) -> std::io::Result<(PathBuf, u64)> {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    for _ in 0..2 {
        let directory = match live.as_ref() {
            Some(directory) => directory.clone(),
            None => {
                let directory = std::env::temp_dir().join(format!("AuroraScripts-{}", uuid::Uuid::new_v4()));
                create_private_dir(&directory)?;
                *live = Some(directory.clone());
                directory
            }
        };
        let path = directory.join(file_name);
        let owner = claim(&path);
        match write_owner_only(&path, source) {
            Ok(()) => return Ok((path, owner)),
            // the folder is gone (a temp cleanup): never made again under
            // the same name, which another account may have taken since
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                release(&path, owner);
                *live = None;
            }
            Err(e) => {
                release(&path, owner);
                return Err(e);
            }
        }
    }
    Err(std::io::ErrorKind::NotFound.into())
}

fn write_owner_only(path: &Path, source: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new();
    file.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        file.mode(0o600);
    }
    file.open(path)?.write_all(source.as_bytes())
}

/// Which watch a file of [`LIVE`] belongs to: a script sent to the editor
/// again takes the same file, and the watch it replaces must not remove it
/// on its way out.
static OWNERS: Mutex<Option<HashMap<PathBuf, u64>>> = Mutex::new(None);
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

fn claim(path: &Path) -> u64 {
    let owner = NEXT_OWNER.fetch_add(1, Ordering::Relaxed);
    OWNERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(path.to_owned(), owner);
    owner
}

/// The watch `owner` ends: when the file is still its own, it is removed,
/// under the lock, so that no new claim slips between the check and the
/// removal. True when it was its own.
fn release(path: &Path, owner: u64) -> bool {
    let mut owners = OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    let owners = owners.get_or_insert_with(HashMap::new);
    if owners.get(path) == Some(&owner) {
        owners.remove(path);
        let _ = fs::remove_file(path);
        return true;
    }
    false
}

pub(super) struct Watch {
    pub changes: mpsc::Receiver<Result<String, String>>,
    stop: Arc<AtomicBool>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// LLScriptEdCore::openInExternalEditor: the script goes to the run's
/// folder as `sl_script_<name>_<script id>.<lsl|luau>`
/// (LLScriptEdContainer::getTmpFileName), the name the editor sync's
/// editors read the script's id from, and the editor opens it.
pub(super) fn external(name: String, script_id: String, source: String, command_line: String, convert_tabs: bool, lua: bool) -> Job {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let file_name = aurora_editor_sync::subscriptions::file_name(&safe_name(&name), &script_id, lua);
        let command_line = if command_line.trim().is_empty() {
            std::env::var("LL_SCRIPT_EDITOR").unwrap_or_default()
        } else {
            command_line
        };
        // the command is checked before anything is written
        if let Err(e) = command(&command_line, Path::new(&file_name)) {
            let _ = tx.send(failure(e));
            return;
        }
        let (path, owner) = match write_live(&file_name, &source) {
            Ok(written) => written,
            Err(e) => {
                let _ = tx.send(failure(e));
                return;
            }
        };
        let started = (|| -> Result<Option<std::time::SystemTime>, String> {
            let (executable, args) = command(&command_line, &path)?;
            // Capture our write before launching: even an editor that saves
            // immediately must trigger the first reload.
            let stamp = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            crate::open_path::spawn(std::process::Command::new(executable).args(args)).map_err(|e| e.to_string())?;
            Ok(stamp)
        })();
        let mut stamp = match started {
            Ok(stamp) => stamp,
            Err(e) => {
                let _ = tx.send(failure(e));
                release(&path, owner);
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (changed_tx, changed_rx) = mpsc::channel();
        let _ = tx.send(Done::External(Watch {
            changes: changed_rx,
            stop: stop.clone(),
        }));
        let mut previous = source;
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            let next = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            if next.is_none() || next == stamp {
                continue;
            }
            stamp = next;
            match read(&path, convert_tabs) {
                Ok(source) if source != previous => {
                    previous.clone_from(&source);
                    if changed_tx.send(Ok(source)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    if changed_tx.send(Err(e)).is_err() {
                        break;
                    }
                }
                _ => {}
            }
        }
        release(&path, owner);
    });
    Job(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_arguments_keep_spaces_backslashes_and_do_not_use_a_shell() {
        let (exe, args) = command(
            r#""C:\Program Files\editor.exe" --wait "%s" "$literal""#,
            Path::new(r"C:\temp\a script.lsl"),
        )
        .unwrap();
        assert_eq!(exe, r"C:\Program Files\editor.exe");
        assert_eq!(args, ["--wait", r"C:\temp\a script.lsl", "$literal"]);
        assert!(command("\"unterminated", Path::new("s.lsl")).is_err());
        assert_eq!(safe_name("a: / b?."), "a  b");
    }
    #[test]
    fn file_import_is_bounded_utf8_with_optional_tab_conversion() {
        let directory = std::env::temp_dir().join(format!("aurora-script-file-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("source.lsl");
        fs::write(&path, b"\xef\xbb\xbf\tcode\r\n").unwrap();
        assert_eq!(read(&path, true).unwrap(), "    code\r\n");
        assert_eq!(read(&path, false).unwrap(), "\tcode\r\n");
        fs::write(&path, [0xff]).unwrap();
        assert!(read(&path, true).is_err());
        fs::write(&path, vec![b'x'; MAX_SOURCE + 1]).unwrap();
        assert!(read(&path, false).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn external_export_is_private_and_its_folder_never_reused() {
        let directory = std::env::temp_dir().join(format!("aurora-script-export-test-{}", uuid::Uuid::new_v4()));
        create_private_dir(&directory).unwrap();
        let path = directory.join("source.lsl");
        write_owner_only(&path, "default {}").unwrap();
        write_owner_only(&path, "x").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "x");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&directory), 0o700);
            assert_eq!(mode(&path), 0o600);
        }
        // A folder already there (planted by another account) is refused.
        assert!(create_private_dir(&directory).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn a_script_sent_again_keeps_its_file_from_the_watch_it_replaces() {
        let path = std::env::temp_dir().join(format!("aurora-owner-test-{}", uuid::Uuid::new_v4()));
        let first = claim(&path);
        fs::write(&path, "first").unwrap();
        let second = claim(&path);
        fs::write(&path, "second").unwrap();
        // the replaced watch leaves the file alone
        assert!(!release(&path, first));
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        // its own watch removes it, once
        assert!(release(&path, second));
        assert!(!path.exists());
        assert!(!release(&path, second));
    }
}
