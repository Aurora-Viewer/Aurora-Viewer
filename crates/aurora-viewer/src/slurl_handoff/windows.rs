//! Windows SLurl handoff, isolated from the session-bus backend.

use super::{MAX_URL, Receiver};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

mod security;

/// `\\.\pipe\aurora-viewer-slurl-<session>[-demo][-<title>]`.
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
pub fn hand_off(url: &str) -> bool {
    hand_off_to(&pipe_name(), url)
}

fn hand_off_to(name: &str, url: &str) -> bool {
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    const ERROR_PIPE_BUSY: i32 = 231;
    // the other viewer reads one link at a time: a second click within the
    // same instant waits for its turn
    for _ in 0..20 {
        match std::fs::OpenOptions::new()
            .access_mode(security::CLIENT_ACCESS)
            // A substituted server must not impersonate the launching process.
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name)
        {
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

/// Starts receiving the links of later launches, when no other viewer of
/// this session (same demo mode and title) receives them already.
pub fn listen() -> Option<Receiver<String>> {
    listen_on(pipe_name())
}

fn listen_on(name: String) -> Option<Receiver<String>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("aurora-slurl-pipe".into())
        .spawn(move || serve(&name, &ready_tx, &tx));
    if let Err(e) = spawned {
        log::warn!("startup links: no pipe thread ({e})");
        return None;
    }
    ready_rx.recv().unwrap_or(false).then_some(rx)
}

/// The pipe's thread: one client at a time on a single instance, so the
/// name exists for the viewer's whole life (clients that come while one is
/// read wait, ERROR_PIPE_BUSY).
fn serve(name: &str, ready: &crossbeam_channel::Sender<bool>, links: &crossbeam_channel::Sender<String>) {
    use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError};
    use windows_sys::Win32::Storage::FileSystem::ReadFile;
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, DisconnectNamedPipe};
    let security = match security::Attributes::new() {
        Ok(security) => Some(security),
        Err(error) => {
            // Do not widen access when the logon's descriptor is unavailable.
            // Default security retains the ordinary, same-elevation handoff.
            log::warn!("startup links: default pipe security retained ({error})");
            None
        }
    };
    let pipe = match create_pipe(name, security.as_ref()) {
        Ok(pipe) => pipe,
        Err(error) => {
            log::info!("startup links: pipe not owned by this viewer ({error})");
            let _ = ready.send(false);
            return;
        }
    };
    drop(security);
    let handle = pipe.as_raw_handle();
    let _ = ready.send(true);
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `pipe` is the valid handle created above; no overlapped I/O.
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            log::warn!("startup links: pipe stopped (error {})", unsafe { GetLastError() });
            break;
        }
        let mut url = Vec::new();
        loop {
            let mut read = 0u32;
            // SAFETY: `buf` outlives the call and `read` receives the count.
            let ok = unsafe { ReadFile(handle, buf.as_mut_ptr(), buf.len() as u32, &mut read, std::ptr::null_mut()) } != 0;
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
            DisconnectNamedPipe(handle);
        }
        let url = String::from_utf8_lossy(&url).trim().to_owned();
        if !url.is_empty() && links.send(url).is_err() {
            break;
        }
    }
}

fn create_pipe(name: &str, security: Option<&security::Attributes>) -> io::Result<OwnedHandle> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
    use windows_sys::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let attributes = security.map(security::Attributes::get);
    // SAFETY: both the terminated name and the optional descriptor outlive
    // the call. Windows copies the descriptor; the handle is not inherited.
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            MAX_URL as u32,
            0,
            attributes.as_ref().map_or(std::ptr::null(), |attributes| attributes),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateNamedPipeW succeeded and returned one owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

#[cfg(test)]
mod tests;
