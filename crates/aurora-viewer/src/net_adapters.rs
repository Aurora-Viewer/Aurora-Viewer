//! Network adapters whose driver loses the link under many simultaneous
//! downloads, and the download mode that follows from them.
//!
//! Marvell AQtion (formerly Aquantia) 10 Gbit adapters dropped their
//! connection until a reboot during login or a teleport, while the adaptive
//! asset pace had up to 192 requests under way (#1545, #1546). With such an
//! adapter present the downloads always keep the Legacy Viewer's bounded
//! connections (`aurora_net::AssetConnections::Limited`): Automatic means
//! limited, and "Follow the connection" cannot be picked (a choice saved
//! before stays in the settings but is not applied). Read once at startup,
//! before the network starts.

use crate::settings::DownloadConnections;
use aurora_net::AssetConnections;
use std::sync::OnceLock;

/// Words of an adapter's description that name a driver known to drop.
const DROPS_UNDER_LOAD: &[&str] = &["aqtion", "aquantia"];

/// The first description of an adapter known to drop, if any.
pub fn needs_limited<'a>(descriptions: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    descriptions.into_iter().find(|d| {
        let d = d.to_lowercase();
        DROPS_UNDER_LOAD.iter().any(|w| d.contains(w))
    })
}

/// The download mode for a setting and the adapter found, if any: with
/// such an adapter, always limited, whatever was chosen.
pub fn resolve(choice: DownloadConnections, detected: Option<&str>) -> AssetConnections {
    match choice {
        _ if detected.is_some() => AssetConnections::Limited,
        DownloadConnections::Limited => AssetConnections::Limited,
        DownloadConnections::Automatic | DownloadConnections::Adaptive => AssetConnections::Adaptive,
    }
}

/// The mode the network starts with, telling once in the log when a saved
/// "Follow the connection" is not applied because of the adapter.
pub fn startup_mode(choice: DownloadConnections) -> AssetConnections {
    let detected = detected();
    if choice == DownloadConnections::Adaptive
        && let Some(name) = detected
    {
        log::info!("download connections: \"follow the connection\" is saved, limited instead for {name:?} (#1545)");
    }
    resolve(choice, detected)
}

static DETECTED: OnceLock<Option<String>> = OnceLock::new();

/// The adapter known to drop that this computer has, looked up once.
pub fn detected() -> Option<&'static str> {
    DETECTED
        .get_or_init(|| {
            // `AURORA_DEMO_NET_ADAPTER=<description>`: this adapter instead
            // of the computer's, to see the automatic mode limited
            let names = match std::env::var("AURORA_DEMO_NET_ADAPTER") {
                Ok(name) if !name.trim().is_empty() => vec![name],
                _ => adapter_descriptions(),
            };
            let found = needs_limited(names.iter().map(String::as_str)).map(str::to_owned);
            if let Some(name) = &found {
                log::info!("network adapter {name:?}: downloads limited (#1545)");
            }
            found
        })
        .as_deref()
}

/// The descriptions of the network adapters present (GetAdaptersAddresses).
#[cfg(windows)]
fn adapter_descriptions() -> Vec<String> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GAA_FLAG_SKIP_UNICAST, GetAdaptersAddresses,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::Networking::WinSock::AF_UNSPEC;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    let flags = GAA_FLAG_SKIP_UNICAST | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    // (the list can grow between the two calls: a few tries)
    for _ in 0..3 {
        // u64 words keep the structures aligned
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `buf` holds `size` writable bytes, aligned for the
        // structure; the call writes at most that and says how much it needs.
        let r = unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, Some(first), &mut size) };
        if r == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if r != 0 {
            log::warn!("network adapters: GetAdaptersAddresses failed ({r})");
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut node = first;
        while !node.is_null() {
            // SAFETY: a node of the list the call just wrote into `buf`,
            // which outlives this loop; its strings are NUL-terminated
            // inside the same buffer.
            let a = unsafe { &*node };
            if !a.Description.is_null()
                && let Ok(d) = unsafe { a.Description.to_string() }
            {
                out.push(d);
            }
            node = a.Next;
        }
        return out;
    }
    log::warn!("network adapters: the list kept growing");
    Vec::new()
}

#[cfg(not(windows))]
fn adapter_descriptions() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aqtion_and_aquantia_adapters_are_found_whatever_their_case() {
        let names = ["Intel(R) Ethernet Controller I226-V", "Marvell AQtion 10Gbit Network Adapter"];
        assert_eq!(needs_limited(names), Some("Marvell AQtion 10Gbit Network Adapter"));
        assert!(needs_limited(["Aquantia AQC107 NBase-T/IEEE 802.3bz Ethernet Controller"]).is_some());
        assert!(needs_limited(["MARVELL AQTION 5GBIT"]).is_some());
    }

    #[test]
    fn other_adapters_keep_the_adaptive_pace() {
        let names = [
            "Realtek PCIe 2.5GbE Family Controller",
            "Intel(R) Wi-Fi 6E AX211 160MHz",
            "Hyper-V Virtual Ethernet Adapter",
            "Software Loopback Interface 1",
        ];
        assert_eq!(needs_limited(names), None);
        assert_eq!(needs_limited([]), None);
    }

    #[test]
    fn without_the_adapter_each_choice_keeps_its_mode() {
        use DownloadConnections::*;
        assert_eq!(resolve(Automatic, None), AssetConnections::Adaptive);
        assert_eq!(resolve(Adaptive, None), AssetConnections::Adaptive);
        assert_eq!(resolve(Limited, None), AssetConnections::Limited);
    }

    #[test]
    fn with_the_adapter_every_choice_is_limited() {
        use DownloadConnections::*;
        let aqtion = Some("Marvell AQtion 10Gbit Network Adapter");
        assert_eq!(resolve(Automatic, aqtion), AssetConnections::Limited);
        assert_eq!(resolve(Limited, aqtion), AssetConnections::Limited);
        // "follow the connection" saved before: limited, the setting itself
        // is left as it was
        assert_eq!(resolve(Adaptive, aqtion), AssetConnections::Limited);
    }
}
