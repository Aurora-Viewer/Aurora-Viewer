//! The device identifiers sent at login (`mac`, `id0`), computed the way
//! Linden Lab's viewer computes them, so that Second Life sees the same values
//! as with the official viewer (Third Party Viewer Policy: the usage
//! information is neither falsified nor circumvented, machine bans hold).
//!
//! - `mac` is llHashedUniqueID (llhasheduniqueid.cpp:33): the md5 hex of the
//!   six bytes of LLMachineID (llmachineid.cpp:399), else of the network
//!   adapter address (LLUUID::getNodeID, lluuid.cpp:456 / 600), else 32 zeros.
//!   On Windows LLMachineID folds the characters of the motherboard UUID
//!   (Win32_ComputerSystemProduct), else of the Windows serial number
//!   (Win32_OperatingSystem), into six bytes. Elsewhere it is the adapter
//!   address, all zeros when none is found.
//! - `id0` is generateSerialNumber: on Windows the md5 hex of the C:\ volume
//!   serial (llappviewerwin32.cpp:1597), empty when it cannot be read; on
//!   Linux the md5 hex of the longest name in /dev/disk/by-uuid
//!   (llappviewerlinux.cpp:556).
//!
//! Only the hashes leave this module, plus the raw adapter address the
//! viewer statistics carry (`adapter_mac`, send_viewer_stats'
//! `mac_address`), and the identifiers are never logged (the Legacy Viewer
//! logs the raw serial; Aurora logs the source only).

use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};

/// The `mac` and `id0` login parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginIds {
    pub mac: String,
    pub id0: String,
}

static IDS: OnceLock<LoginIds> = OnceLock::new();
/// The raw adapter address of LLUUID::getNodeID, for the viewer statistics'
/// `mac_address` (send_viewer_stats reads it there, not the login hash).
static ADAPTER: OnceLock<Option<[u8; 6]>> = OnceLock::new();
static START: Once = Once::new();

/// How long a login waits for the identifiers (WMI can be slow on a cold
/// start); past it the login goes with the values of "unavailable".
const WAIT: Duration = Duration::from_secs(10);

/// Computes the identifiers once, on their own thread (WMI runs COM in its
/// own apartment there). Called at startup, so they are ready at login.
pub fn start() {
    START.call_once(|| {
        let spawned = std::thread::Builder::new().name("machine-id".into()).spawn(|| {
            let (ids, mac_source, id0_source) = compute();
            log::info!("login device ids computed: mac from {mac_source}, id0 from {id0_source}");
            let _ = IDS.set(ids);
            let _ = ADAPTER.set(os_adapter_address());
        });
        if let Err(e) = spawned {
            log::warn!("login device ids: thread not started: {e}");
        }
    });
}

