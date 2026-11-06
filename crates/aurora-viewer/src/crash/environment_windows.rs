//! Windows environment snapshot. All user- or machine-specific identifiers are
//! discarded at collection time: module paths and display device names never
//! leave this module.

use super::environment::{Display, Module, Snapshot};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, LocalFree};
use windows_sys::Win32::Graphics::Gdi::{
    DEVMODEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS, EnumDisplayDevicesW, EnumDisplaySettingsW,
};
use windows_sys::Win32::Security::Cryptography::{
    CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
    CERT_QUERY_FORMAT_FLAG_BINARY, CERT_QUERY_OBJECT_FILE, CMSG_SIGNER_INFO, CMSG_SIGNER_INFO_PARAM, CertCloseStore,
    CertFindCertificateInStore, CertFreeCertificateContext, CertGetNameStringW, CryptMsgClose, CryptMsgGetParam, CryptQueryObject,
    PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
};
use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE,
    WTD_REVOCATION_CHECK_NONE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FFI_SIGNATURE, VS_FIXEDFILEINFO, VerQueryValueW,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W, Process32FirstW, Process32NextW,
    TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, PowerGetActiveScheme, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;

const MAX_MODULES: usize = 1024;
const MAX_PROCESSES: usize = 4096;
const MAX_VERSION_RESOURCE: u32 = 64 * 1024;
const MAX_SIGNER_INFO: u32 = 64 * 1024;
const MAX_SIGNER_NAME_CHARS: u32 = 512;
const MAX_SIGNER_CACHE_ENTRIES: usize = 2048;
static SIGNER_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
const OVERLAY_PROCESSES: &[(&str, &[&str])] = &[
    ("Discord", &["discord.exe", "discordcanary.exe", "discordptb.exe"]),
    ("OBS", &["obs64.exe", "obs32.exe"]),
    ("Steam", &["steam.exe", "gameoverlayui.exe"]),
    ("Overwolf", &["overwolf.exe", "overwolflauncher.exe"]),
    ("Medal", &["medal.exe", "medalencoder.exe"]),
    ("Xbox Game Bar", &["gamebar.exe", "gamebarftserver.exe"]),
    ("NVIDIA overlay", &["nvidia share.exe", "nvidia overlay.exe", "nvidia app.exe"]),
    ("RivaTuner", &["rtss.exe"]),
    ("MSI Afterburner", &["msiafterburner.exe"]),
    ("FPS Monitor", &["fpsmonitor.exe"]),
    ("Bandicam", &["bandicam.exe"]),
    ("AMD overlay", &["radeonsoftware.exe", "amdrsserv.exe", "amdrssrcext.exe"]),
    ("Wallpaper Engine", &["wallpaper32.exe", "wallpaper64.exe"]),
];

pub(super) fn collect() -> Snapshot {
    let mut snapshot = Snapshot::default();
    if let Some(modules) = current_modules() {
        snapshot.modules_collected = true;
        snapshot.modules = modules;
    }
    if let Some(processes) = overlay_processes() {
        snapshot.processes_collected = true;
        snapshot.processes = processes;
    }
    if let Some(displays) = displays() {
        snapshot.displays_collected = true;
        snapshot.displays = displays;
    }
    let (profile, source, collected) = power();
    snapshot.power_profile = profile;
    snapshot.power_source = source;
    snapshot.power_collected = collected;
    snapshot
}

