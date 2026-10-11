//! Aurora's per-HUD visual zoom. Simulator transforms remain untouched.
//! Inventory name + creator identify replacement copies; the file is per account.
use super::World;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use uuid::Uuid;

pub const FILE: &str = "hud_scales.json";
pub const MIN: f32 = 0.1;
pub const MAX: f32 = 8.0;

pub fn wheel_scale(scale: f32, steps: f32, fine: bool) -> f32 {
    if !steps.is_finite() || !scale.is_finite() {
        return scale;
    }
    (scale * (if fine { 1.02_f32 } else { 1.1_f32 }).powf(steps.clamp(-100.0, 100.0))).clamp(MIN, MAX)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    name: String,
    creator: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Profile {
    identity: Identity,
    scale: f32,
}

#[derive(Default)]
struct Local {
    scale: f32,
    identity: Option<Identity>,
    edited: bool,
    offset: Vec3,
    requested_item: bool,
    needs_clamp: bool,
}

#[derive(Default)]
pub struct HudScales {
    profiles: Vec<Profile>,
    local: HashMap<Uuid, Local>,
    writer: Option<Sender<Vec<Profile>>>,
    worker: Option<JoinHandle<()>>,
    viewport: Option<[f32; 2]>,
}

impl Drop for HudScales {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl HudScales {
    pub fn request_item(&mut self, id: Uuid) -> bool {
        let Some(s) = self.local.get_mut(&id) else { return false };
        let requested = s.requested_item;
        s.requested_item = true;
        !requested
    }
    pub fn load(path: PathBuf) -> Self {
        let profiles: Vec<Profile> = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(profiles) => profiles,
                Err(e) => {
                    log::warn!("cannot read HUD scales: {e}");
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                log::warn!("cannot read HUD scales: {e}");
                Vec::new()
            }
        };
        let profiles = profiles
            .into_iter()
            .filter(|p| !p.identity.name.is_empty() && !p.identity.creator.is_nil() && p.scale.is_finite())
            .map(|mut p| {
                p.scale = p.scale.clamp(MIN, MAX);
                p
            })
            .collect();
        let (tx, rx) = mpsc::channel::<Vec<Profile>>();
        let worker = std::thread::Builder::new().name("aurora-hud-scales".into()).spawn(move || {
            while let Ok(mut profiles) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    profiles = newer;
                }
                let save = || -> Result<(), Box<dyn std::error::Error>> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let tmp = path.with_extension("json.tmp");
                    std::fs::write(&tmp, serde_json::to_vec_pretty(&profiles)?)?;
                    std::fs::rename(tmp, &path)?;
                    Ok(())
                };
                if let Err(e) = save() {
                    log::warn!("cannot save HUD scales: {e}");
                }
            }
        });
        match worker {
            Ok(worker) => Self {
                profiles,
                writer: Some(tx),
                worker: Some(worker),
                local: HashMap::new(),
                viewport: None,
            },
            Err(e) => {
                log::warn!("cannot start HUD scale writer: {e}");
                Self {
                    profiles,
                    local: HashMap::new(),
                    writer: None,
                    worker: None,
                    viewport: None,
                }
            }
        }
    }

    fn save(&self) {
        if let Some(writer) = &self.writer {
            let _ = writer.send(self.profiles.clone());
        }
    }

    fn remember(&mut self, identity: Identity, scale: f32) {
        if let Some(p) = self.profiles.iter_mut().find(|p| p.identity == identity) {
            p.scale = scale;
        } else {
            self.profiles.push(Profile { identity, scale });
        }
        self.save();
    }

    pub fn scale(&self, id: Uuid) -> f32 {
        self.local.get(&id).map_or(1.0, |s| s.scale)
    }

    pub fn offset(&self, id: Uuid) -> Vec3 {
        self.local.get(&id).map_or(Vec3::ZERO, |s| s.offset)
    }

    pub fn set_offset(&mut self, id: Uuid, offset: Vec3) {
        if let Some(s) = self.local.get_mut(&id) {
            s.offset = offset;
        }
    }

    pub fn prepare_clamp(&mut self, viewport: [f32; 2]) {
        if self.viewport != Some(viewport) {
            self.viewport = Some(viewport);
            for s in self.local.values_mut() {
                s.needs_clamp = true;
            }
        }
    }

    pub fn needs_clamp(&self, id: Uuid) -> bool {
        self.local.get(&id).is_some_and(|s| s.needs_clamp && s.scale != 1.0)
    }

    pub fn clamped(&mut self, id: Uuid) {
        if let Some(s) = self.local.get_mut(&id) {
            s.needs_clamp = false;
        }
    }

    pub fn set(&mut self, id: Uuid, scale: f32) {
        if !scale.is_finite() {
            return;
        }
        let s = self.local.entry(id).or_insert_with(|| Local {
            scale: 1.0,
            ..Default::default()
        });
        s.scale = scale.clamp(MIN, MAX);
        s.needs_clamp = true;
        s.edited = true;
        if let Some(identity) = s.identity.clone() {
            self.remember(identity, scale.clamp(MIN, MAX));
        }
    }

    pub fn has_profile(&self, id: Uuid) -> bool {
        self.local
            .get(&id)
            .and_then(|s| s.identity.as_ref())
            .is_some_and(|i| self.profiles.iter().any(|p| &p.identity == i))
    }

    /// The current copy keeps its zoom; subsequent copies start at 100%.
    pub fn forget(&mut self, id: Uuid) {
        if let Some(s) = self.local.get_mut(&id) {
            s.edited = false;
            if let Some(identity) = &s.identity {
                self.profiles.retain(|p| &p.identity != identity);
                self.save();
            }
        }
    }

    fn refresh(&mut self, roots: &[(Uuid, Option<Identity>)]) {
        self.local.retain(|id, _| roots.iter().any(|(root, _)| root == id));
        for (id, identity) in roots {
            let s = self.local.entry(*id).or_insert_with(|| Local {
                scale: 1.0,
                ..Default::default()
            });
            if s.identity == *identity {
                continue;
            }
            s.identity = identity.clone();
            if let Some(identity) = identity {
                if s.edited {
                    let scale = s.scale;
                    self.remember(identity.clone(), scale);
                } else {
                    s.scale = self
                        .profiles
                        .iter()
                        .rev()
                        .find(|p| &p.identity == identity)
                        .map_or(1.0, |p| p.scale);
                    s.needs_clamp = true;
                }
            }
        }
    }
}

