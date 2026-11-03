//! Is the graphics driver recent enough? Checked once when the renderer
//! starts, offline (no server is asked): the version the Vulkan driver
//! reports and, on Windows, the date and version of the display driver as
//! installed (the display adapter class in the registry). An old driver is
//! the first cause of the crashes and broken images testers report; the
//! login screen then shows a warning with the vendor's download page
//! (`ui/login/driver_notice.rs`). A driver this module cannot read is never
//! called old. The Legacy Viewer's equivalent is the OldGPUDriver
//! notification (LLAppViewer::init, indra/newview/llappviewer.cpp, with the
//! NvidiaDriverPage / AMDDriverPage / IntelDriverPage links): a modal at
//! start, "Yes / No / don't ask again", but its test compares the OpenGL
//! version with an expected one that is always zero, so it never shows.
//! Here the minimums are written below, and the warning is a card of the
//! login screen rather than a modal.

/// NVIDIA driver older than this (major.minor) is out of date: 572.16 is
/// the Game Ready driver of 30 January 2025, the first with the GeForce
/// RTX 5090 / 5080 (https://www.nvidia.com/en-us/drivers/details/240794/).
/// Maxwell and Pascal cards get it too (their last full branch is 580,
/// then security updates). Review at each release.
pub const NVIDIA_MINIMUM: (u32, u32) = (572, 16);
/// Kepler cards (GeForce 600 / 700 series) stopped at the 470 branch; its
/// last security update is 474.89 (2024).
pub const NVIDIA_KEPLER_MINIMUM: (u32, u32) = (474, 89);
/// The same on Linux, whose driver branches are numbered on their own (a
/// Linux 570 is as recent as a Windows 572): 570.86.16, of the same day, is
/// the first Linux driver with the RTX 5090 / 5080
/// (https://www.nvidia.com/en-us/drivers/unix/).
pub const NVIDIA_LINUX_MINIMUM: (u32, u32) = (570, 86);
/// Kepler's last Linux driver: 470.256.02 (2024).
pub const NVIDIA_LINUX_KEPLER_MINIMUM: (u32, u32) = (470, 256);
/// Any driver, any vendor, installed more than this many months ago.
pub const MAX_AGE_MONTHS: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
}

impl Vendor {
    /// PCI vendor ids, as Vulkan reports them.
    pub fn from_id(id: u32) -> Option<Vendor> {
        match id {
            0x10DE => Some(Vendor::Nvidia),
            0x1002 => Some(Vendor::Amd),
            0x8086 => Some(Vendor::Intel),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
        }
    }

    /// The vendor's official driver download page.
    pub fn download_url(self) -> &'static str {
        match self {
            Vendor::Nvidia => "https://www.nvidia.com/en-us/drivers/",
            Vendor::Amd => "https://www.amd.com/en/support/download/drivers.html",
            Vendor::Intel => "https://www.intel.com/content/www/us/en/support/detect.html",
        }
    }
}

/// The display driver as Windows installed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// Year, month, day.
    pub date: (i32, u32, u32),
    /// Windows' four-part version (`32.0.15.8278`).
    pub version: String,
}

/// What the check reads.
#[derive(Clone, Debug)]
pub struct Input {
    pub vendor_id: u32,
    pub gpu: String,
    /// The Vulkan driver's name and information (`NVIDIA 582.78`,
    /// `NVIDIA 570.86.16` on Linux, `AMD proprietary driver 25.20.42.14
    /// (LLPC)`).
    pub driver: String,
    /// The driver is NVIDIA's Linux one, whose versions follow their own
    /// branches (`NVIDIA_LINUX_MINIMUM`).
    pub linux: bool,
    pub installed: Option<Installed>,
    pub today: (i32, u32, u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Older than the vendor's minimum: the version found and the minimum.
    Version { found: String, minimum: String },
    /// Installed more than `MAX_AGE_MONTHS` ago, on that date.
    Age { date: (i32, u32, u32), months: u32 },
}

/// The warning the login screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub vendor: Vendor,
    pub gpu: String,
    pub reason: Reason,
    /// What a dismissal remembers (`Settings::gpu_driver_dismissed`): the
    /// driver itself, so a newer one still too old warns again.
    pub key: String,
}

