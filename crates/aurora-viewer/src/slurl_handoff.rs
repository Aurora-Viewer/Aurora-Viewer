//! A SLURL opened while Aurora already runs goes to that viewer instead of
//! starting a second one, as LLAppViewer::initConfiguration hands the
//! startup SLURL to the other instance and quits
//! (LLAppViewerWin32::sendURLToOtherInstance, FindWindow + WM_COPYDATA;
//! LLViewerWindow::handleDataCopy; indra/newview/llappviewer.cpp,
//! llappviewerwin32.cpp, llviewerwindow.cpp, originally LGPL 2.1).
//!
//! Aurora uses a named pipe of the user's session instead of a window
//! message (winit owns the window procedure): the first viewer creates it
//! and a thread reads the links sent to it; a viewer started with a SLURL
//! writes its link there and stops before opening a window. The pipe's
//! default security lets only its creator's account write to it, and remote
//! clients are refused. Demo mode and `--title` test windows use pipes of
//! their own, so a test never takes the links meant for the human's viewer.

use crossbeam_channel::Receiver;

/// Longest link read from the pipe (a SLURL is far shorter).
#[cfg(windows)]
const MAX_URL: usize = 8 * 1024;

/// `\\.\pipe\aurora-viewer-slurl-<session>[-demo][-<title>]`.
#[cfg(windows)]
fn pipe_name() -> String {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    let mut session = 0u32;
    // SAFETY: a valid pointer to a u32 the call fills; on failure it stays 0.
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
    }
    let mut name = format!(r"\\.\pipe\aurora-viewer-slurl-{session}");
    if std::env::var_os("AURORA_DEMO").is_some() {
        name.push_str("-demo");
    }
    if let Some(slug) = crate::cli::title_slug() {
        name.push('-');
        name.push_str(&slug);
    }
    name
}

/// Sends the link to the viewer already running; false when there is none
/// (this process then starts with it).
#[cfg(windows)]
pub fn hand_off(url: &str) -> bool {
    use std::io::Write;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    const ERROR_PIPE_BUSY: i32 = 231;
    let name = pipe_name();
    // the other viewer reads one link at a time: a second click within the
    // same instant waits for its turn
    for _ in 0..20 {
        match std::fs::OpenOptions::new().write(true).open(&name) {
            Ok(mut pipe) => {
                // started by the user's click, this process may bring the
                // other window to the front; it gives that right away
                // SAFETY: no pointer; a refusal only leaves the window behind.
                unsafe {
                    AllowSetForegroundWindow(ASFW_ANY);
                }
                return pipe.write_all(url.as_bytes()).is_ok();
            }
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
    false
}

#[cfg(not(windows))]
pub fn hand_off(_url: &str) -> bool {
    false
}

/// Starts receiving the links of later launches, when no other viewer of
/// this session (same demo mode and title) receives them already.
#[cfg(windows)]
pub fn listen() -> Option<Receiver<String>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
    let name = pipe_name();
    let spawned = std::thread::Builder::new()
        .name("aurora-slurl-pipe".into())
        .spawn(move || serve(&name, &ready_tx, &tx));
    if let Err(e) = spawned {
        log::warn!("startup links: no pipe thread ({e})");
        return None;
    }
    ready_rx.recv().unwrap_or(false).then_some(rx)
}

#[cfg(not(windows))]
pub fn listen() -> Option<Receiver<String>> {
    None
}

/// The pipe's thread: one client at a time on a single instance, so the
/// name exists for the viewer's whole life (clients that come while one is
/// read wait, ERROR_PIPE_BUSY).
#[cfg(windows)]
fn serve(name: &str, ready: &crossbeam_channel::Sender<bool>, links: &crossbeam_channel::Sender<String>) {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, ReadFile};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string alive for the call;
    // a null security descriptor takes the default one (creator only).
    let pipe = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            MAX_URL as u32,
            0,
            std::ptr::null(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        // another viewer of this session already receives the links
        log::info!("startup links: received by another viewer");
        let _ = ready.send(false);
        return;
    }
    let _ = ready.send(true);
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `pipe` is the valid handle created above; no overlapped I/O.
        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            log::warn!("startup links: pipe stopped (error {})", unsafe { GetLastError() });
            break;
        }
        let mut url = Vec::new();
        loop {
            let mut read = 0u32;
            // SAFETY: `buf` outlives the call and `read` receives the count.
            let ok = unsafe { ReadFile(pipe, buf.as_mut_ptr(), buf.len() as u32, &mut read, std::ptr::null_mut()) } != 0;
            if !ok || read == 0 {
                break;
            }
            url.extend_from_slice(&buf[..read as usize]);
            if url.len() > MAX_URL {
                url.clear();
                break;
            }
        }
        // SAFETY: the same valid handle; ends this client's connection.
        unsafe {
            DisconnectNamedPipe(pipe);
        }
        let url = String::from_utf8_lossy(&url).trim().to_owned();
        if !url.is_empty() && links.send(url).is_err() {
            break;
        }
    }
    // SAFETY: the handle is not used after this.
    unsafe {
        CloseHandle(pipe);
    }
}
