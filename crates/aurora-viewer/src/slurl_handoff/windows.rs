//! Windows SLurl handoff, isolated from the session-bus backend.

use super::{MAX_URL, Receiver};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

mod discovery;
mod security;

use discovery::Scope;

static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn window_ready(window: &std::sync::Arc<winit::window::Window>) {
    if READY.load(std::sync::atomic::Ordering::Acquire) {
        discovery::publish(window);
    }
}

pub fn focused() {
    discovery::focused();
}

pub fn release() {
    READY.store(false, std::sync::atomic::Ordering::Release);
    discovery::release();
}

/// `\\.\pipe\aurora-viewer-slurl-<session>[-demo][-<title>]`.
fn pipe_name() -> String {
    legacy_name(&Scope::current())
}

fn legacy_name(scope: &Scope) -> String {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    let mut session = 0u32;
    // SAFETY: a valid pointer to a u32 the call fills; on failure it stays 0.
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
    }
    let mut name = format!(r"\\.\pipe\aurora-viewer-slurl-{session}");
    if scope.demo {
        name.push_str("-demo");
    }
    if let Some(slug) = &scope.title {
        name.push('-');
        name.push_str(slug);
    }
    name
}

fn instance_name(pid: u32) -> String {
    format!("{}-v2-{pid}", legacy_name(&Scope { demo: false, title: None }))
}

/// Sends the link to the viewer already running; false when there is none
/// (this process then starts with it).
pub fn hand_off(url: &str) -> bool {
    route_to(&Scope::current(), url) || hand_off_to(&pipe_name(), url)
}

fn route_to(scope: &Scope, url: &str) -> bool {
    discovery::candidates(scope)
        .into_iter()
        .any(|candidate| send_to(&instance_name(candidate.pid), url, Some(candidate.pid)))
}

fn hand_off_to(name: &str, url: &str) -> bool {
    send_to(name, url, None)
}

fn send_to(name: &str, url: &str, expected_pid: Option<u32>) -> bool {
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
    const ERROR_PIPE_BUSY: i32 = 231;
    if url.is_empty() || url.len() > MAX_URL {
        return false;
    }
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
                let mut pid = 0;
                // SAFETY: a live client pipe and valid output pointer. The
                // PID must match the discovered HWND, preventing a different
                // server from taking a reused or pre-created endpoint name.
                if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0
                    || expected_pid.is_some_and(|expected| expected != pid)
                {
                    return false;
                }
                // started by the user's click, this process may bring the
                // other window to the front; it gives that right away
                // SAFETY: no pointer; a refusal only leaves the window behind.
                unsafe {
                    AllowSetForegroundWindow(pid);
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

/// Every instance owns an endpoint. The old shared name remains a relay for
/// registered handlers from an older installation, which know no HWND/PID.
pub fn listen() -> Option<Receiver<String>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    if !start_receiver(instance_name(std::process::id()), tx.clone(), None) {
        return None;
    }
    let scope = Scope::current();
    start_receiver(legacy_name(&scope), tx.clone(), Some(scope.clone()));
    if !scope.demo && scope.title.is_some() {
        let online = Scope { demo: false, title: None };
        start_receiver(legacy_name(&online), tx, Some(online));
    }
    READY.store(true, std::sync::atomic::Ordering::Release);
    Some(rx)
}

#[cfg(test)]
fn listen_on(name: String) -> Option<Receiver<String>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    start_receiver(name, tx, None).then_some(rx)
}

fn start_receiver(name: String, tx: crossbeam_channel::Sender<String>, relay: Option<Scope>) -> bool {
    let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("aurora-slurl-pipe".into())
        .spawn(move || serve(&name, &ready_tx, &tx, relay.as_ref()));
    if let Err(e) = spawned {
        log::warn!("startup links: no pipe thread ({e})");
        return false;
    }
    ready_rx.recv().unwrap_or(false)
}

/// The pipe's thread: one client at a time on a single instance, so the
/// name exists for the viewer's whole life (clients that come while one is
/// read wait, ERROR_PIPE_BUSY).
fn serve(name: &str, ready: &crossbeam_channel::Sender<bool>, links: &crossbeam_channel::Sender<String>, relay: Option<&Scope>) {
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
    let _ = ready.send(true);
    let mut buf = vec![0u8; 1024];
    loop {
        let url = match read_request(&pipe, &mut buf) {
            Ok(url) => url,
            Err(error) => {
                log::warn!("startup links: pipe stopped ({error})");
                break;
            }
        };
        if !url.is_empty() && !relay.is_some_and(|scope| route_to(scope, &url)) && links.send(url).is_err() {
            break;
        }
    }
}

fn read_request(pipe: &OwnedHandle, buf: &mut [u8]) -> io::Result<String> {
    use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, GetLastError};
    use windows_sys::Win32::Storage::FileSystem::ReadFile;
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, DisconnectNamedPipe};
    let handle = pipe.as_raw_handle();
    // SAFETY: the caller retains the live, synchronous server handle.
    if unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } == 0 {
        // SAFETY: read this thread's error immediately after the call.
        let error = unsafe { GetLastError() };
        // A client can write and close before this accept. Its buffered
        // bytes remain readable: disconnecting now would discard the link.
        if !matches!(error, ERROR_PIPE_CONNECTED | ERROR_NO_DATA | ERROR_BROKEN_PIPE) {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
    }
    let mut url = Vec::new();
    loop {
        let mut read = 0u32;
        // SAFETY: buf and the count outlive the synchronous read.
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
    unsafe { DisconnectNamedPipe(handle) };
    Ok(String::from_utf8_lossy(&url).trim().to_owned())
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
