//! The interactive logon's pipe access, shared by normal and elevated launches.
//! Windows' default pipe descriptor can name the elevated token's owner instead
//! of its unelevated logon. Use the logon SID (not all interactive users), a
//! medium integrity label, and individual rights: GENERIC_WRITE also permits
//! creating server instances (Microsoft, "Named Pipe Security and Access Rights").

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_GROUPS, TOKEN_QUERY, TokenElevation, TokenLogonSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_CREATE_PIPE_INSTANCE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES, FILE_WRITE_DATA, SYNCHRONIZE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

// The native file-open path and pipe identity query need attribute access
// too. An AccessCheck on WRITE_DATA alone does not test that Windows path.
pub(super) const CLIENT_ACCESS: u32 = FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
pub(super) fn sddl(logon: &str, elevated: bool) -> String {
    // Older clients requested GENERIC_WRITE. Preserve it only within the
    // same trust level: the logon for a normal viewer, its elevated owner
    // otherwise. A normal client reaching an elevated owner gets only the
    // request rights. OW also replaces the owner's implicit WRITE_DAC.
    // Fix the elevated owner explicitly: a token whose default owner was
    // changed to the user's SID must not give its normal token owner rights.
    let identity = if elevated { "O:BAG:BA" } else { "" };
    let owner = FILE_GENERIC_READ | if elevated { FILE_GENERIC_WRITE } else { FILE_CREATE_PIPE_INSTANCE };
    let client = if elevated { CLIENT_ACCESS } else { FILE_GENERIC_WRITE };
    format!("{identity}D:P(A;;0x{owner:x};;;OW)(A;;0x{client:x};;;{logon})S:(ML;;NW;;;ME)")
}

/// A LocalAlloc allocation returned by the security conversion functions.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: both conversion functions allocate with LocalAlloc; this
        // allocation has one owner and no pointers are used after its drop.
        unsafe { LocalFree(self.0) };
    }
}

pub(super) struct Attributes(Local);

impl Attributes {
    pub(super) fn new() -> io::Result<Self> {
        let (logon, elevated) = logon_context()?;
        Self::for_logon(&logon, elevated)
    }

    pub(super) fn for_logon(logon: &str, elevated: bool) -> io::Result<Self> {
        Self::from_sddl(&sddl(logon, elevated))
    }

    pub(super) fn from_sddl(sddl: &str) -> io::Result<Self> {
        let sddl: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: a terminated UTF-16 string and valid output pointer. On
        // success the descriptor is owned by Local until CreateNamedPipeW
        // has copied it; the callers retain Self across that call.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut())
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(Local(descriptor)))
    }

    pub(super) fn get(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: 0,
        }
    }
}

pub(super) fn logon_context() -> io::Result<(String, bool)> {
    let mut token = std::ptr::null_mut();
    // SAFETY: the current process pseudo handle and a valid output pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken succeeded and transferred one owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    // SAFETY: a live token and an exactly sized TOKEN_ELEVATION output.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of_val(&elevation) as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // TokenLogonSid returns one TOKEN_GROUPS entry and its SID. This aligned
    // buffer holds the header and even a maximum-size SID on 32 and 64 bits.
    let mut storage = [0usize; 32];
    // SAFETY: the buffer's byte length is exact, its alignment is sufficient
    // for TOKEN_GROUPS, and token stays open until after the query.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenLogonSid,
            storage.as_mut_ptr().cast(),
            std::mem::size_of_val(&storage) as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if (returned as usize) < std::mem::size_of::<TOKEN_GROUPS>() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "missing logon SID"));
    }
    // SAFETY: the successful query wrote an aligned TOKEN_GROUPS header.
    let groups = unsafe { &*storage.as_ptr().cast::<TOKEN_GROUPS>() };
    if groups.GroupCount != 1 || groups.Groups[0].Sid.is_null() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "missing logon SID"));
    }
    let mut text = std::ptr::null_mut();
    // SAFETY: the SID belongs to the query's live buffer; text receives an
    // allocated, terminated UTF-16 string only on success.
    if unsafe { ConvertSidToStringSidW(groups.Groups[0].Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let allocation = Local(text.cast());
    // SAFETY: ConvertSidToStringSidW guarantees a terminated string. Local
    // keeps its allocation alive until the UTF-16 contents have been copied.
    let sid = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    drop(allocation);
    Ok((sid, elevation.TokenIsElevated != 0))
}
