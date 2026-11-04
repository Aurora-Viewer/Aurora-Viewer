//! The graphics card as every program on the PC uses it, not only this one.
//!
//! The driver's memory budget (aurora_render::vram) is the part of the card
//! the system lets this process use; it knows nothing of what the other
//! programs are about to do, and says nothing of the card's time. A tester's
//! crash with another viewer open (and a second one beside the Legacy Viewer) lost the
//! device at 2.9 GB used of a 7.3 GB budget, with no timeout in Windows'
//! event log: the card was shared and busy, and the budget alone never
//! showed it. Windows counts, per adapter, the memory used
//! (`GPU Adapter Memory`), the number Task Manager shows. A thread reads it
//! every [`PERIOD`] (through PDH, by the counter's English name whatever the
//! system's language) for the card this viewer draws on: when every program
//! holds together more than the card keeps, the viewer's textures give back
//! its part of the excess ([`oversubscribed`], `scene/textures.rs`).
//!
//! The Legacy Viewer reads neither. Windows only: elsewhere nothing is read
//! and the viewer goes on with the driver's budget alone.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::Duration;

/// Time between two readings.
#[cfg(windows)]
const PERIOD: Duration = Duration::from_secs(2);
/// Share of the card's memory left to the system (the compositor, the
/// programs that ask for a little more): what the viewers together may not
/// take.
const SYSTEM_SHARE: u64 = 10;
/// What the counters said at the last reading.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load {
    /// Bytes of the card's own memory every program holds, this one
    /// included; None until the first reading or without counters.
    pub adapter_used: Option<u64>,
    /// Counts the readings (and the loss of the counters): a figure derived
    /// from one reading is applied once, not at every pass of the caller.
    pub seq: u64,
}

#[derive(Default)]
struct Shared {
    /// `u64::MAX` before the first reading.
    adapter_used: AtomicU64,
    /// Readings made so far, stored last.
    seq: AtomicU64,
}

/// The reader of the counters, started once the card is known.
pub struct GpuLoad {
    shared: Arc<Shared>,
    started: bool,
}

impl GpuLoad {
    pub fn new() -> Self {
        let shared = Shared::default();
        shared.adapter_used.store(u64::MAX, Ordering::Relaxed);
        GpuLoad {
            shared: Arc::new(shared),
            started: false,
        }
    }

    /// Start reading for the card `luid` names (aurora_render::vram, `card`);
    /// once.
    pub fn start(&mut self, luid: u64) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        #[cfg(windows)]
        {
            let weak = Arc::downgrade(&self.shared);
            let spawned = std::thread::Builder::new()
                .name("gpu-load".into())
                .spawn(move || reader::run(luid, weak));
            if let Err(e) = spawned {
                log::warn!("GPU load: the reader could not start: {e}");
            }
        }
        #[cfg(not(windows))]
        let _ = luid;
    }

    pub fn get(&self) -> Load {
        let seq = self.shared.seq.load(Ordering::Acquire);
        let used = self.shared.adapter_used.load(Ordering::Relaxed);
        Load {
            adapter_used: (used != u64::MAX).then_some(used),
            seq,
        }
    }
}

/// The name Windows gives the card in its counters' instances.
pub fn luid_text(luid: u64) -> String {
    format!("luid_0x{:08x}_0x{:08x}", luid >> 32, luid & 0xffff_ffff)
}

/// Bytes of dedicated memory in use on the card `luid` (`GPU Adapter Memory`
/// instances are named `luid_0x<high>_0x<low>_phys_<n>`).
pub fn adapter_used(memory: &[(String, f64)], luid: &str) -> Option<u64> {
    memory
        .iter()
        .filter(|(name, _)| name.to_ascii_lowercase().starts_with(luid))
        .map(|(_, bytes)| bytes.max(0.0) as u64)
        .max()
}

/// This viewer's part of the memory by which the programs together hold more
/// of the card than it can keep without the system moving memory to system
/// RAM (the card less the system's share); 0 while they fit. The driver's
/// budget does not say it: with a viewer open beside this one it reported
/// 9.3 GB for this process with 8.8 GB already in use on a 10 GB card.
/// Each program owes the excess in proportion to what it holds (`own` of
/// `adapter_used`): asked for all of it, this viewer's textures fell to the
/// minimum for eight minutes while the other program, holding four times
/// as much, went on filling the card.
pub fn oversubscribed(total: u64, adapter_used: u64, own: u64) -> u64 {
    let excess = adapter_used.saturating_sub(total - total / SYSTEM_SHARE);
    (excess as u128 * own.min(adapter_used) as u128 / adapter_used.max(1) as u128) as u64
}

#[cfg(windows)]
mod reader {
    use super::*;
    use std::sync::Weak;
    use windows_sys::Win32::System::Performance::{
        PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    };

