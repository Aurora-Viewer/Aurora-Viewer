//! Per-account list of regions to watch (ALFloaterRegionTracker,
//! indra/newview/alfloaterregiontracker.cpp): `tracked_regions.json` in the
//! account's folder, the Legacy Viewer's `{name: {label}}` map written as
//! JSON (it writes LLSD notation under the same name, so the two files are
//! not interchangeable), and the state of each region read from the world
//! map.

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
    /// The saved list; empty when there is none yet or it cannot be read.
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else { return Self::default() };
        let parsed: Result<std::collections::BTreeMap<String, LegacyEntry>, _> = serde_json::from_slice(&bytes);
        let entries = match parsed {
            Ok(entries) => entries,
            Err(error) => {
                log::warn!("region tracker: {} unreadable ({error})", path.display());
                return Self::default();
            }
        };
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
    /// An entry without a label keeps its region.
    #[serde(default)]
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
    fn missing_or_unreadable_file_is_an_empty_list_and_a_label_is_optional() {
        let dir = std::env::temp_dir().join(format!("aurora-region-tracker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        assert!(RegionTracker::load(&dir.join(FILE)).regions().is_empty());
        let path = dir.join(FILE);
        std::fs::write(&path, b"not json").expect("write");
        assert!(RegionTracker::load(&path).regions().is_empty());
        std::fs::write(&path, br#"{"Bravo": {"label": "shop"}, "Alpha": {}}"#).expect("write");
        let list = RegionTracker::load(&path);
        assert_eq!(
            list.regions(),
            [
                TrackedRegion {
                    name: "Alpha".into(),
                    label: String::new()
                },
                TrackedRegion {
                    name: "Bravo".into(),
                    label: "shop".into()
                },
            ]
        );
        // only the names and labels are written
        list.save(&path).expect("save");
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        assert_eq!(saved, serde_json::json!({"Alpha": {"label": ""}, "Bravo": {"label": "shop"}}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn refresh_asks_each_region_by_name_and_its_avatars_once_a_minute() {
        use aurora_net::NetCommand;
        let mut w = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.region_tracker.add("Ahern", "");
        w.region_tracker.add("Nowhere", "");
        w.map.sims.insert(
            (1000, 1000),
            SimInfo {
                name: "Ahern".into(),
                access: SIM_ACCESS_PG,
                flags: 0,
                image: Uuid::nil(),
                size_x: 512,
                size_y: 512,
            },
        );
        w.refresh_tracked_regions();
        let sent = w.map.take_commands();
        let names: Vec<_> = sent
            .iter()
            .filter_map(|c| match c {
                NetCommand::MapNameRequest { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["ahern", "nowhere"]);
        assert_eq!(
            sent.iter()
                .filter(|c| matches!(c, NetCommand::MapItemRequest { item_type, handle }
                    if *item_type == aurora_net::map_item::AGENT_LOCATIONS
                        && *handle == aurora_net::origin_to_handle(1000 * 256, 1000 * 256)))
                .count(),
            1
        );
        w.refresh_tracked_regions();
        assert!(w.map.take_commands().is_empty());
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