impl World {
    /// Only attachments of the local avatar can carry a local zoom.
    pub fn hud_root(&self, mut idx: usize) -> Option<usize> {
        for _ in 0..16 {
            let o = self.objects.get(idx)?;
            let parent = self.objects.parent_of(o)?;
            let p = self.objects.get(parent)?;
            if p.is_avatar() {
                return (p.full_id == self.agent_id && (31..=38).contains(&o.attachment_point())).then_some(idx);
            }
            idx = parent;
        }
        None
    }

    pub fn hud_scale(&self, idx: usize) -> f32 {
        if self.hud_scales.local.is_empty() {
            return 1.0;
        }
        self.hud_root(idx)
            .and_then(|root| self.objects.get(root))
            .map_or(1.0, |o| self.hud_scales.scale(o.full_id))
    }

    pub fn visual_scale(&self, idx: usize) -> Vec3 {
        self.objects.get(idx).map_or(Vec3::ONE, |o| o.scale * self.hud_scale(idx))
    }

    /// Root translation stays unchanged; linked offsets follow the zoom.
    pub fn hud_position_scale(&self, idx: usize) -> f32 {
        self.hud_root(idx).filter(|root| *root != idx).map_or(1.0, |_| self.hud_scale(idx))
    }

    pub fn dirty_hud(&mut self, root: usize) {
        for idx in crate::build::family(self, root) {
            self.objects.touch(idx);
        }
    }

