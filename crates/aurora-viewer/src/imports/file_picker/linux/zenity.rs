//! rfd's no-portal fallback, with ownership of the exact child process.
use super::super::Kind;
use super::{Cancellation, PathBuf, Request};
use std::{
    io::{self, Read},
    os::unix::ffi::OsStringExt,
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Picker(Child);

impl Drop for Picker {
    fn drop(&mut self) {
        // Child::kill addresses this unreaped child only, never another picker.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn select(request: &Request, cancellation: &Cancellation) -> io::Result<Vec<PathBuf>> {
    if cancellation.cancelled() {
        return Ok(Vec::new());
    }
    let mut command = Command::new("zenity");
    command.args(["--file-selection", "--title", &request.title]);
    match request.kind {
        Kind::Files => {
            command.args(["--multiple", "--separator=\n"]);
        }
        Kind::Folder => {
            command.arg("--directory");
        }
        Kind::File => {}
    }
    if !request.extensions.is_empty() {
        let patterns: Vec<_> = request.extensions.iter().map(|ext| format!("*.{ext}")).collect();
        command
            .arg("--file-filter")
            .arg(format!("{} | {}", request.label, patterns.join(" ")));
    }
    let mut child = Picker(command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?);
    let mut stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("file picker stdout missing"))?;
    // Drain concurrently: a large multi-selection must not fill the pipe and
    // prevent the child from exiting. The picker worker, never a frame, waits.
    let reader = std::thread::Builder::new()
        .name("aurora-import-picker-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        })?;
    let status = loop {
        if cancellation.cancelled() {
            break None;
        }
        match child.0.try_wait() {
            Ok(Some(status)) => break Some(Ok(status)),
            Err(error) => break Some(Err(error)),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    drop(child); // closes the dialog on cancel/error, then reaps the process
    let bytes = reader
        .join()
        .map_err(|_| io::Error::other("file picker output reader panicked"))??;
    let Some(status) = status else { return Ok(Vec::new()) };
    if !status?.success() || cancellation.cancelled() {
        return Ok(Vec::new());
    }
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    let paths = if request.kind == Kind::Files {
        bytes.split(|byte| *byte == b'\n').collect()
    } else {
        vec![bytes]
    };
    Ok(paths
        .into_iter()
        .filter(|path| !path.is_empty())
        .map(|path| std::ffi::OsString::from_vec(path.to_vec()).into())
        .collect())
}