    pub(super) fn run(luid: u64, shared: Weak<Shared>) {
        let luid = luid_text(luid);
        let (mut first, mut lost) = (true, false);
        loop {
            // (the instances of a wildcard are listed when the counter is
            // added: a new query each time sees the programs started since)
            let Some(shared) = Weak::upgrade(&shared) else { return };
            match sample() {
                Some(memory) => {
                    let used = adapter_used(&memory, &luid);
                    shared.adapter_used.store(used.unwrap_or(u64::MAX), Ordering::Relaxed);
                    shared.seq.fetch_add(1, Ordering::Release);
                    if first || lost {
                        log::info!("GPU load: {} MB in use on the card by every program", used.unwrap_or(0) >> 20);
                    }
                    (first, lost) = (false, false);
                }
                None => {
                    // (stale figures would hold the window at 10 fps, or the
                    // textures down, for the whole session: forgotten, and
                    // tried again)
                    shared.adapter_used.store(u64::MAX, Ordering::Relaxed);
                    shared.seq.fetch_add(1, Ordering::Release);
                    if !lost {
                        log::info!("GPU load: the system's GPU counters could not be read, trying again");
                    }
                    lost = true;
                }
            }
            drop(shared);
            std::thread::sleep(PERIOD);
        }
    }

    /// `GPU Adapter Memory` dedicated usage per adapter instance.
    fn sample() -> Option<Vec<(String, f64)>> {
        let mut query: PDH_HQUERY = std::ptr::null_mut();
        // SAFETY: the out pointer is valid.
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) } != 0 {
            return None;
        }
        let result = (|| {
            let memory = add(query, "\\GPU Adapter Memory(*)\\Dedicated Usage")?;
            // SAFETY: the query is open.
            if unsafe { PdhCollectQueryData(query) } != 0 {
                return None;
            }
            std::thread::sleep(Duration::from_secs(1));
            // SAFETY: as above.
            if unsafe { PdhCollectQueryData(query) } != 0 {
                return None;
            }
            Some(read(memory))
        })();
        // SAFETY: the query is open and not used after this.
        unsafe { PdhCloseQuery(query) };
        result
    }

    fn add(query: PDH_HQUERY, path: &str) -> Option<PDH_HCOUNTER> {
        let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let mut counter: PDH_HCOUNTER = std::ptr::null_mut();
        // SAFETY: the path ends with a zero, the out pointer is valid.
        (unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) } == 0).then_some(counter)
    }

    /// The values of every instance of a wildcard counter, by name.
    fn read(counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
        let (mut size, mut count) = (0u32, 0u32);
        // SAFETY: a null buffer asks for the size.
        if unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut()) } != PDH_MORE_DATA {
            return Vec::new();
        }
        let item = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        // SAFETY: the structure is plain data, all zeroes is a valid value.
        let mut buffer: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = vec![unsafe { std::mem::zeroed() }; (size as usize).div_ceil(item) + 1];
        // SAFETY: the buffer is sized from the size reported above and
        // aligned for the items.
        if unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, buffer.as_mut_ptr()) } != 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count as usize);
        for it in buffer.iter().take(count as usize) {
            if it.szName.is_null() || !matches!(it.FmtValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA) {
                continue;
            }
            // SAFETY: only the `count` items written are read; each name is
            // a zero-ended string inside the buffer, and the value was asked
            // as a double.
            let (name, value) = unsafe {
                let len = (0..).take_while(|&i| *it.szName.add(i) != 0).count();
                (
                    String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len)),
                    it.FmtValue.Anonymous.doubleValue,
                )
            };
            out.push((name, value));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn named(items: &[(&str, f64)]) -> Vec<(String, f64)> {
        items.iter().map(|(n, v)| (n.to_string(), *v)).collect()
    }

    #[test]
    fn the_card_is_named_like_windows_names_it() {
        assert_eq!(luid_text(0x0000_0000_0001_095e), "luid_0x00000000_0x0001095e");
        assert_eq!(luid_text(0x0000_0001_dead_beef), "luid_0x00000001_0xdeadbeef");
    }

    #[test]
    fn the_memory_of_the_card_is_the_one_its_name_gives() {
        let luid = "luid_0x00000000_0x0001095e";
        let found = named(&[
            ("luid_0x00000000_0x0001095E_phys_0", 6.0 * GB as f64),
            ("luid_0x00000000_0x0000AAAA_phys_0", 1.0 * GB as f64),
        ]);
        assert_eq!(adapter_used(&found, luid), Some(6 * GB));
        assert_eq!(adapter_used(&found, "luid_0x00000000_0x0000bbbb"), None);
    }

    #[test]
    fn each_program_gives_back_its_part_of_what_is_past_the_card_less_the_system() {
        // 10 GB card, 1 GB for the system
        assert_eq!(oversubscribed(10 * GB, 8 * GB, 2 * GB), 0);
        assert_eq!(oversubscribed(10 * GB, 9 * GB, 2 * GB), 0);
        // 2 GB of the 10 GB in use: a fifth of the 1 GB excess
        assert_eq!(oversubscribed(10 * GB, 10 * GB, 2 * GB), GB / 5);
        // the whole card in use by this viewer: all of it
        assert_eq!(oversubscribed(10 * GB, 12 * GB, 12 * GB), 3 * GB);
        // counters behind this viewer's figure, nothing in use
        assert_eq!(oversubscribed(10 * GB, 11 * GB, 20 * GB), 2 * GB);
        assert_eq!(oversubscribed(10 * GB, 0, 0), 0);
    }
}