/// The identifiers for a login, waiting up to [`WAIT`] for [`start`]'s
/// thread.
pub fn login_ids() -> LoginIds {
    start();
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(ids) = IDS.get() {
            return ids.clone();
        }
        if Instant::now() >= deadline {
            log::warn!("login device ids not ready after {} s: sent as unavailable", WAIT.as_secs());
            return unavailable();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The network adapter address LLUUID::getNodeID picks (the raw bytes, for
/// the viewer statistics only: never logged, never hashed into anything
/// else); None until [`start`]'s thread has read it, or when there is none.
pub(crate) fn adapter_mac() -> Option<[u8; 6]> {
    start();
    ADAPTER.get().copied().flatten()
}

/// The login identifiers if [`start`]'s thread has computed them, without
/// waiting (the viewer statistics read them every second).
pub(crate) fn ready_login_ids() -> Option<LoginIds> {
    start();
    IDS.get().cloned()
}

#[cfg(windows)]
fn os_adapter_address() -> Option<[u8; 6]> {
    windows_ids::adapter_address()
}

#[cfg(not(windows))]
fn os_adapter_address() -> Option<[u8; 6]> {
    unix_ids::adapter_address()
}

/// What the Legacy Viewer sends when nothing can be read.
fn unavailable() -> LoginIds {
    LoginIds {
        mac: mac_hash(None),
        id0: if cfg!(windows) { id0_windows(None) } else { id0_linux(&[]) },
    }
}

/// md5 hex, lower case (LLMD5::hex_digest, `%02x`).
fn md5_hex(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}

/// llHashedUniqueID: the md5 of the six bytes, or 32 zeros without any.
fn mac_hash(unique: Option<[u8; 6]>) -> String {
    match unique {
        Some(bytes) => md5_hex(&bytes),
        None => "0".repeat(32),
    }
}

/// LLWMIMethods::getGenericSerialNumber's folding: the UTF-16 units of the
/// serial, up to the first NUL, added in turn to the six bytes, each sum
/// kept modulo 256 (the store into an unsigned char).
fn fold_serial(units: &[u16]) -> [u8; 6] {
    let mut id = [0u8; 6];
    let mut j = 0;
    while j < units.len() && units[j] != 0 {
        for byte in &mut id {
            if j >= units.len() || units[j] == 0 {
                break;
            }
            *byte = byte.wrapping_add(units[j] as u8);
            j += 1;
        }
    }
    id
}

/// LLUUID::validate (lluuid.cpp:308) and the null / all-F refusal of
/// getGenericSerialNumber for the motherboard UUID: 36 characters (or 35,
/// the old format without the fourth dash), 32 hex digits at the fixed
/// positions. The dashes themselves are not checked, as there.
fn usable_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    let broken = match bytes.len() {
        36 => false,
        35 => true,
        _ => return false,
    };
    let mut digits = Vec::with_capacity(32);
    let mut pos = 0usize;
    for i in 0..16 {
        if matches!(i, 4 | 6 | 8 | 10) {
            pos += 1;
            if broken && i == 10 {
                pos -= 1;
            }
        }
        for _ in 0..2 {
            let Some(d) = (bytes[pos] as char).to_digit(16) else {
                return false;
            };
            digits.push(d);
            pos += 1;
        }
    }
    let null = digits.iter().all(|&d| d == 0);
    let all_f = digits.iter().all(|&d| d == 0xf);
    !null && !all_f
}

/// LLMachineID::init on Windows, then llHashedUniqueID's fallback: the
/// motherboard UUID, else the Windows serial, else the adapter address.
fn pick_windows(product: Option<[u8; 6]>, legacy: Option<[u8; 6]>, node: Option<[u8; 6]>) -> (Option<[u8; 6]>, &'static str) {
    if let Some(id) = product {
        (Some(id), "the motherboard UUID")
    } else if let Some(id) = legacy {
        (Some(id), "the Windows serial number")
    } else if let Some(id) = node {
        (Some(id), "the network adapter")
    } else {
        (None, "nothing (unavailable)")
    }
}

/// generateSerialNumber on Windows: md5 of the four bytes of the volume
/// serial (a DWORD in memory, little endian), empty when it is unknown.
fn id0_windows(serial: Option<u32>) -> String {
    serial.map_or_else(String::new, |s| md5_hex(&s.to_le_bytes()))
}

/// generateSerialNumber on Linux: md5 of the longest name of
/// /dev/disk/by-uuid, the alphabetically last of equal lengths (the md5 of
/// nothing when the folder is empty).
fn id0_linux(names: &[String]) -> String {
    let mut best = "";
    for name in names {
        if name.len() > best.len() || (name.len() == best.len() && name.as_str() > best) {
            best = name;
        }
    }
    md5_hex(best.as_bytes())
}

#[cfg(windows)]
fn compute() -> (LoginIds, &'static str, &'static str) {
    let (product, legacy) = match windows_ids::Wmi::connect() {
        Some(wmi) => (
            wmi.serial("SELECT * FROM Win32_ComputerSystemProduct", windows::core::w!("UUID"), true)
                .map(|u| fold_serial(&u)),
            wmi.serial("SELECT * FROM Win32_OperatingSystem", windows::core::w!("SerialNumber"), false)
                .map(|u| fold_serial(&u)),
        ),
        None => (None, None),
    };
    // getNodeID only runs when LLMachineID found nothing
    let node = if product.is_none() && legacy.is_none() {
        windows_ids::adapter_address()
    } else {
        None
    };
    let (unique, mac_source) = pick_windows(product, legacy, node);
    let serial = windows_ids::volume_serial();
    let id0_source = if serial.is_some() {
        "the C: volume serial"
    } else {
        "nothing (unavailable)"
    };
    (
        LoginIds {
            mac: mac_hash(unique),
            id0: id0_windows(serial),
        },
        mac_source,
        id0_source,
    )
}

#[cfg(not(windows))]
fn compute() -> (LoginIds, &'static str, &'static str) {
    // LLMachineID::init (#else): the adapter address, zeros without one
    let node = unix_ids::adapter_address();
    let mac_source = if node.is_some() { "the network adapter" } else { "nothing (zeros)" };
    let names = unix_ids::disk_uuids();
    let id0_source = if names.is_empty() {
        "nothing (empty disk list)"
    } else {
        "the disk UUIDs"
    };
    (
        LoginIds {
            mac: mac_hash(Some(node.unwrap_or([0; 6]))),
            id0: id0_linux(&names),
        },
        mac_source,
        id0_source,
    )
}

#[cfg(windows)]
mod windows_ids {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize, EOAC_NONE,
        RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VariantClear};
    use windows::Win32::System::Wmi::{
        IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE,
        WBEM_INFINITE, WbemLocator,
    };
    use windows::core::{BSTR, PCWSTR};

    /// RPC_C_AUTHN_WINNT / RPC_C_AUTHZ_NONE (rpcdce.h), as LLWMIMethods sets
    /// them on the proxy.
    const AUTHN_WINNT: u32 = 10;
    const AUTHZ_NONE: u32 = 0;

    /// The ROOT\CIMV2 connection of LLWMIMethods::initCOMObjects. Unlike it,
    /// the process-wide CoInitializeSecurity is not called: the proxy
    /// blanket is enough for these reads, and other COM users of the viewer
    /// keep the default security.
    pub(super) struct Wmi {
        services: Option<IWbemServices>,
        uninit: bool,
    }

    impl Wmi {
        pub(super) fn connect() -> Option<Self> {
            // SAFETY: COM calls on this thread only; every interface is
            // released before CoUninitialize (Drop).
            unsafe {
                let uninit = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
                let mut wmi = Wmi { services: None, uninit };
                let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
                let empty = BSTR::new();
                let services = locator
                    .ConnectServer(&BSTR::from("ROOT\\CIMV2"), &empty, &empty, &empty, 0, &empty, None)
                    .ok()?;
                CoSetProxyBlanket(
                    &services,
                    AUTHN_WINNT,
                    AUTHZ_NONE,
                    PCWSTR::null(),
                    RPC_C_AUTHN_LEVEL_CALL,
                    RPC_C_IMP_LEVEL_IMPERSONATE,
                    None,
                    EOAC_NONE,
                )
                .ok()?;
                wmi.services = Some(services);
                Some(wmi)
            }
        }

        /// getGenericSerialNumber: the property of the first object whose
        /// value is a non-empty string (and a usable UUID when asked).
        pub(super) fn serial(&self, query: &str, property: PCWSTR, as_uuid: bool) -> Option<Vec<u16>> {
            let services = self.services.as_ref()?;
            let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
            // SAFETY: WMI calls on the thread that opened the connection;
            // the VARIANT is cleared after its string is copied.
            unsafe {
                let objects = services.ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), flags, None).ok()?;
                loop {
                    let mut row: [Option<IWbemClassObject>; 1] = [None];
                    let mut returned = 0u32;
                    let _ = objects.Next(WBEM_INFINITE, &mut row, &mut returned);
                    if returned == 0 {
                        return None;
                    }
                    let object = row[0].take()?;
                    let mut value = VARIANT::default();
                    if object.Get(property, 0, &mut value, None, None).is_err() {
                        continue;
                    }
                    let inner = &value.Anonymous.Anonymous;
                    let units: Vec<u16> = if inner.vt == VT_BSTR {
                        inner.Anonymous.bstrVal.to_vec()
                    } else {
                        Vec::new()
                    };
                    let _ = VariantClear(&mut value);
                    if units.is_empty() {
                        continue;
                    }
                    if as_uuid && !super::usable_uuid(&String::from_utf16_lossy(&units)) {
                        continue;
                    }
                    return Some(units);
                }
            }
        }
    }

    impl Drop for Wmi {
        fn drop(&mut self) {
            self.services = None;
            if self.uninit {
                // SAFETY: matches the successful CoInitializeEx of connect.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// LLUUID::getNodeID on Windows: the NetBIOS adapter status of each
    /// LAN adapter, the last one that answers.
    pub(super) fn adapter_address() -> Option<[u8; 6]> {
        use windows_sys::Win32::NetworkManagement::NetBios::{
            ADAPTER_STATUS, LANA_ENUM, NAME_BUFFER, NCB, NCBASTAT, NCBENUM, NCBRESET, Netbios,
        };

        #[repr(C)]
        struct AdapterStatus {
            adapt: ADAPTER_STATUS,
            names: [NAME_BUFFER; 30],
        }

        // SAFETY: each NCB points at a buffer of the size it declares, alive
        // for the synchronous Netbios call.
        unsafe {
            let mut lanas: LANA_ENUM = std::mem::zeroed();
            let mut ncb: NCB = std::mem::zeroed();
            ncb.ncb_command = NCBENUM as u8;
            ncb.ncb_buffer = (&raw mut lanas).cast();
            ncb.ncb_length = size_of::<LANA_ENUM>() as u16;
            Netbios(&mut ncb);
            let mut found = None;
            for &lana in &lanas.lana[..usize::from(lanas.length).min(lanas.lana.len())] {
                let mut ncb: NCB = std::mem::zeroed();
                ncb.ncb_command = NCBRESET as u8;
                ncb.ncb_lana_num = lana;
                Netbios(&mut ncb);

                let mut status: AdapterStatus = std::mem::zeroed();
                let mut ncb: NCB = std::mem::zeroed();
                ncb.ncb_command = NCBASTAT as u8;
                ncb.ncb_lana_num = lana;
                ncb.ncb_callname = *b"*              \0";
                ncb.ncb_buffer = (&raw mut status).cast();
                ncb.ncb_length = size_of::<AdapterStatus>() as u16;
                if Netbios(&mut ncb) == 0 {
                    found = Some(status.adapt.adapter_address);
                }
            }
            found
        }
    }

    /// GetVolumeInformation("C:\\")'s serial number.
    pub(super) fn volume_serial() -> Option<u32> {
        use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;
        let root: Vec<u16> = "C:\\\0".encode_utf16().collect();
        let mut serial = 0u32;
        let mut flags = 0u32;
        // SAFETY: a NUL-terminated root path; the null buffers are declared
        // with a zero size.
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut serial,
                std::ptr::null_mut(),
                &mut flags,
                std::ptr::null_mut(),
                0,
            )
        };
        (ok != 0).then_some(serial)
    }
}

