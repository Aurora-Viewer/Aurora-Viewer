//! Per-account list of regions to watch (Legacy Viewer Region Tracker).

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE: &str = "tracked_regions.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedRegion {
    pub name: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegionTracker {
    regions: Vec<TrackedRegion>,
}

impl RegionTracker {
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else { return Self::default() };
        // Accept the Legacy Viewer `{name: {label}}` shape.
        let parsed: Result<std::collections::BTreeMap<String, LegacyEntry>, _> = serde_json::from_slice(&bytes);
        let Ok(entries) = parsed else { return Self::default() };
        Self {
            regions: entries
                .into_iter()
                .map(|(name, entry)| TrackedRegion { name, label: entry.label })
                .collect(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let entries: std::collections::BTreeMap<_, _> = self
            .regions
            .iter()
            .map(|r| (r.name.clone(), LegacyEntry { label: r.label.clone() }))
            .collect();
        let bytes = serde_json::to_vec_pretty(&entries).map_err(std::io::Error::other)?;
        crate::atomic_file::write(path, &bytes)
    }

    pub fn regions(&self) -> &[TrackedRegion] {
        &self.regions
    }

    pub fn add(&mut self, name: &str, label: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        if let Some(existing) = self.regions.iter_mut().find(|r| r.name.eq_ignore_ascii_case(name)) {
            existing.label = label.trim().to_owned();
            return true;
        }
        self.regions.push(TrackedRegion {
            name: name.to_owned(),
            label: label.trim().to_owned(),
        });
        true
    }

    pub fn remove(&mut self, name: &str) {
        self.regions.retain(|r| !r.name.eq_ignore_ascii_case(name));
    }
}

#[derive(Deserialize, Serialize)]
struct LegacyEntry {
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionState {
    Unknown,
    Online,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionStatus {
    pub state: RegionState,
    pub agents: Option<u32>,
    pub access: Option<u8>,
}

pub fn status(map: &super::worldmap::WorldMap, name: &str, own_region: Option<(u32, u32)>) -> RegionStatus {
    let Some(key) = map.region_by_name(name) else {
        return RegionStatus {
            state: RegionState::Unknown,
            agents: None,
            access: None,
        };
    };
    let Some(sim) = map.sims.get(&key) else {
        return RegionStatus {
            state: RegionState::Unknown,
            agents: None,
            access: None,
        };
    };
    if sim.is_down() {
        return RegionStatus {
            state: RegionState::Down,
            agents: Some(0),
            access: Some(sim.access),
        };
    }
    let own = own_region == Some(key);
    let count = map
        .agents
        .get(&key)
        .map(|v| v.iter().map(|i| i.2.max(0) as u32).sum::<u32>() + u32::from(own))
        .or_else(|| own.then_some(1));
    RegionStatus {
        state: RegionState::Online,
        agents: count,
        access: Some(sim.access),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::worldmap::{SIM_ACCESS_DOWN, SIM_ACCESS_PG, SimInfo};
    use uuid::Uuid;
    #[test]
    fn add_updates_case_insensitive_name_and_remove_works() {
        let mut list = RegionTracker::default();
        assert!(list.add(" Ahern ", "home"));
        assert!(list.add("ahern", "shop"));
        assert_eq!(list.regions().len(), 1);
        assert_eq!(list.regions()[0].label, "shop");
        list.remove("AHERN");
        assert!(list.regions().is_empty());
    }

    #[test]
    fn legacy_json_list_round_trips() {
        let path = std::env::temp_dir().join(format!("aurora-region-tracker-{}.json", uuid::Uuid::new_v4()));
        let mut list = RegionTracker::default();
        list.add("Ahern", "home");
        list.save(&path).expect("save tracker");
        assert_eq!(RegionTracker::load(&path), list);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn status_distinguishes_unknown_online_and_down_and_adds_our_avatar() {
        let mut map = super::super::worldmap::WorldMap::default();
        let key = (10, 20);
        map.sims.insert(
            key,
            SimInfo {
                name: "Ahern".into(),
                access: SIM_ACCESS_PG,
                flags: 0,
                image: Uuid::nil(),
                size_x: 256,
                size_y: 256,
            },
        );
        map.agents.insert(key, vec![(2560, 5120, 4)]);
        assert_eq!(status(&map, "missing", None).state, RegionState::Unknown);
        assert_eq!(status(&map, "ahern", None).agents, Some(4));
        assert_eq!(status(&map, "ahern", Some(key)).agents, Some(5));
        map.agents.remove(&key);
        assert_eq!(status(&map, "ahern", Some(key)).agents, Some(1));
        map.agents.insert(key, vec![(2560, 5120, 4)]);
        map.sims.get_mut(&key).expect("inserted sim").access = SIM_ACCESS_DOWN;
        let down = status(&map, "ahern", None);
        assert_eq!(down.state, RegionState::Down);
        assert_eq!(down.agents, Some(0));
        assert_eq!(down.access, Some(SIM_ACCESS_DOWN));
    }
}