/// The driver's verdict; `None` when it is recent enough or cannot be read.
pub fn check(input: &Input) -> Option<Notice> {
    let vendor = Vendor::from_id(input.vendor_id)?;
    let notice = |reason| Notice {
        vendor,
        gpu: input.gpu.clone(),
        reason,
        key: input.installed.as_ref().map_or_else(|| input.driver.clone(), |i| i.version.clone()),
    };
    if vendor == Vendor::Nvidia {
        let version = nvidia_version(&input.driver).or_else(|| input.installed.as_ref().and_then(|i| nvidia_from_windows(&i.version)));
        if let Some(found) = version {
            let minimum = match (input.linux, kepler(&input.gpu)) {
                (false, false) => NVIDIA_MINIMUM,
                (false, true) => NVIDIA_KEPLER_MINIMUM,
                (true, false) => NVIDIA_LINUX_MINIMUM,
                (true, true) => NVIDIA_LINUX_KEPLER_MINIMUM,
            };
            if found < minimum {
                return Some(notice(Reason::Version {
                    found: format!("{}.{:02}", found.0, found.1),
                    minimum: format!("{}.{:02}", minimum.0, minimum.1),
                }));
            }
        }
    }
    let date = input.installed.as_ref()?.date;
    let months = months_between(date, input.today)?;
    // A Kepler card's last driver is old whatever the user does.
    (months > MAX_AGE_MONTHS && !(vendor == Vendor::Nvidia && kepler(&input.gpu))).then(|| notice(Reason::Age { date, months }))
}

/// `NVIDIA 582.78` → (582, 78); Linux's third number is left out
/// (`NVIDIA 570.86.16` → (570, 86)).
pub fn nvidia_version(driver: &str) -> Option<(u32, u32)> {
    let rest = driver.trim().strip_prefix("NVIDIA")?.trim();
    let mut parts = rest.split_whitespace().next()?.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    if parts.next().is_some_and(|patch| patch.parse::<u32>().is_err()) || parts.next().is_some() {
        return None;
    }
    Some((major, minor))
}

/// Windows writes NVIDIA's version in its last five digits:
/// `32.0.15.8278` is 582.78.
pub fn nvidia_from_windows(version: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = version.trim().split('.').collect();
    let [.., third, fourth] = parts.as_slice() else { return None };
    if !third.bytes().chain(fourth.bytes()).all(|b| b.is_ascii_digit()) || third.is_empty() || fourth.is_empty() || fourth.len() > 4 {
        return None;
    }
    let digits = format!("{}{fourth:0>4}", &third[third.len() - 1..]);
    let value: u32 = digits.parse().ok()?;
    Some((value / 100, value % 100))
}

/// GeForce 600 / 700 cards on the Kepler chip (the GTX 745 / 750 are
/// Maxwell), and the first GTX TITANs.
fn kepler(gpu: &str) -> bool {
    let gpu = gpu.to_ascii_uppercase();
    if gpu.contains("GTX TITAN") {
        return !["TITAN X", "TITAN V", "TITAN XP"].iter().any(|later| gpu.contains(later));
    }
    let model = gpu
        .split_once("GTX ")
        .or_else(|| gpu.split_once("GT "))
        .map(|(_, rest)| rest.chars().take_while(char::is_ascii_digit).collect::<String>());
    match model.as_deref() {
        Some(m) if m.len() == 3 => (m.starts_with('6') || m.starts_with('7')) && !matches!(m, "745" | "750"),
        _ => false,
    }
}

/// The registry's `DriverDate`: month-day-year (`7-22-2026`).
pub fn parse_driver_date(text: &str) -> Option<(i32, u32, u32)> {
    let mut parts = text.trim().split('-');
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let year: i32 = parts.next()?.parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day) && year >= 1990 && parts.next().is_none()).then_some((year, month, day))
}

/// Whole months from `from` to `to`; `None` when `from` is later.
pub fn months_between(from: (i32, u32, u32), to: (i32, u32, u32)) -> Option<u32> {
    let months = (to.0 - from.0) * 12 + to.1 as i32 - from.1 as i32 - i32::from(to.2 < from.2);
    u32::try_from(months).ok()
}

pub fn today() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = aurora_net::profile::civil_from_days(secs.div_euclid(86_400));
    (y as i32, m, d)
}

/// The installed display driver of the adapter named `gpu`: the display
/// class's entries, matched by their description (the name Vulkan gives).
#[cfg(windows)]
pub fn installed(gpu: &str) -> Option<Installed> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    const CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let value = |key: &str, name: &str| -> Option<String> {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut buffer = [0u16; 512];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: both strings end with a zero and the buffer holds `bytes`
        // writable bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buffer[..end]))
    };
    // The entries are numbered 0000, 0001…; a hole ends nothing (removed
    // adapters leave gaps), the first few dozen are plenty.
    (0..32)
        .filter_map(|i| {
            let key = format!(r"{CLASS}\{i:04}");
            let desc = value(&key, "DriverDesc")?;
            if !desc.trim().eq_ignore_ascii_case(gpu.trim()) {
                return None;
            }
            Some(Installed {
                date: parse_driver_date(&value(&key, "DriverDate")?)?,
                version: value(&key, "DriverVersion")?,
            })
        })
        .max_by_key(|i| i.date)
}

