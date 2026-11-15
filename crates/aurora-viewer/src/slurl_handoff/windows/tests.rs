use super::*;
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

fn name() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        r"\\.\pipe\aurora-slurl-unit-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

struct Listener {
    name: String,
    rx: Option<Receiver<String>>,
}

impl Listener {
    fn new() -> Self {
        let name = name();
        let rx = listen_on(name.clone()).expect("isolated native pipe");
        Self { name, rx: Some(rx) }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        drop(self.rx.take());
        // Wake the worker, which then observes the disconnected receiver.
        let _ = hand_off_to(&self.name, "secondlife://Test/1/2/3");
    }
}

#[test]
fn links_reach_the_running_viewer_in_order() {
    let listener = Listener::new();
    for index in 0..8 {
        let url = format!("secondlife://Test%20Region/{index}/128/20");
        assert!(hand_off_to(&listener.name, &url));
        assert_eq!(listener.rx.as_ref().unwrap().recv_timeout(Duration::from_secs(2)).unwrap(), url);
    }
}

#[test]
fn no_viewer_still_starts_with_the_link() {
    assert!(!hand_off_to(&name(), "secondlife://Test/128/128/20"));
}

#[test]
fn an_endpoint_owned_by_another_pid_does_not_receive_the_link() {
    let listener = Listener::new();
    assert!(!send_to(&listener.name, "secondlife://Test/1/2/3", Some(0)));
    assert!(listener.rx.as_ref().unwrap().try_recv().is_err());
    assert!(send_to(&listener.name, "secondlife://Test/1/2/3", Some(std::process::id())));
    assert_eq!(
        listener.rx.as_ref().unwrap().recv_timeout(Duration::from_secs(2)).unwrap(),
        "secondlife://Test/1/2/3"
    );
}

#[test]
fn aborted_connections_do_not_stop_the_receiver() {
    let listener = Listener::new();
    for _ in 0..32 {
        assert!(!send_to(&listener.name, "secondlife://Test/1/2/3", Some(0)));
    }
    assert!(hand_off_to(&listener.name, "secondlife://Test/1/2/3"));
    assert_eq!(
        listener.rx.as_ref().unwrap().recv_timeout(Duration::from_secs(2)).unwrap(),
        "secondlife://Test/1/2/3"
    );
}

#[test]
fn a_client_closed_before_accept_still_delivers_its_buffered_link() {
    let security = security::Attributes::new().unwrap();
    let mut buf = [0; 1024];
    for url in [
        "secondlife://Test/11/128/25".to_owned(),
        format!("secondlife://{}/22/128/25", "Région".repeat(300)),
    ] {
        let name = name();
        let pipe = create_pipe(&name, Some(&security)).unwrap();
        assert!(hand_off_to(&name, &url));
        // A newly created pipe can receive a client before its first accept.
        // hand_off_to has already closed that client, with data still buffered.
        assert_eq!(read_request(&pipe, &mut buf).unwrap(), url);
    }
}

#[test]
fn an_oversized_link_is_refused_before_connecting() {
    let listener = Listener::new();
    assert!(!hand_off_to(&listener.name, &"x".repeat(MAX_URL + 1)));
    assert!(listener.rx.as_ref().unwrap().try_recv().is_err());
    assert!(hand_off_to(&listener.name, "secondlife://Test/1/2/3"));
    assert_eq!(
        listener.rx.as_ref().unwrap().recv_timeout(Duration::from_secs(2)).unwrap(),
        "secondlife://Test/1/2/3"
    );
}

#[test]
fn native_open_requires_attribute_access_in_addition_to_data_writing() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_WRITE_DATA, SYNCHRONIZE};
    let logon = security::logon_context().unwrap().0;
    let descriptor = |rights| {
        // Suppress the owner's implicit rights so this tests the actual
        // client ACE even when the server and client are the same process.
        security::Attributes::from_sddl(&format!("D:P(A;;RC;;;OW)(A;;0x{rights:x};;;{logon})S:(ML;;NW;;;ME)")).unwrap()
    };
    let old_rights = FILE_WRITE_DATA | SYNCHRONIZE;
    let old_security = descriptor(old_rights);
    let old_name = name();
    let _old_pipe = create_pipe(&old_name, Some(&old_security)).unwrap();
    let error = std::fs::OpenOptions::new()
        .access_mode(old_rights)
        .open(&old_name)
        .expect_err("native open needs attribute access");
    assert_eq!(error.raw_os_error(), Some(5));
    let fixed_security = descriptor(old_rights | FILE_READ_ATTRIBUTES);
    let fixed_name = name();
    let pipe = create_pipe(&fixed_name, Some(&fixed_security)).unwrap();
    assert!(hand_off_to(&fixed_name, "secondlife://Test/1/2/3"));
    let mut bytes = vec![0; "secondlife://Test/1/2/3".len()];
    std::fs::File::from(pipe).read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, b"secondlife://Test/1/2/3");
}

