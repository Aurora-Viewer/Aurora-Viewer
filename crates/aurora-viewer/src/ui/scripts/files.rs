//! LLScriptEdCore::loadScriptText / writeToFile / openInExternalEditor and
//! LLLiveLSLFile (Firestorm, LGPL 2.1). Dialogs and file IO run off the UI thread.

use aurora_net::scripts::MAX_SOURCE;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
        let done = match rfd::FileDialog::new().add_filter("LSL", &["lsl", "txt"]).pick_file() {
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

pub(super) fn save(name: String, source: String) -> Job {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let done = match rfd::FileDialog::new()
            .set_file_name(format!("{}.lsl", safe_name(&name)))
            .add_filter("LSL", &["lsl"])
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

/// Creates the export folder and writes the script into it, readable by its
/// owner only: the shared temp folder of Linux and macOS is open to every
/// account, and a script can hold a key or a private URL. The folder's 0700
/// is what keeps it private once an editor saves by writing a new file (with
/// the umask's 0644) and renaming it over ours. Windows' temp folder is
/// already per user.
fn write_private(directory: &Path, path: &Path, source: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new();
    file.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        fs::DirBuilder::new().mode(0o700).create(directory)?;
        file.mode(0o600);
    }
    #[cfg(not(unix))]
    fs::create_dir(directory)?;
    file.open(path)?.write_all(source.as_bytes())
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

pub(super) fn external(name: String, source: String, command_line: String, convert_tabs: bool) -> Job {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let directory = std::env::temp_dir().join(format!("AuroraScript-{}", uuid::Uuid::new_v4()));
        let path = directory.join(format!("{}.lsl", safe_name(&name)));
        let command_line = if command_line.trim().is_empty() {
            std::env::var("LL_SCRIPT_EDITOR").unwrap_or_default()
        } else {
            command_line
        };
        let started = (|| -> Result<Option<std::time::SystemTime>, String> {
            let (executable, args) = command(&command_line, &path)?;
            write_private(&directory, &path, &source).map_err(|e| e.to_string())?;
            // Capture our write before launching: even an editor that saves
            // immediately must trigger the first reload.
            let stamp = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            std::process::Command::new(executable)
                .args(args)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(stamp)
        })();
        let mut stamp = match started {
            Ok(stamp) => stamp,
            Err(e) => {
                let _ = tx.send(failure(e));
                let _ = fs::remove_file(&path);
                let _ = fs::remove_dir(&directory);
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
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&directory);
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
    fn external_export_is_private_and_never_reuses_a_path() {
        let directory = std::env::temp_dir().join(format!("aurora-script-export-test-{}", uuid::Uuid::new_v4()));
        let path = directory.join("source.lsl");
        write_private(&directory, &path, "default {}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "default {}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&directory), 0o700);
            assert_eq!(mode(&path), 0o600);
        }
        // A folder already there (planted by another account) is refused.
        assert!(write_private(&directory, &path, "x").is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