#[cfg(not(windows))]
mod unix_ids {
    /// LLUUID::getNodeID (lluuid.cpp:600): the first interface of
    /// SIOCGIFCONF whose hardware address (SIOCGIFHWADDR) is not all zeros.
    #[cfg(target_os = "linux")]
    pub(super) fn adapter_address() -> Option<[u8; 6]> {
        /// struct ifreq on Linux: the name, then a 24-byte union whose
        /// hardware address is a sockaddr (family, then the bytes).
        #[repr(C)]
        struct IfReq {
            name: [libc::c_char; 16],
            data: [u8; 24],
        }
        #[repr(C)]
        struct IfConf {
            len: libc::c_int,
            buf: *mut libc::c_char,
        }

        // SAFETY: the ioctls write into buffers of the declared sizes; the
        // socket is closed on every path.
        unsafe {
            let sd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_IP);
            if sd < 0 {
                return None;
            }
            let mut buf = [0 as libc::c_char; 1024];
            let mut ifc = IfConf {
                len: buf.len() as libc::c_int,
                buf: buf.as_mut_ptr(),
            };
            if libc::ioctl(sd, libc::SIOCGIFCONF, &mut ifc) < 0 {
                libc::close(sd);
                return None;
            }
            let step = size_of::<IfReq>();
            let mut found = None;
            let mut offset = 0usize;
            while offset + step <= ifc.len as usize {
                let listed = buf.as_ptr().add(offset).cast::<IfReq>().read_unaligned();
                offset += step;
                let mut req = IfReq {
                    name: listed.name,
                    data: [0; 24],
                };
                if libc::ioctl(sd, libc::SIOCGIFHWADDR, &mut req) < 0 {
                    continue;
                }
                let mut mac = [0u8; 6];
                mac.copy_from_slice(&req.data[2..8]);
                if mac.iter().any(|&b| b != 0) {
                    found = Some(mac);
                    break;
                }
            }
            libc::close(sd);
            found
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn adapter_address() -> Option<[u8; 6]> {
        None
    }

    /// The names of /dev/disk/by-uuid.
    pub(super) fn disk_uuids() -> Vec<String> {
        std::fs::read_dir("/dev/disk/by-uuid/")
            .map(|dir| dir.filter_map(|entry| entry.ok()?.file_name().into_string().ok()).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn a_serial_folds_into_six_bytes_modulo_256() {
        // "ABCDEFGH": A..F in the six bytes, then G and H added to the first two
        assert_eq!(
            fold_serial(&wide("ABCDEFGH")),
            [b'A'.wrapping_add(b'G'), b'B'.wrapping_add(b'H'), b'C', b'D', b'E', b'F']
        );
        assert_eq!(fold_serial(&wide("AB")), [b'A', b'B', 0, 0, 0, 0]);
        // stops at the first NUL, and a unit above 255 keeps its low byte
        assert_eq!(fold_serial(&[0x41, 0, 0x42]), [0x41, 0, 0, 0, 0, 0]);
        assert_eq!(fold_serial(&[0x1ff]), [0xff, 0, 0, 0, 0, 0]);
        assert_eq!(fold_serial(&[]), [0; 6]);
    }

    #[test]
    fn the_mac_parameter_is_the_md5_hex_of_six_bytes_or_zeros() {
        assert_eq!(mac_hash(Some([0; 6])), format!("{:x}", md5::compute([0u8; 6])));
        let hash = mac_hash(Some([1, 2, 3, 4, 5, 6]));
        assert_eq!(hash.len(), 32);
        assert!(
            hash.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "lower-case hex like LLMD5::hex_digest"
        );
        assert_eq!(mac_hash(None), "00000000000000000000000000000000");
    }

    #[test]
    fn the_motherboard_uuid_is_kept_only_when_it_identifies_the_machine() {
        assert!(usable_uuid("4C4C4544-0042-3510-8051-B2C04F503232"));
        assert!(usable_uuid("4c4c4544-0042-3510-8051-b2c04f503232"));
        // the old format without the fourth dash
        assert!(usable_uuid("4C4C4544-0042-3510-8051B2C04F503232"));
        assert!(!usable_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(!usable_uuid("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"));
        assert!(!usable_uuid("To be filled by O.E.M."));
        assert!(!usable_uuid("4C4C4544-0042-3510-8051-B2C04F50323Z"));
        assert!(!usable_uuid(""));
    }

    #[test]
    fn windows_takes_the_motherboard_then_the_os_serial_then_the_adapter() {
        let (a, b, c) = ([1; 6], [2; 6], [3; 6]);
        assert_eq!(pick_windows(Some(a), Some(b), Some(c)).0, Some(a));
        assert_eq!(pick_windows(None, Some(b), Some(c)).0, Some(b));
        assert_eq!(pick_windows(None, None, Some(c)).0, Some(c));
        assert_eq!(pick_windows(None, None, None).0, None);
    }

    #[test]
    fn id0_hashes_the_volume_serial_or_the_longest_disk_uuid() {
        let serial = 0x1234_abcd_u32;
        assert_eq!(id0_windows(Some(serial)), format!("{:x}", md5::compute([0xcd, 0xab, 0x34, 0x12])));
        assert_eq!(id0_windows(None), "", "GetVolumeInformation failed: an empty id0");
        let names = [
            "1234-ABCD",
            "0f3e5c2a-8d1b-4c6e-9a7f-2b4d6e8f0a1c",
            "0f3e5c2a-8d1b-4c6e-9a7f-2b4d6e8f0a1b",
            "ABCD",
        ]
        .map(String::from);
        assert_eq!(
            id0_linux(&names),
            format!("{:x}", md5::compute("0f3e5c2a-8d1b-4c6e-9a7f-2b4d6e8f0a1c"))
        );
        assert_eq!(id0_linux(&[]), format!("{:x}", md5::compute("")));
    }

    #[test]
    fn nothing_readable_gives_the_legacy_placeholders() {
        let ids = unavailable();
        assert_eq!(ids.mac, "00000000000000000000000000000000");
        if cfg!(windows) {
            assert_eq!(ids.id0, "");
        }
    }
}