#[test]
fn a_long_unicode_link_keeps_every_byte_across_pipe_reads() {
    let listener = Listener::new();
    let url = format!("secondlife://{}/128/128/20", "Région".repeat(600));
    assert!(url.len() < MAX_URL);
    assert!(hand_off_to(&listener.name, &url));
    assert_eq!(listener.rx.as_ref().unwrap().recv_timeout(Duration::from_secs(2)).unwrap(), url);
}

#[test]
fn an_administrator_owner_does_not_block_the_normal_logon_or_grant_it_server_rights() {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{
        AccessCheck, CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, DuplicateTokenEx, GENERIC_MAPPING, PRIVILEGE_SET, SID_AND_ATTRIBUTES,
        SecurityImpersonation, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY, TokenImpersonation,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_CREATE_PIPE_INSTANCE, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, WRITE_DAC, WRITE_OWNER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // An elevated process can own objects as Administrators. Disable that
    // group in a duplicate, even on an elevated CI runner, to model the
    // normal browser launch. No real process elevation is requested.
    let sid_text: Vec<u16> = "S-1-5-32-544".encode_utf16().chain([0]).collect();
    let mut admin = std::ptr::null_mut();
    // SAFETY: a terminated constant and valid output pointer.
    assert_ne!(unsafe { ConvertStringSidToSidW(sid_text.as_ptr(), &mut admin) }, 0);
    let mut token = std::ptr::null_mut();
    // SAFETY: this process's pseudo handle and a valid output pointer.
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut token) },
        0
    );
    // SAFETY: the query returned one owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let disabled = SID_AND_ATTRIBUTES { Sid: admin, Attributes: 0 };
    let mut normal = std::ptr::null_mut();
    // SAFETY: all input/output buffers are live; only a duplicate is changed.
    let restricted = unsafe {
        CreateRestrictedToken(
            token.as_raw_handle(),
            DISABLE_MAX_PRIVILEGE,
            1,
            &disabled,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            &mut normal,
        )
    };
    // SAFETY: the conversion allocated this SID and the call copied it.
    unsafe { LocalFree(admin) };
    assert_ne!(restricted, 0);
    // SAFETY: CreateRestrictedToken returned one owned handle.
    let normal = unsafe { OwnedHandle::from_raw_handle(normal) };
    let mut impersonation = std::ptr::null_mut();
    // SAFETY: AccessCheck needs an impersonation token; this copies normal.
    assert_ne!(
        unsafe {
            DuplicateTokenEx(
                normal.as_raw_handle(),
                TOKEN_QUERY | TOKEN_IMPERSONATE,
                std::ptr::null(),
                SecurityImpersonation,
                TokenImpersonation,
                &mut impersonation,
            )
        },
        0
    );
    // SAFETY: DuplicateTokenEx returned one owned handle.
    let impersonation = unsafe { OwnedHandle::from_raw_handle(impersonation) };
    let allows = |descriptor: &security::Attributes, desired| {
        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ,
            GenericWrite: FILE_GENERIC_WRITE,
            GenericExecute: FILE_GENERIC_EXECUTE,
            GenericAll: FILE_ALL_ACCESS,
        };
        let mut privileges = PRIVILEGE_SET::default();
        let mut bytes = std::mem::size_of_val(&privileges) as u32;
        let mut granted = 0;
        let mut allowed = 0;
        // SAFETY: live descriptor/token and correctly sized output buffers.
        assert_ne!(
            unsafe {
                AccessCheck(
                    descriptor.get().lpSecurityDescriptor,
                    impersonation.as_raw_handle(),
                    desired,
                    &mapping,
                    &mut privileges,
                    &mut bytes,
                    &mut granted,
                    &mut allowed,
                )
            },
            0
        );
        allowed != 0
    };
    let former = security::Attributes::from_sddl("O:BAG:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)").unwrap();
    assert!(
        !allows(&former, security::CLIENT_ACCESS),
        "the elevated owner's default ACL refuses the normal launch"
    );
    let fixed = security::Attributes::from_sddl(&security::sddl(&security::logon_context().unwrap().0, true)).unwrap();
    assert!(allows(&fixed, security::CLIENT_ACCESS), "the interactive logon can submit its link");
    for denied in [WRITE_DAC, WRITE_OWNER, FILE_CREATE_PIPE_INSTANCE, FILE_GENERIC_WRITE] {
        assert!(
            !allows(&fixed, security::CLIENT_ACCESS | denied),
            "normal clients cannot administer or serve the elevated pipe"
        );
    }
}

#[test]
fn client_requests_only_data_writing_and_cannot_administer_the_pipe() {
    use windows_sys::Win32::Storage::FileSystem::{WRITE_DAC, WRITE_OWNER};
    let security = security::Attributes::new().expect("interactive logon's descriptor");
    for denied in [WRITE_DAC, WRITE_OWNER] {
        let name = name();
        let _pipe = create_pipe(&name, Some(&security)).expect("native pipe");
        let error = std::fs::OpenOptions::new()
            .access_mode(security::CLIENT_ACCESS | denied)
            .open(&name)
            .expect_err("client must not receive administrative rights");
        assert_eq!(error.raw_os_error(), Some(5));
        assert!(
            hand_off_to(&name, "secondlife://Test/1/2/3"),
            "the same pipe still accepts a request"
        );
    }
}

