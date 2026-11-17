//! What a client proves itself with: a secret written to a new file only the
//! user can read. The client is told the file's path and answers with what
//! it says; a program of another account of the computer, or a web page,
//! cannot read it.
//!
//! Port of LLScriptEditorWSConnection::writeChallenge / writeUserOnlyFile
//! (indra/newview/llscripteditorws.cpp, originally LGPL 2.1): 128 random
//! bits from the system's generator, written as a UUID to
//! `sl_script_challenge_<random id>.tmp` in the temp folder, created new
//! (never over an existing file or through a link), readable by the user
//! alone (Windows: a protected DACL with the user's SID only; elsewhere
//! 0600), and removed once answered, whatever the answer. The answer is
//! compared in constant time (stricter).

use serde_json::Value;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct Challenge {
    secret: Uuid,
    path: PathBuf,
}

impl Challenge {
    /// Writes a new challenge in the system's temp folder.
    pub fn create() -> io::Result<Challenge> {
        Challenge::create_in(&std::env::temp_dir())
    }

    pub fn create_in(dir: &Path) -> io::Result<Challenge> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
        let secret = Uuid::from_bytes(bytes);
        // named for nothing it holds: the name goes to whoever connects
        let path = dir.join(format!("sl_script_challenge_{}.tmp", Uuid::new_v4()));
        let mut file = create_user_only(&path)?;
        let written = file
            .write_all(secret.hyphenated().to_string().as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(e) = written {
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        Ok(Challenge { secret, path })
    }

    /// The path told to the client (`challenge` of `session.handshake`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the client's `challenge_response` is what the file says.
    /// Legacy reads it with `asUUID()`: any spelling of the same UUID is
    /// taken, a trailing newline too.
    pub fn answered_by(&self, answer: Option<&Value>) -> bool {
        let Some(Value::String(text)) = answer else {
            return false;
        };
        let Ok(given) = Uuid::parse_str(text.trim()) else {
            return false;
        };
        given
            .as_bytes()
            .iter()
            .zip(self.secret.as_bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl Drop for Challenge {
    fn drop(&mut self) {
        // whatever the answer, the file has done its work
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn create_user_only(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // create_new is O_EXCL: it fails on an existing file and never follows
    // a link left in the shared temp folder
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)
}

#[cfg(windows)]
fn create_user_only(path: &Path) -> io::Result<fs::File> {
    windows::create_user_only(path)
}

#[cfg(not(any(unix, windows)))]
fn create_user_only(_path: &Path) -> io::Result<fs::File> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no user-only files on this system"))
}

/// Creates a new folder only the user can enter (0700; Windows: a protected
/// DACL for the user's SID, inherited by what is created in it). It fails
/// on an existing folder: never one another account made first. For the
/// exported scripts, which an editor finds by listing the folder.
pub fn create_user_only_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(windows)]
    {
        windows::create_user_only_dir(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(io::ErrorKind::Unsupported, "no user-only folders on this system"))
    }
}

#[cfg(windows)]
mod windows {
    //! The file's DACL, explicitly: Windows ignores POSIX modes, and a temp
    //! folder whose ACL was widened would let other accounts in.
    use std::ffi::c_void;
    use std::fs;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;
    use windows_sys::Win32::Foundation::{INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::Storage::FileSystem::{CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// A LocalAlloc allocation of the conversion functions.
    struct Local(*mut c_void);
    impl Drop for Local {
        fn drop(&mut self) {
            // SAFETY: both conversion functions allocate with LocalAlloc;
            // this allocation has one owner and is not used after its drop.
            unsafe { LocalFree(self.0) };
        }
    }

    /// The current user's SID, as text (`S-1-5-21-…`).
    pub(super) fn user_sid() -> io::Result<String> {
        let mut token = std::ptr::null_mut();
        // SAFETY: the current process pseudo handle and a valid output pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken succeeded and transferred one owned handle.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        // TOKEN_USER and its SID: an aligned buffer that holds the header and
        // even a maximum-size SID (68 bytes) on 32 and 64 bits.
        let mut storage = [0usize; 32];
        let mut returned = 0;
        // SAFETY: the buffer's byte length is exact, its alignment suits
        // TOKEN_USER, and the token stays open until after the query.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                std::mem::size_of_val(&storage) as u32,
                &mut returned,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if (returned as usize) < std::mem::size_of::<TOKEN_USER>() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "missing user SID"));
        }
        // SAFETY: the successful query wrote an aligned TOKEN_USER header.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        if user.User.Sid.is_null() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "missing user SID"));
        }
        let mut text = std::ptr::null_mut();
        // SAFETY: the SID lives in the query's buffer; text receives an
        // allocated, terminated UTF-16 string only on success.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let allocation = Local(text.cast());
        // SAFETY: ConvertSidToStringSidW returns a terminated string, kept
        // alive by `allocation` until it has been copied.
        let sid = unsafe {
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        };
        drop(allocation);
        Ok(sid)
    }

    /// The descriptor of the file: protected (nothing inherited from the
    /// folder), one entry, full access for the user alone.
    pub(super) fn sddl(sid: &str) -> String {
        format!("D:P(A;;FA;;;{sid})")
    }