#[cfg(not(windows))]
pub fn installed(_gpu: &str) -> Option<Installed> {
    None
}

/// `AURORA_DEMO_GPU_DRIVER=old|old-linux|aged`: a GeForce GTX 1060 on an
/// NVIDIA driver of 2023 (`old`, under the minimum), the same card on a
/// Linux driver of 2024 (`old-linux`, under the Linux minimum), or an AMD
/// card whose driver was installed three years ago (`aged`).
pub fn demo_input() -> Option<Input> {
    std::env::var_os("AURORA_DEMO")?;
    let today = today();
    let (vendor_id, gpu, driver, installed, linux) = match std::env::var("AURORA_DEMO_GPU_DRIVER").ok()?.as_str() {
        "old" => (
            0x10DE,
            "NVIDIA GeForce GTX 1060 6GB",
            "NVIDIA 546.33",
            Some(Installed {
                date: (2023, 12, 12),
                version: "31.0.15.4633".into(),
            }),
            false,
        ),
        // Linux has no installation date to read
        "old-linux" => (0x10DE, "NVIDIA GeForce GTX 1060 6GB", "NVIDIA 565.57.01", None, true),
        "aged" => (
            0x1002,
            "AMD Radeon RX 6700 XT",
            "AMD proprietary driver 23.10.2",
            Some(Installed {
                date: (today.0 - 3, today.1, 1),
                version: "31.0.22011.4008".into(),
            }),
            false,
        ),
        _ => return None,
    };
    Some(Input {
        vendor_id,
        gpu: gpu.into(),
        driver: driver.into(),
        linux,
        installed,
        today,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(vendor_id: u32, gpu: &str, driver: &str, installed: Option<((i32, u32, u32), &str)>) -> Input {
        Input {
            vendor_id,
            gpu: gpu.into(),
            driver: driver.into(),
            linux: false,
            installed: installed.map(|(date, version)| Installed {
                date,
                version: version.into(),
            }),
            today: (2026, 10, 25),
        }
    }

    #[test]
    fn nvidia_versions_are_read_from_vulkan_and_from_windows() {
        assert_eq!(nvidia_version("NVIDIA 582.78"), Some((582, 78)));
        assert_eq!(nvidia_version("NVIDIA 617.14"), Some((617, 14)));
        assert_eq!(nvidia_version("AMD proprietary driver 25.20.42.14 (LLPC)"), None);
        assert_eq!(nvidia_version("NVIDIA"), None);
        assert_eq!(nvidia_from_windows("32.0.16.1088"), Some((610, 88)));
        assert_eq!(nvidia_from_windows("32.0.15.8278"), Some((582, 78)));
        assert_eq!(nvidia_from_windows("31.0.15.4633"), Some((546, 33)));
        assert_eq!(nvidia_from_windows("30.0.14.7489"), Some((474, 89)));
        assert_eq!(nvidia_from_windows("garbage"), None);
        assert_eq!(nvidia_from_windows("1.2.x.4"), None);
    }

    #[test]
    fn the_drivers_of_the_crash_reports_are_recent_enough() {
        let today = Some(((2025, 11, 20), "32.0.15.8278"));
        assert_eq!(check(&input(0x10DE, "NVIDIA GeForce GTX 1060 6GB", "NVIDIA 582.78", today)), None);
        assert_eq!(check(&input(0x10DE, "NVIDIA GeForce RTX 5070 Ti", "NVIDIA 617.14", None)), None);
        assert_eq!(
            check(&input(
                0x1002,
                "AMD Radeon RX 9070 XT",
                "AMD proprietary driver 25.20.42.14 (LLPC)",
                None
            )),
            None
        );
    }

    #[test]
    fn an_old_nvidia_driver_is_named_with_the_minimum() {
        let notice = check(&input(0x10DE, "NVIDIA GeForce GTX 1060 6GB", "NVIDIA 546.33", None)).expect("old");
        assert_eq!(notice.vendor, Vendor::Nvidia);
        assert_eq!(
            notice.reason,
            Reason::Version {
                found: "546.33".into(),
                minimum: "572.16".into()
            }
        );
        assert_eq!(notice.key, "NVIDIA 546.33");
        // Kepler stopped at 474.89: no newer one exists for it.
        assert_eq!(check(&input(0x10DE, "NVIDIA GeForce GTX 780 Ti", "NVIDIA 474.89", None)), None);
        assert!(check(&input(0x10DE, "NVIDIA GeForce GTX 750 Ti", "NVIDIA 474.89", None)).is_some());
        assert!(check(&input(0x10DE, "NVIDIA GeForce GT 730", "NVIDIA 472.12", None)).is_some());
    }

    #[test]
    fn any_vendor_s_driver_installed_over_two_years_ago_is_old() {
        let aged = check(&input(
            0x1002,
            "AMD Radeon RX 580",
            "AMD proprietary driver 22.5.1",
            Some(((2024, 9, 1), "31.0.12027.9001")),
        ))
        .expect("aged");
        assert_eq!(
            aged.reason,
            Reason::Age {
                date: (2024, 9, 1),
                months: 25
            }
        );
        assert_eq!(aged.key, "31.0.12027.9001");
        let recent = input(
            0x8086,
            "Intel(R) Arc(TM) A770 Graphics",
            "Intel driver",
            Some(((2025, 1, 10), "32.0.101.6299")),
        );
        assert_eq!(check(&recent), None);
        // Unknown vendor or unreadable driver: never a warning.
        assert_eq!(check(&input(0x5143, "Adreno", "x", Some(((2015, 1, 1), "1.0.0.0")))), None);
        assert_eq!(check(&input(0x1002, "AMD Radeon", "AMD proprietary driver", None)), None);
        // A Kepler card's last driver ages without any newer one.
        assert_eq!(
            check(&input(
                0x10DE,
                "NVIDIA GeForce GTX 680",
                "NVIDIA 474.89",
                Some(((2024, 5, 1), "30.0.14.7489"))
            )),
            None
        );
    }

    #[test]
    fn linux_versions_have_three_numbers_or_two() {
        assert_eq!(nvidia_version("NVIDIA 615.78.08"), Some((615, 78)));
        assert_eq!(nvidia_version("NVIDIA 550.54.14"), Some((550, 54)));
        assert_eq!(nvidia_version("NVIDIA 470.256.02"), Some((470, 256)));
        assert_eq!(nvidia_version("NVIDIA 570.144"), Some((570, 144)));
        assert_eq!(nvidia_version("NVIDIA 570.86.x"), None);
        assert_eq!(nvidia_version("NVIDIA 1.2.3.4"), None);
    }

    #[test]
    fn linux_drivers_are_held_to_the_linux_branches() {
        let linux = |gpu: &str, driver: &str| Input {
            linux: true,
            ..input(0x10DE, gpu, driver, None)
        };
        // the first Linux drivers of the RTX 50 are recent enough, though
        // under the Windows number
        assert_eq!(check(&linux("NVIDIA GeForce RTX 5090", "NVIDIA 570.86.16")), None);
        assert_eq!(check(&linux("NVIDIA GeForce RTX 4070", "NVIDIA 570.144")), None);
        assert_eq!(check(&linux("NVIDIA GeForce RTX 4070", "NVIDIA 615.78.08")), None);
        let old = check(&linux("NVIDIA GeForce GTX 1060 6GB", "NVIDIA 565.57.01")).expect("old");
        assert_eq!(
            old.reason,
            Reason::Version {
                found: "565.57".into(),
                minimum: "570.86".into()
            }
        );
        assert_eq!(old.key, "NVIDIA 565.57.01");
        // Kepler's last Linux driver, and one before it
        assert_eq!(check(&linux("NVIDIA GeForce GTX 780 Ti", "NVIDIA 470.256.02")), None);
        assert!(check(&linux("NVIDIA GeForce GTX 680", "NVIDIA 470.199.02")).is_some());
    }

    #[test]
    fn driver_dates_and_months() {
        assert_eq!(parse_driver_date("7-22-2026"), Some((2026, 7, 22)));
        assert_eq!(parse_driver_date("12-1-2023"), Some((2023, 12, 1)));
        for bad in ["2026-07-22", "13-1-2023", "x", "7-22", "7-22-2026-1"] {
            assert_eq!(parse_driver_date(bad), None, "{bad}");
        }
        assert_eq!(months_between((2024, 10, 25), (2026, 10, 25)), Some(24));
        assert_eq!(months_between((2024, 10, 26), (2026, 10, 25)), Some(23));
        assert_eq!(months_between((2027, 1, 1), (2026, 10, 25)), None);
    }
}