    pub fn refresh_hud_scales(&mut self) {
        let roots: Vec<_> = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|idx| self.objects.get(idx))
            .map(|me| {
                self.objects
                    .children_of(&me.key)
                    .iter()
                    .filter_map(|idx| {
                        let o = self.objects.get(*idx)?;
                        if !(31..=38).contains(&o.attachment_point()) {
                            return None;
                        }
                        let identity = o
                            .attachment_item_id()
                            .and_then(|id| self.inventory.items.get(&id))
                            .filter(|item| !item.creator.is_nil() && !item.name.trim().is_empty())
                            .map(|item| Identity {
                                name: item.name.trim().to_owned(),
                                creator: item.creator,
                            });
                        Some((o.full_id, identity))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let before: Vec<_> = roots.iter().map(|(id, _)| self.hud_scales.scale(*id)).collect();
        self.hud_scales.refresh(&roots);
        for ((id, _), before) in roots.iter().zip(before) {
            if before != self.hud_scales.scale(*id)
                && let Some(root) = self.objects.index_of_uuid(id)
            {
                self.dirty_hud(root);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(name: &str, creator: u128) -> Identity {
        Identity {
            name: name.into(),
            creator: Uuid::from_u128(creator),
        }
    }

    #[test]
    fn wheel_is_reversible_fine_and_bounded() {
        let enlarged = wheel_scale(1.0, 5.0, false);
        assert!(enlarged > 1.5);
        assert!((wheel_scale(enlarged, -5.0, false) - 1.0).abs() < 1e-5);
        assert!(wheel_scale(1.0, 1.0, true) < wheel_scale(1.0, 1.0, false));
        assert_eq!(wheel_scale(MAX, 1000.0, false), MAX);
        assert_eq!(wheel_scale(MIN, -1000.0, false), MIN);
        assert_eq!(wheel_scale(1.0, f32::NAN, false), 1.0);
    }

    #[test]
    fn replacement_copies_restore_name_and_creator_without_changing_other_worn_copies() {
        let [a, b, c, d] = [1, 2, 3, 4].map(Uuid::from_u128);
        let profile = identity("HUD", 10);
        let mut scales = HudScales::default();
        scales.refresh(&[(a, Some(profile.clone())), (b, Some(profile.clone()))]);
        scales.set(a, 1.7);
        assert_eq!(scales.scale(b), 1.0);
        let bytes = serde_json::to_vec(&scales.profiles).unwrap();
        let mut restored = HudScales::default();
        restored.profiles = serde_json::from_slice(&bytes).unwrap();
        restored.refresh(&[(c, Some(profile.clone())), (d, Some(identity("HUD", 11)))]);
        assert_eq!(restored.scale(c), 1.7);
        assert_eq!(restored.scale(d), 1.0);
        restored.forget(c);
        assert_eq!(restored.scale(c), 1.7);
        restored.refresh(&[]);
        restored.refresh(&[(a, Some(profile))]);
        assert_eq!(restored.scale(a), 1.0);
    }

    #[test]
    fn late_inventory_metadata_saves_the_current_choice_and_reset_persists() {
        let id = Uuid::from_u128(1);
        let profile = identity("HUD", 10);
        let mut scales = HudScales::default();
        scales.refresh(&[(id, None)]);
        scales.set(id, 2.0);
        scales.refresh(&[(id, Some(profile.clone()))]);
        assert!(scales.has_profile(id));
        assert_eq!(scales.profiles[0].scale, 2.0);
        scales.set(id, 1.0);
        scales.refresh(&[]);
        scales.refresh(&[(Uuid::from_u128(2), Some(profile))]);
        assert_eq!(scales.scale(Uuid::from_u128(2)), 1.0);
    }

    #[test]
    fn asynchronous_save_is_flushed_and_account_files_are_independent() {
        let folder = std::env::temp_dir().join(format!("aurora-hud-scales-{}", Uuid::new_v4()));
        let path = folder.join(FILE);
        let id = Uuid::from_u128(1);
        let profile = identity("HUD", 10);
        let mut scales = HudScales::load(path.clone());
        scales.refresh(&[(id, Some(profile.clone()))]);
        scales.set(id, 2.0);
        scales.set(id, 1.8);
        drop(scales);
        let mut restored = HudScales::load(path.clone());
        restored.refresh(&[(id, Some(profile.clone()))]);
        assert_eq!(restored.scale(id), 1.8);
        let other_path = folder.join("other-account.json");
        let mut other = HudScales::load(other_path);
        other.refresh(&[(id, Some(profile))]);
        assert_eq!(other.scale(id), 1.0);
        drop(other);
        drop(restored);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