    /// The descriptor of a folder: the same entry, inherited by the files
    /// and folders created in it (OICI).
    pub(super) fn dir_sddl(sid: &str) -> String {
        format!("D:P(A;OICI;FA;;;{sid})")
    }

    /// The user's SID, checked to be one: letters, digits and dashes,
    /// nothing that could end the ACE.
    fn checked_sid() -> io::Result<String> {
        let sid = user_sid()?;
        if !sid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected user SID"));
        }
        Ok(sid)
    }

    /// A security descriptor made from its SDDL text.
    fn descriptor(text: &str) -> io::Result<Local> {
        let sddl: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: a terminated UTF-16 string and a valid output pointer; on
        // success the descriptor is owned by `Local`, which the caller keeps
        // until the creation call has applied it.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut())
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Local(descriptor))
    }

    pub(super) fn create_user_only_dir(path: &Path) -> io::Result<()> {
        let descriptor = descriptor(&dir_sddl(&checked_sid()?))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: a terminated path, attributes whose descriptor outlives the
        // call. CreateDirectoryW fails on an existing folder.
        let created = unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) };
        drop(descriptor);
        if created == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn create_user_only(path: &Path) -> io::Result<fs::File> {
        let descriptor = descriptor(&sddl(&checked_sid()?))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: a terminated path, attributes whose descriptor outlives the
        // call. CREATE_NEW fails on an existing file (as LLFile::noreplace);
        // share mode 0 keeps it closed to others while it is written.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_WRITE,
                0,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        drop(descriptor);
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateFileW succeeded and transferred one owned handle.
        Ok(fs::File::from(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    #[cfg(test)]
    pub(super) fn file_sddl(path: &Path) -> io::Result<String> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
        };
        use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: a terminated path; only the descriptor is asked for, which
        // the call allocates with LocalAlloc.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let descriptor = Local(descriptor);
        let mut text = std::ptr::null_mut();
        // SAFETY: a valid descriptor; text is a LocalAlloc string on success.
        if unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor.0,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let allocation = Local(text.cast());
        // SAFETY: a terminated string kept alive by `allocation`.
        let sddl = unsafe {
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        };
        drop(allocation);
        drop(descriptor);
        Ok(sddl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-editor-sync-test-{}", Uuid::new_v4()));
        fs::create_dir(&dir).expect("test folder");
        dir
    }

    #[test]
    fn the_file_holds_the_secret_and_goes_away_with_the_challenge() {
        let dir = dir();
        let challenge = Challenge::create_in(&dir).expect("challenge");
        let path = challenge.path().to_path_buf();
        let name = path.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(name.starts_with("sl_script_challenge_") && name.ends_with(".tmp"));
        let text = fs::read_to_string(&path).expect("readable by its user");
        assert!(challenge.answered_by(Some(&json!(text))));
        assert!(challenge.answered_by(Some(&json!(format!("{}\n", text.to_uppercase())))));
        assert!(!challenge.answered_by(Some(&json!(Uuid::new_v4().to_string()))));
        assert!(!challenge.answered_by(Some(&json!(Uuid::nil().to_string()))));
        assert!(!challenge.answered_by(Some(&json!(""))));
        assert!(!challenge.answered_by(Some(&json!(1))));
        assert!(!challenge.answered_by(Some(&json!(null))));
        assert!(!challenge.answered_by(Some(&json!([text]))));
        assert!(!challenge.answered_by(None));
        drop(challenge);
        assert!(!path.exists());
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn two_challenges_never_share_a_secret_or_a_file() {
        let dir = dir();
        let a = Challenge::create_in(&dir).expect("a");
        let b = Challenge::create_in(&dir).expect("b");
        assert_ne!(a.path(), b.path());
        assert_ne!(a.secret, b.secret);
        let b_text = fs::read_to_string(b.path()).expect("b");
        assert!(!a.answered_by(Some(&json!(b_text))));
        drop((a, b));
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn an_existing_file_is_never_written_over() {
        let dir = dir();
        let path = dir.join("taken");
        fs::write(&path, "x").expect("file");
        assert!(create_user_only(&path).is_err());
        assert_eq!(fs::read_to_string(&path).expect("file"), "x");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn readable_by_its_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let challenge = Challenge::create_in(&dir).expect("challenge");
        let mode = fs::metadata(challenge.path()).expect("file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        drop(challenge);
        let _ = fs::remove_dir(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn readable_by_its_user_only() {
        let dir = dir();
        let challenge = Challenge::create_in(&dir).expect("challenge");
        let sid = windows::user_sid().expect("sid");
        // protected, one entry: full access for the user, nothing inherited
        assert_eq!(windows::file_sddl(challenge.path()).expect("descriptor"), windows::sddl(&sid));
        drop(challenge);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn a_user_only_folder_is_new_and_closed_to_others() {
        let dir = dir();
        let folder = dir.join("scripts");
        create_user_only_dir(&folder).expect("folder");
        // never one that is already there
        assert!(create_user_only_dir(&folder).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&folder).expect("folder").permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        #[cfg(windows)]
        {
            let sid = windows::user_sid().expect("sid");
            assert_eq!(windows::file_sddl(&folder).expect("descriptor"), windows::dir_sddl(&sid));
        }
        let _ = fs::remove_dir(&folder);
        let _ = fs::remove_dir(&dir);
    }
}