#[test]
fn an_older_client_still_reaches_the_same_owners_fixed_viewer() {
    use std::io::Write;
    let security = security::Attributes::new().expect("interactive logon's descriptor");
    let name = name();
    let pipe = create_pipe(&name, Some(&security)).expect("fixed viewer's native pipe");
    let url = "secondlife://Test/128/128/20";
    let mut client = std::fs::OpenOptions::new()
        .write(true)
        .open(&name)
        .expect("old client's GENERIC_WRITE");
    client.write_all(url.as_bytes()).unwrap();
    drop(client);
    let mut pipe = std::fs::File::from(pipe);
    let mut bytes = vec![0; url.len()];
    pipe.read_exact(&mut bytes).expect("buffered request");
    assert_eq!(bytes, url.as_bytes());
}

#[test]
fn a_different_logon_cannot_write_even_if_it_owns_the_pipe() {
    let security = security::Attributes::for_logon("S-1-5-5-4294967295-4294967295", false).expect("valid, unrelated logon SID");
    let name = name();
    let _pipe = create_pipe(&name, Some(&security)).expect("native pipe owned by this process");
    assert!(!hand_off_to(&name, "secondlife://Test/1/2/3"));
}

#[test]
fn a_second_server_cannot_take_the_name() {
    let security = security::Attributes::new().expect("interactive logon's descriptor");
    let name = name();
    let _pipe = create_pipe(&name, Some(&security)).expect("first server");
    assert!(create_pipe(&name, Some(&security)).is_err());
}

#[test]
fn a_client_still_reaches_a_viewer_using_default_security() {
    let name = name();
    let pipe = create_pipe(&name, None).expect("old viewer's native pipe");
    let url = "secondlife://Test/128/128/20";
    assert!(hand_off_to(&name, url));
    let mut pipe = std::fs::File::from(pipe);
    let mut bytes = vec![0; url.len()];
    pipe.read_exact(&mut bytes).expect("buffered request");
    assert_eq!(bytes, url.as_bytes());
}

#[test]
fn a_low_integrity_client_is_denied() {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{
        DuplicateTokenEx, GetLengthSid, ImpersonateLoggedOnUser, RevertToSelf, SID_AND_ATTRIBUTES, SecurityImpersonation,
        SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        TokenImpersonation, TokenIntegrityLevel,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let security = security::Attributes::new().expect("interactive logon's descriptor");
    let name = name();
    let _pipe = create_pipe(&name, Some(&security)).expect("medium integrity pipe");
    let mut token = std::ptr::null_mut();
    // SAFETY: query/duplicate access to this test process's own token.
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut token) },
        0
    );
    // SAFETY: the successful call returned one owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut low = std::ptr::null_mut();
    // SAFETY: a live token, valid output pointer and no inherited attributes.
    assert_ne!(
        unsafe {
            DuplicateTokenEx(
                token.as_raw_handle(),
                TOKEN_QUERY | TOKEN_IMPERSONATE | TOKEN_ADJUST_DEFAULT,
                std::ptr::null(),
                SecurityImpersonation,
                TokenImpersonation,
                &mut low,
            )
        },
        0
    );
    // SAFETY: the successful call returned one owned handle.
    let low = unsafe { OwnedHandle::from_raw_handle(low) };
    let sid_text: Vec<u16> = "S-1-16-4096".encode_utf16().chain([0]).collect();
    let mut sid = std::ptr::null_mut();
    // SAFETY: terminated constant string and valid output pointer.
    assert_ne!(unsafe { ConvertStringSidToSidW(sid_text.as_ptr(), &mut sid) }, 0);
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: sid,
            Attributes: 0x20,
        },
    };
    // SAFETY: the duplicate token and label are live; Windows copies the SID.
    let set = unsafe {
        SetTokenInformation(
            low.as_raw_handle(),
            TokenIntegrityLevel,
            (&label as *const TOKEN_MANDATORY_LABEL).cast(),
            std::mem::size_of_val(&label) as u32 + GetLengthSid(sid),
        )
    };
    // SAFETY: conversion allocated the SID with LocalAlloc; it is now copied.
    unsafe { LocalFree(sid) };
    assert_ne!(set, 0);
    // SAFETY: only this test thread uses this same-user, lower-integrity token.
    assert_ne!(unsafe { ImpersonateLoggedOnUser(low.as_raw_handle()) }, 0);
    let result = std::fs::OpenOptions::new().access_mode(security::CLIENT_ACCESS).open(&name);
    // SAFETY: end thread impersonation before assertions, logging or cleanup.
    let reverted = unsafe { RevertToSelf() };
    assert_ne!(reverted, 0);
    assert_eq!(
        result.expect_err("low integrity must not write to a medium pipe").raw_os_error(),
        Some(5)
    );
    assert!(hand_off_to(&name, "secondlife://Test/1/2/3"), "normal integrity still works");
}