fn file_version(path: &str) -> Option<String> {
    if !is_local_drive_path(path) {
        return None;
    }
    let path = path.encode_utf16().chain([0]).collect::<Vec<_>>();
    let mut handle = 0u32;
    // SAFETY: `path` is nul-terminated UTF-16 and `handle` is writable.
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut handle) };
    if size == 0 || size > MAX_VERSION_RESOURCE {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` is writable for the exact capacity reported by the size
    // query, and the input path remains alive and nul-terminated.
    if unsafe { GetFileVersionInfoW(path.as_ptr(), handle, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let root = "\\".encode_utf16().chain([0]).collect::<Vec<_>>();
    let mut value = std::ptr::null_mut();
    let mut value_len = 0u32;
    // SAFETY: `data` remains alive; the query path is nul-terminated UTF-16;
    // the API fills the output pointer and length within the resource.
    if unsafe { VerQueryValueW(data.as_ptr().cast(), root.as_ptr(), &mut value, &mut value_len) } == 0
        || value.is_null()
        || value_len < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
    {
        return None;
    }
    // SAFETY: VerQueryValueW succeeded and reported at least a complete
    // VS_FIXEDFILEINFO structure inside the still-live byte buffer. A byte
    // allocation does not promise alignment for VS_FIXEDFILEINFO.
    let info = unsafe { value.cast::<VS_FIXEDFILEINFO>().read_unaligned() };
    (info.dwSignature == VS_FFI_SIGNATURE as u32).then(|| {
        let major = info.dwFileVersionMS >> 16;
        let minor = info.dwFileVersionMS & 0xFFFF;
        let build = info.dwFileVersionLS >> 16;
        let revision = info.dwFileVersionLS & 0xFFFF;
        format!("{major}.{minor}.{build}.{revision}")
    })
}

/// Returns a fixed signer category only after WinVerifyTrust accepts the file.
/// Paths are used as in-memory cache keys and never leave the collector.
fn verified_signer(path: &str) -> String {
    if !is_local_drive_path(path) {
        return "unknown".into();
    }
    let cache = SIGNER_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock() {
        if let Some(signer) = cache.get(path) {
            return signer.clone();
        }
    }
    let signer = verify_signer_uncached(path);
    if let Ok(mut cache) = cache.lock() {
        if cache.len() < MAX_SIGNER_CACHE_ENTRIES {
            cache.entry(path.to_owned()).or_insert_with(|| signer.clone()).clone()
        } else {
            signer
        }
    } else {
        signer
    }
}

fn is_local_drive_path(path: &str) -> bool {
    let path = path.strip_prefix("\\\\?\\").unwrap_or(path);
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/')
}

fn verify_signer_uncached(path: &str) -> String {
    let wide_path = path.encode_utf16().chain([0]).collect::<Vec<_>>();
    let mut file_info = WINTRUST_FILE_INFO::default();
    file_info.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
    file_info.pcwszFilePath = wide_path.as_ptr();
    let mut trust_data = WINTRUST_DATA::default();
    trust_data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
    trust_data.dwUIChoice = WTD_UI_NONE;
    trust_data.fdwRevocationChecks = WTD_REVOKE_NONE;
    trust_data.dwUnionChoice = WTD_CHOICE_FILE;
    trust_data.Anonymous.pFile = &mut file_info;
    trust_data.dwStateAction = WTD_STATEACTION_VERIFY;
    // Do not cause an environment snapshot to make network requests.
    trust_data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: The file path and WINTRUST structures remain alive for the call,
    // the union matches WTD_CHOICE_FILE, and UI/network retrieval are disabled.
    let trusted = unsafe { WinVerifyTrust(std::ptr::null_mut(), &mut action, (&mut trust_data as *mut WINTRUST_DATA).cast()) == 0 };
    trust_data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: This closes any state created by the preceding verification call;
    // all data and file-path pointers remain valid through cleanup.
    unsafe { WinVerifyTrust(std::ptr::null_mut(), &mut action, (&mut trust_data as *mut WINTRUST_DATA).cast()) };
    if !trusted {
        return "unknown".into();
    }
    let Some(subject) = embedded_signer_subject(&wide_path) else {
        return "unknown".into();
    };
    classify_verified_subject(&subject).into()
}

fn embedded_signer_subject(wide_path: &[u16]) -> Option<String> {
    let mut encoding = 0u32;
    let mut content = 0u32;
    let mut format = 0u32;
    let mut store = std::ptr::null_mut();
    let mut message = std::ptr::null_mut();
    // SAFETY: The path is nul-terminated UTF-16, outputs are valid writable
    // locals, and the requested content is limited to embedded Authenticode.
    let queried = unsafe {
        CryptQueryObject(
            CERT_QUERY_OBJECT_FILE,
            wide_path.as_ptr().cast(),
            CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            &mut encoding,
            &mut content,
            &mut format,
            &mut store,
            &mut message,
            std::ptr::null_mut(),
        ) != 0
    };
    let subject = if queried && !store.is_null() && !message.is_null() {
        signer_subject_from_message(store, message, encoding)
    } else {
        None
    };
    if !message.is_null() {
        // SAFETY: CryptQueryObject returned this message handle; it has not
        // been closed or otherwise transferred.
        unsafe { CryptMsgClose(message) };
    }
    if !store.is_null() {
        // SAFETY: CryptQueryObject returned this store handle; no contexts
        // created from it remain live after signer_subject_from_message returns.
        unsafe { CertCloseStore(store, 0) };
    }
    subject
}

fn signer_subject_from_message(
    store: windows_sys::Win32::Security::Cryptography::HCERTSTORE,
    message: *mut std::ffi::c_void,
    encoding: u32,
) -> Option<String> {
    let mut size = 0u32;
    // SAFETY: The message is live; a null data pointer with a valid size output
    // asks CryptoAPI for the required signer-info buffer length. This sizing
    // call may return false with ERROR_MORE_DATA, which is expected here.
    let _ = unsafe { CryptMsgGetParam(message, CMSG_SIGNER_INFO_PARAM, 0, std::ptr::null_mut(), &mut size) };
    if size == 0 || size > MAX_SIGNER_INFO {
        return None;
    }
    let words = (size as usize).div_ceil(std::mem::size_of::<usize>());
    let mut buffer = vec![0usize; words];
    // SAFETY: `buffer` is suitably aligned and has at least `size` writable
    // bytes; the message remains live while CryptMsgGetParam fills it.
    if unsafe { CryptMsgGetParam(message, CMSG_SIGNER_INFO_PARAM, 0, buffer.as_mut_ptr().cast(), &mut size) } == 0
        || size < std::mem::size_of::<CMSG_SIGNER_INFO>() as u32
    {
        return None;
    }
    // SAFETY: The successful API call filled at least a complete signer-info
    // structure into an aligned buffer, whose backing storage remains alive.
    let signer = unsafe { &*buffer.as_ptr().cast::<CMSG_SIGNER_INFO>() };
    let mut identity = CERT_INFO::default();
    identity.Issuer = signer.Issuer;
    identity.SerialNumber = signer.SerialNumber;
    // SAFETY: `store` is the live certificate store returned with this message;
    // the find parameter has the issuer and serial number required by this flag.
    let certificate = unsafe {
        CertFindCertificateInStore(
            store,
            encoding | X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SUBJECT_CERT,
            (&identity as *const CERT_INFO).cast(),
            std::ptr::null(),
        )
    };
    if certificate.is_null() {
        return None;
    }
    // SAFETY: `certificate` is a live context returned by CertFindCertificateInStore;
    // the null output pointer requests the name length.
    let name_len = unsafe {
        CertGetNameStringW(
            certificate,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    };
    let subject = if name_len > 1 && name_len <= MAX_SIGNER_NAME_CHARS {
        let mut name = vec![0u16; name_len as usize];
        // SAFETY: The certificate context is live and `name` provides the
        // writable capacity returned by the preceding length query.
        let actual = unsafe {
            CertGetNameStringW(
                certificate,
                CERT_NAME_SIMPLE_DISPLAY_TYPE,
                0,
                std::ptr::null(),
                name.as_mut_ptr(),
                name_len,
            )
        };
        (actual > 1).then(|| wide_string(&name))
    } else {
        None
    };
    // SAFETY: The context was returned by CertFindCertificateInStore and has
    // not previously been freed.
    unsafe { CertFreeCertificateContext(certificate) };
    subject
}

fn classify_verified_subject(subject: &str) -> &'static str {
    match subject.trim().to_ascii_lowercase().as_str() {
        "microsoft corporation" | "microsoft windows" | "microsoft windows publisher" => "Microsoft",
        "nvidia corporation" | "advanced micro devices, inc." | "amd inc." | "intel corporation" => "GPU vendor",
        _ => "other",
    }
}

fn power() -> (Option<String>, Option<String>, bool) {
    // SAFETY: The API writes one fixed-size status structure into a valid,
    // initialized local buffer and retains no pointer to it.
    let mut status = SYSTEM_POWER_STATUS::default();
    let source = if unsafe { GetSystemPowerStatus(&mut status) } != 0 {
        match status.ACLineStatus {
            0 if status.BatteryFlag != 0x80 && status.BatteryFlag != 0xFF => Some("battery".to_owned()),
            0 => None,
            1 => Some("AC".to_owned()),
            _ => None,
        }
    } else {
        None
    };

    // SAFETY: `scheme` is a writable pointer; on success the API allocates
    // the returned GUID using LocalAlloc, which is released below.
    let mut scheme = std::ptr::null_mut();
    let result = unsafe { PowerGetActiveScheme(std::ptr::null_mut(), &mut scheme) };
    let profile = if !scheme.is_null() {
        let name = if result == 0 {
            // SAFETY: successful PowerGetActiveScheme returned a valid GUID.
            let guid = unsafe { *scheme };
            let guid = format!(
                "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                guid.data1,
                guid.data2,
                guid.data3,
                guid.data4[0],
                guid.data4[1],
                guid.data4[2],
                guid.data4[3],
                guid.data4[4],
                guid.data4[5],
                guid.data4[6],
                guid.data4[7],
            );
            Some(super::environment::power_profile_from_guid(&guid).to_owned())
        } else {
            None
        };
        // SAFETY: This pointer came from PowerGetActiveScheme and has not
        // been freed or otherwise exposed.
        unsafe { LocalFree(scheme.cast()) };
        name
    } else {
        None
    };
    let collected = source.is_some() || profile.is_some();
    (profile, source, collected)
}

fn current_modules() -> Option<Vec<Module>> {
    // SAFETY: GetCurrentProcessId has no pointer arguments or preconditions.
    let pid = unsafe { GetCurrentProcessId() };
    // SAFETY: The snapshot flags are valid, and the current PID is supplied;
    // the returned handle is checked against INVALID_HANDLE_VALUE below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entries = BTreeMap::new();
    let mut entry = MODULEENTRY32W::default();
    entry.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
    // SAFETY: `snapshot` is a valid module snapshot and `entry` is initialized
    // with the required structure size.
    let mut ok = unsafe { Module32FirstW(snapshot, &mut entry) } != 0;
    while ok && entries.len() < MAX_MODULES {
        let name = wide_basename(&entry.szModule);
        if let Some(name) = safe_name(&name) {
            // The full path is used transiently for classification only; the
            // serialized record contains the validated basename above.
            let path = wide_string(&entry.szExePath);
            let version = file_version(&path);
            let signer = verified_signer(&path);
            let class = classify_module_with_signer(&name, &path, &signer);
            // Keep distinct files with the same basename as distinct records;
            // the full path is only a temporary sort/dedup key.
            entries.entry((name.clone(), path)).or_insert(Module {
                name,
                version,
                signer,
                class: class.into(),
            });
        }
        // SAFETY: `snapshot` remains open and `entry` retains the required size.
        ok = unsafe { Module32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: `snapshot` is the valid handle returned by CreateToolhelp32Snapshot.
    unsafe { CloseHandle(snapshot) };
    Some(entries.into_values().collect())
}

fn overlay_processes() -> Option<Vec<String>> {
    // SAFETY: The process snapshot flag is valid; PID zero requests the system
    // process list. The returned handle is validated below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut names = Vec::new();
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    // SAFETY: `snapshot` is an open process snapshot and `entry` has its size set.
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while ok && names.len() < MAX_PROCESSES {
        names.push(wide_string(&entry.szExeFile).to_ascii_lowercase());
        // SAFETY: `snapshot` remains open and `entry` retains the required size.
        ok = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: `snapshot` is the valid handle returned by CreateToolhelp32Snapshot.
    unsafe { CloseHandle(snapshot) };
    if names.len() >= MAX_PROCESSES {
        return None;
    }
    Some(
        names
            .iter()
            .filter_map(|name| known_overlay_process(name))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
}

fn displays() -> Option<Vec<Display>> {
    let mut displays = Vec::new();
    let hdr = active_hdr_flags();
    let mut index = 0;
    loop {
        let mut device = DISPLAY_DEVICEW::default();
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // SAFETY: A null device name requests enumeration by index; `device`
        // is writable and its required size is initialized above.
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) } == 0 {
            break;
        }
        index += 1;
        if device.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        let mut mode = DEVMODEW::default();
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        // SAFETY: DeviceName is a nul-terminated buffer provided by Windows,
        // and `mode` is writable with its required size initialized.
        if unsafe { EnumDisplaySettingsW(device.DeviceName.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) } == 0 {
            continue;
        }
        displays.push(Display {
            width: mode.dmPelsWidth,
            height: mode.dmPelsHeight,
            refresh_hz: (mode.dmDisplayFrequency > 1).then_some(mode.dmDisplayFrequency),
            hdr: hdr
                .as_ref()
                .and_then(|values| values.get(&wide_string(&device.DeviceName).to_ascii_lowercase()).copied().flatten()),
            // The generic Windows display APIs expose refresh and advanced
            // color, but not whether variable refresh is currently active.
            vrr: None,
        });
    }
    (!displays.is_empty()).then_some(displays)
}

/// Matches active target color state to its GDI source. Cloned targets are
/// ambiguous and remain unknown rather than choosing a monitor by ordinal.
fn active_hdr_flags() -> Option<BTreeMap<String, Option<bool>>> {
    let mut path_count = 0u32;
    let mut mode_count = 0u32;
    // SAFETY: both count outputs are valid writable pointers, and the flags
    // request the currently active topology only.
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) } != 0
        || path_count > 32
        || mode_count > 128
    {
        return None;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: the vectors have the capacities returned by the preceding size
    // query, and the output counters are writable. The topology output must be
    // null with QDC_ONLY_ACTIVE_PATHS.
    if unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    } != 0
    {
        return None;
    }
    paths.truncate(path_count as usize);
    let mut flags = BTreeMap::new();
    for path in paths {
        let target = path.targetInfo;
        let hdr = target_hdr(target.adapterId, target.id);
        let source = path.sourceInfo;
        let mut source_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source_name.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source_name.header.adapterId = source.adapterId;
        source_name.header.id = source.id;
        // SAFETY: The header identifies an active source returned by
        // QueryDisplayConfig, and the output packet has its required size.
        if unsafe { DisplayConfigGetDeviceInfo(&mut source_name.header) } == 0 {
            let name = wide_string(&source_name.viewGdiDeviceName).to_ascii_lowercase();
            if flags.insert(name.clone(), hdr).is_some() {
                // A cloned source can drive multiple physical targets; do
                // not attribute one target's HDR state to the whole source.
                flags.insert(name, None);
            }
        }
    }
    Some(flags)
}

// windows-sys 0.61 lacks this Windows SDK packet. Layout/type/mode values:
// microsoft/win32metadata generation/WinSDK/RecompiledIdlHeaders/um/wingdi.h,
// DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2. It separates WCG from actual HDR.
#[repr(C)]
#[derive(Default)]
struct AdvancedColorInfo2 {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    flags: u32,
    color_encoding: i32,
    bits_per_channel: u32,
    active_color_mode: i32,
}

fn target_hdr(adapter: windows_sys::Win32::Foundation::LUID, id: u32) -> Option<bool> {
    let mut info = AdvancedColorInfo2::default();
    info.header.r#type = 15; // DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2
    info.header.size = std::mem::size_of::<AdvancedColorInfo2>() as u32;
    info.header.adapterId = adapter;
    info.header.id = id;
    // SAFETY: repr(C) matches the SDK packet, the header identifies an active
    // target, and the advertised size covers the entire writable packet.
    if unsafe { DisplayConfigGetDeviceInfo(&mut info.header) } == 0 {
        return hdr_from_color_mode(info.active_color_mode);
    }
    let mut old = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();
    old.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
    old.header.size = std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
    old.header.adapterId = adapter;
    old.header.id = id;
    // SAFETY: The SDK packet is writable and initialized for this target.
    if unsafe { DisplayConfigGetDeviceInfo(&mut old.header) } == 0 {
        // SAFETY: The API initialized the union on success. Bit 1 indicates
        // some advanced color, which can include SDR auto color management.
        let bits = unsafe { old.Anonymous.value };
        return hdr_from_legacy_color_bits(bits);
    }
    None
}

fn hdr_from_color_mode(mode: i32) -> Option<bool> {
    match mode {
        0 | 1 => Some(false), // SDR or WCG
        2 => Some(true),      // HDR
        _ => None,
    }
}

fn hdr_from_legacy_color_bits(bits: u32) -> Option<bool> {
    // Old APIs cannot distinguish HDR from SDR with advanced color enabled.
    (bits & 0b10 == 0).then_some(false)
}

fn wide_string(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|unit| *unit == 0).unwrap_or(value.len())])
}

fn wide_basename(value: &[u16]) -> String {
    let name = wide_string(value);
    name.rsplit(['\\', '/']).next().unwrap_or_default().to_owned()
}

fn safe_name(name: &str) -> Option<String> {
    (!name.is_empty() && name.len() <= 128 && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte)))
        .then(|| name.to_owned())
}

fn classify_module(name: &str, path: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let path = path.to_ascii_lowercase();
    if lower.starts_with("libstd-") || lower.starts_with("libcore-") || lower.starts_with("rustc_") {
        "Rust standard library"
    } else if lower == "aurora-viewer.exe" {
        "Aurora"
    } else if ["\\nvidia\\", "\\amd\\", "\\ati\\", "\\intel\\", "\\igfx\\"]
        .iter()
        .any(|token| path.contains(token))
        || ["nvwgf", "nvld", "nvspcap", "atidxx", "amdxx", "igd", "igc"]
            .iter()
            .any(|token| lower.starts_with(token))
    {
        "GPU vendor"
    } else if [
        "\\windows\\system32\\",
        "\\windows\\syswow64\\",
        "\\windows\\winsxs\\",
        "\\systemroot\\system32\\",
    ]
    .iter()
    .any(|directory| path.contains(directory))
    {
        "system"
    } else {
        "third-party"
    }
}

fn classify_module_with_signer(name: &str, path: &str, signer: &str) -> &'static str {
    super::environment::module_class_for_signer(signer, classify_module(name, path))
}

fn known_overlay_process(name: &str) -> Option<&'static str> {
    OVERLAY_PROCESSES
        .iter()
        .find(|(_, aliases)| aliases.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(label, _)| *label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advanced_color_does_not_imply_hdr() {
        assert_eq!(hdr_from_color_mode(0), Some(false));
        assert_eq!(hdr_from_color_mode(1), Some(false));
        assert_eq!(hdr_from_color_mode(2), Some(true));
        assert_eq!(hdr_from_color_mode(99), None);
        assert_eq!(hdr_from_legacy_color_bits(0), Some(false));
        assert_eq!(hdr_from_legacy_color_bits(0b11), None);
        assert_eq!(hdr_from_legacy_color_bits(0b111), None);
    }

    #[test]
    fn advanced_color_packet_matches_windows_sdk_layout() {
        assert_eq!(std::mem::size_of::<AdvancedColorInfo2>(), 36);
        assert_eq!(std::mem::offset_of!(AdvancedColorInfo2, active_color_mode), 32);
    }

    #[test]
    fn module_name_filter_keeps_only_a_basename_token() {
        assert_eq!(safe_name("example.dll"), Some("example.dll".into()));
        for unsafe_name in ["", "C:\\Users\\name\\mod.dll", "module name.dll", "x\n.dll"] {
            assert_eq!(safe_name(unsafe_name), None);
        }
        assert_eq!(
            wide_basename(&"C:\\private\\hook.dll\0".encode_utf16().collect::<Vec<_>>()),
            "hook.dll"
        );
    }

    #[test]
    fn module_classifier_uses_a_fixed_label_vocabulary() {
        assert_eq!(classify_module("DiscordHook64.dll", "C:\\Apps\\DiscordHook64.dll"), "third-party");
        assert_eq!(
            classify_module_with_signer("dxgi.dll", "C:\\Aurora-Viewer\\dxgi.dll", "unknown"),
            "third-party"
        );
        assert_eq!(
            classify_module("nvwgf2umx.dll", "C:\\Windows\\System32\\nvwgf2umx.dll"),
            "GPU vendor"
        );
        assert_eq!(classify_module("kernel32.dll", "C:\\Windows\\System32\\kernel32.dll"), "system");
        assert_eq!(
            classify_module("Aurora-Viewer.exe", "C:\\Aurora-Viewer\\Aurora-Viewer.exe"),
            "Aurora"
        );
        assert_eq!(
            classify_module("libstd-abc.dll", "C:\\Aurora-Viewer\\libstd-abc.dll"),
            "Rust standard library"
        );
    }

    #[test]
    fn verified_signers_override_conflicting_module_path_classes() {
        assert_eq!(
            classify_module_with_signer("plugin.dll", "C:\\NVIDIA\\plugin.dll", "other"),
            "third-party"
        );
        assert_eq!(
            classify_module_with_signer("plugin.dll", "C:\\Apps\\plugin.dll", "GPU vendor"),
            "GPU vendor"
        );
        assert_eq!(
            classify_module_with_signer("kernel32.dll", "C:\\Windows\\System32\\kernel32.dll", "Microsoft"),
            "Microsoft"
        );
    }

    #[test]
    fn overlay_allowlist_matches_executable_basenames_only() {
        assert_eq!(known_overlay_process("rtss.exe"), Some("RivaTuner"));
        assert_eq!(known_overlay_process("BANDICAM.exe"), Some("Bandicam"));
        assert_eq!(known_overlay_process("wallpaper64.exe"), Some("Wallpaper Engine"));
        assert_eq!(known_overlay_process("AMDRSServ.exe"), Some("AMD overlay"));
        assert_eq!(known_overlay_process("C:\\RTSS\\rtss.exe"), None);
    }

    #[test]
    fn verified_signers_use_a_fixed_publisher_vocabulary() {
        assert_eq!(classify_verified_subject("Microsoft Corporation"), "Microsoft");
        assert_eq!(classify_verified_subject("NVIDIA Corporation"), "GPU vendor");
        assert_eq!(classify_verified_subject("Advanced Micro Devices, Inc."), "GPU vendor");
        assert_eq!(classify_verified_subject("Unknown Publisher LLC"), "other");
    }

    #[test]
    fn file_metadata_reads_are_limited_to_drive_paths() {
        assert!(is_local_drive_path("C:\\Windows\\System32\\kernel32.dll"));
        assert!(is_local_drive_path("\\\\?\\C:\\Windows\\System32\\kernel32.dll"));
        assert!(!is_local_drive_path("\\\\server\\share\\module.dll"));
        assert!(!is_local_drive_path("\\Device\\HarddiskVolume1\\module.dll"));
    }
}
