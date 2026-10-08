//! Teleport history for the navigation bar's back / forward buttons
//! (behaviour of LLTeleportHistory: arrivals after a teleport are recorded,
//! going back or forward moves within the list without truncating it).

use aurora_net::RegionHandle;
use glam::Vec3;

const MAX_ITEMS: usize = 100;

#[derive(Debug, Clone)]
pub struct TpEntry {
    pub region: String,
    pub handle: RegionHandle,
    /// Region-local position.
    pub position: Vec3,
}

#[derive(Debug, Default)]
pub struct TeleportHistory {
    items: Vec<TpEntry>,
    current: usize,
    /// Index being travelled to with back / forward.
    pending: Option<usize>,
}

impl TeleportHistory {
    /// Arrival at a new location (login or teleport).
    pub fn arrived(&mut self, entry: TpEntry) {
        if let Some(i) = self.pending.take()
            && i < self.items.len()
        {
            self.current = i;
            self.items[i] = entry;
            return;
        }
        if !self.items.is_empty() {
            self.items.truncate(self.current + 1);
        }
        self.items.push(entry);
        if self.items.len() > MAX_ITEMS {
            self.items.remove(0);
        }
        self.current = self.items.len() - 1;
    }

    /// The teleport did not happen.
    pub fn failed(&mut self) {
        self.pending = None;
    }

    /// Update the current entry's region name once it is known.
    pub fn name_current(&mut self, handle: RegionHandle, name: &str) {
        if let Some(e) = self.items.get_mut(self.current)
            && e.handle == handle
            && e.region.is_empty()
        {
            e.region = name.to_owned();
        }
    }

    pub fn previous(&self) -> Option<&TpEntry> {
        self.current.checked_sub(1).and_then(|i| self.items.get(i))
    }

    pub fn next(&self) -> Option<&TpEntry> {
        self.items.get(self.current + 1)
    }

    /// Start travelling back; returns the destination.
    pub fn go_back(&mut self) -> Option<TpEntry> {
        let i = self.current.checked_sub(1)?;
        self.pending = Some(i);
        self.items.get(i).cloned()
    }

    pub fn go_forward(&mut self) -> Option<TpEntry> {
        let i = self.current + 1;
        let e = self.items.get(i).cloned()?;
        self.pending = Some(i);
        Some(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(n: &str) -> TpEntry {
        TpEntry {
            region: n.into(),
            handle: 0,
            position: Vec3::ZERO,
        }
    }

    #[test]
    fn back_forward_and_new_branch() {
        let mut h = TeleportHistory::default();
        h.arrived(e("A"));
        h.arrived(e("B"));
        h.arrived(e("C"));
        assert_eq!(h.go_back().map(|x| x.region), Some("B".into()));
        h.arrived(e("B"));
        assert_eq!(h.previous().map(|x| x.region.clone()), Some("A".into()));
        assert_eq!(h.next().map(|x| x.region.clone()), Some("C".into()));
        // a normal teleport from B drops the forward part
        h.arrived(e("D"));
        assert!(h.next().is_none());
        assert_eq!(h.previous().map(|x| x.region.clone()), Some("B".into()));
        // a failed trip keeps the position
        h.go_back();
        h.failed();
        h.arrived(e("E"));
        assert_eq!(h.previous().map(|x| x.region.clone()), Some("D".into()));
    }
}
