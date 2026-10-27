//! Derender and the asset blacklist of objects, residents, sounds and
//! animations. Port of FSAssetBlacklist (indra/newview/fsassetblacklist.cpp)
//! for its object, resident, sound and animation entries, and of the derender list of LLViewerObjectList
//! (mDerendered, addDerenderedItem / resetDerenderList,
//! indra/newview/llviewerobjectlist.cpp, originally LGPL 2.1).
//!
//! A derendered object or avatar is taken out of the world, with what hangs
//! from it, and its updates are dropped while its entry lasts: nothing of it
//! is drawn, heard or clicked, at no cost to the frame (the check is made
//! per object update, and not at all while the list is empty). A temporary
//! entry lasts until the next teleport (FSTempDerenderUntilTeleport, on by
//! default) or until the Asset blacklist window clears it; a permanent one
//! is written to the account's `asset_blacklist.xml`, in the Legacy
//! Viewer's format (UUIDs XOR-masked, LLSD XML), and survives relogs.
//!
//! Deliberate difference: the Legacy Viewer waits for the simulator to send
//! an object again once it leaves the list (a removed object may stay away
//! until the next teleport). Aurora puts it back at once from the last
//! update it kept, then asks the simulator for it (RequestMultipleObjects).

use super::objects::ObjKey;
use aurora_llsd::Llsd;
use aurora_net::RegionHandle;
use aurora_net::objects::ObjectUpdate;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// The account's file, as the Legacy Viewer names it (LL_PATH_PER_SL_ACCOUNT).
pub const FILE: &str = "asset_blacklist.xml";

/// FSAssetBlacklist's MAGIC_ID: the key that masks the UUIDs of the file.
const MAGIC_ID: Uuid = Uuid::from_u128(0x3c11_5e51_04f4_523c_9fa6_98af_f103_4730);

/// LLAssetType values of the file's entries Aurora acts on.
const AT_OBJECT: i32 = 6;
const AT_SOUND: i32 = 1;
const AT_ANIMATION: i32 = 20;
const AT_PERSON: i32 = 45;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Object,
    Resident,
    /// 0: asset UUID; 1/2/4: owner's worn/rezzed/gesture sounds.
    Sound(u8),
    /// An animation asset, never played on any avatar (AT_ANIMATION).
    Animation,
}

impl Kind {
    fn asset_type(self) -> i32 {
        match self {
            Kind::Object => AT_OBJECT,
            Kind::Resident => AT_PERSON,
            Kind::Sound(_) => AT_SOUND,
            Kind::Animation => AT_ANIMATION,
        }
    }
}

/// FSAssetBlacklistData.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    /// The object's region (empty for a resident, as derenderObject leaves it).
    pub region: String,
    pub kind: Kind,
    /// When it was added, in seconds since the epoch (UTC).
    pub date: i64,
    pub permanent: bool,
}

/// One blacklist row: a complete entry (flag 0), or one sound category.
/// FSFloaterAssetBlacklist::addElementToList / removeElements use the same
/// UUID with a separate flag for each category, keeping the saved bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowId {
    pub id: Uuid,
    pub flag: u8,
}

impl RowId {
    pub fn whole(id: Uuid) -> Self {
        Self { id, flag: 0 }
    }
}

/// An object kept out of the world: the entry that hides it and its last
/// full update, to put it back without waiting for the simulator.
struct Hidden {
    by: Uuid,
    update: Option<ObjectUpdate>,
}

/// An object update dropped: the key it hides, the entry that hides it,
/// and whether this key is hidden for the first time (what already hangs
/// from it in the world must go too).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dropped {
    pub key: ObjKey,
    pub by: Uuid,
    pub newly: bool,
}

/// Objects to put back after entries left the list: their last updates, and
/// the local ids to ask the simulator for, by region.
#[derive(Default, Debug)]
pub struct Restore {
    pub updates: Vec<(RegionHandle, ObjectUpdate)>,
    pub requests: HashMap<RegionHandle, Vec<u32>>,
}

#[derive(Default)]
pub struct Derender {
    entries: HashMap<Uuid, Entry>,
    /// The file's entries of other asset types (textures): Aurora does
    /// not act on them and writes them back as they came.
    other: Vec<(String, Llsd)>,
    hidden: HashMap<ObjKey, Hidden>,
    path: Option<PathBuf>,
    /// RequestMultipleObjects waiting for the app.
    requests: Vec<(RegionHandle, Vec<u32>)>,
    /// Bumped on every change of the list (the window's rows).
    pub generation: u64,
}

/// XOR with MAGIC_ID (LLXORCipher over the 16 bytes): the same to mask and
/// to unmask.
fn mask(id: Uuid) -> Uuid {
    let (a, b) = (id.as_bytes(), MAGIC_ID.as_bytes());
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = a[i] ^ b[i];
    }
    Uuid::from_bytes(out)
}

/// "YYYY-MM-DD HH:MM:SS", the file's asset_date (LLDate::asString without
/// the T and the Z).
fn format_date(secs: i64) -> String {
    let (y, m, d) = aurora_net::profile::civil_from_days(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
}

/// Seconds of an asset_date, its fraction of a second ignored; 0 if unreadable.
fn parse_date(s: &str) -> i64 {
    let s = s.trim().trim_end_matches('Z');
    let (day, time) = s.split_once([' ', 'T']).unwrap_or((s, "00:00:00"));
    let num = |v: Option<&str>| v.and_then(|v| v.split('.').next()?.trim().parse::<i64>().ok());
    let mut d = day.split('-');
    let (Some(y), Some(m), Some(dd)) = (num(d.next()), num(d.next()), num(d.next())) else {
        return 0;
    };
    let mut t = time.split(':');
    let (h, mi, sec) = (num(t.next()).unwrap_or(0), num(t.next()).unwrap_or(0), num(t.next()).unwrap_or(0));
    aurora_net::profile::days_from_civil(y, m.clamp(1, 12) as u32, dd.clamp(1, 31) as u32) * 86_400 + h * 3600 + mi * 60 + sec
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Derender {
    /// FSAssetBlacklist::loadBlacklist: the account's permanent entries.
    pub fn load(path: PathBuf) -> Self {
        let mut list = Derender {
            path: Some(path.clone()),
            ..Derender::default()
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("asset blacklist not read: {e}");
                }
                return list;
            }
        };
        let data = match aurora_llsd::from_xml(&bytes) {
            Ok(data) => data,
            Err(e) => {
                log::warn!("asset blacklist unreadable: {e:?}");
                return list;
            }
        };
        for (key, value) in data.as_map().into_iter().flatten() {
            let Ok(masked) = Uuid::parse_str(key) else {
                continue;
            };
            let id = mask(masked);
            if id.is_nil() {
                continue;
            }
            // Derender entries, sound asset/owner categories and animations
            // share the legacy file; preserve unsupported asset types below.
            let kind = match value.get("asset_type").as_i32() {
                AT_OBJECT => Kind::Object,
                AT_PERSON => Kind::Resident,
                AT_SOUND => Kind::Sound(value.get("asset_blacklist_flag").as_i32() as u8),
                AT_ANIMATION => Kind::Animation,
                _ => {
                    list.other.push((key.clone(), value.clone()));
                    continue;
                }
            };
            list.entries.insert(
                id,
                Entry {
                    name: value.get("asset_name").as_str().to_owned(),
                    region: value.get("asset_region").as_str().to_owned(),
                    kind,
                    date: parse_date(value.get("asset_date").as_str()),
                    permanent: true,
                },
            );
        }
        log::info!("asset blacklist: {} entries", list.entries.len());
        list
    }

    /// FSAssetBlacklist::saveBlacklist: the permanent entries only.
    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut map = aurora_llsd::Map::new();
        for (key, value) in &self.other {
            map.insert(key.clone(), value.clone());
        }
        for (id, e) in self.entries.iter().filter(|(_, e)| e.permanent) {
            let mut data = aurora_llsd::Map::new();
            data.insert("asset_name".into(), Llsd::from(e.name.as_str()));
            data.insert("asset_region".into(), Llsd::from(e.region.as_str()));
            data.insert("asset_type".into(), Llsd::from(e.kind.asset_type()));
            let flags = match e.kind {
                Kind::Sound(flags) => i32::from(flags),
                _ => 0,
            };
            data.insert("asset_blacklist_flag".into(), Llsd::from(flags));
            data.insert("asset_date".into(), Llsd::from(format_date(e.date)));
            data.insert("asset_permanent".into(), Llsd::from(true));
            map.insert(mask(*id).to_string(), Llsd::Map(data));
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = crate::atomic_file::write(path, &aurora_llsd::to_xml(&Llsd::Map(map))) {
            log::warn!("asset blacklist not saved: {e}");
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.hidden.is_empty()
    }

    pub fn contains(&self, id: &Uuid) -> bool {
        self.entries
            .get(id)
            .is_some_and(|e| matches!(e.kind, Kind::Object | Kind::Resident))
    }

    pub fn sound_blocked(&self, asset: Uuid, owner: Uuid, flag: u8) -> bool {
        self.entries.get(&asset).is_some_and(|e| e.kind == Kind::Sound(0))
            || (!owner.is_nil()
                && self
                    .entries
                    .get(&owner)
                    .is_some_and(|e| matches!(e.kind, Kind::Sound(flags) if flags & flag != 0)))
    }

    /// FSAssetBlacklist::isBlacklisted(id, AT_ANIMATION).
    pub fn animation_blocked(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(|e| e.kind == Kind::Animation)
    }

    pub fn owner_sound_flags(&self, owner: Uuid) -> u8 {
        match self.entries.get(&owner).map(|e| e.kind) {
            Some(Kind::Sound(flags)) => flags & 7,
            _ => 0,
        }
    }

    /// Set one category without changing the other two, using the same saved
    /// mask as Sound Explorer. OFF removes only that category's row.
    pub fn set_sound_category(&mut self, owner: Uuid, flag: u8, blocked: bool, mut entry: Entry) {
        if owner.is_nil()
            || !matches!(flag, 1 | 2 | 4)
            || self.entries.get(&owner).is_some_and(|e| !matches!(e.kind, Kind::Sound(_)))
            || (self.owner_sound_flags(owner) & flag != 0) == blocked
        {
            return;
        }
        if blocked {
            entry.kind = Kind::Sound(flag);
            self.add_sounds([(owner, entry)]);
        } else {
            self.remove_rows(&[RowId { id: owner, flag }]);
        }
    }

    #[cfg(test)]
    pub fn add_sound(&mut self, id: Uuid, entry: Entry) {
        self.add_sounds(std::iter::once((id, entry)));
    }

    /// One selection is saved once, including when many rows share an owner.
    pub fn add_sounds(&mut self, entries: impl IntoIterator<Item = (Uuid, Entry)>) {
        let mut save = false;
        let mut changed = false;
        for (id, mut entry) in entries {
            if id.is_nil() {
                continue;
            }
            if let Kind::Sound(flags) = entry.kind
                && flags != 0
                && let Some(Entry {
                    kind: Kind::Sound(old), ..
                }) = self.entries.get(&id)
                && *old != 0
            {
                entry.kind = Kind::Sound(flags | old);
            }
            save |= entry.permanent || self.entries.get(&id).is_some_and(|e| e.permanent);
            self.entries.insert(id, entry);
            changed = true;
        }
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        if save {
            self.save();
        }
    }

    /// The list, for the window.
    pub fn entries(&self) -> impl Iterator<Item = (&Uuid, &Entry)> {
        self.entries.iter()
    }

    pub fn has_row(&self, row: RowId) -> bool {
        self.entries.get(&row.id).is_some_and(|entry| match entry.kind {
            Kind::Sound(flags) if flags != 0 => row.flag != 0 && flags & row.flag == row.flag,
            _ => row.flag == 0,
        })
    }

    /// FSAssetBlacklist::addNewItemToBlacklist: an entry made again takes
    /// the new name, date and permanence.
    pub fn add(&mut self, id: Uuid, entry: Entry) {
        let save = entry.permanent || self.entries.get(&id).is_some_and(|e| e.permanent);
        self.entries.insert(id, entry);
        self.generation += 1;
        if save {
            self.save();
        }
    }

    /// LLViewerObjectList::processObjectUpdate / createObject: the update
    /// of a derendered object, or of what hangs from one, is dropped (and
    /// kept, to put the object back without waiting for the simulator).
    pub fn filter(&mut self, region: RegionHandle, u: &ObjectUpdate) -> Option<Dropped> {
        if self.is_empty() {
            return None;
        }
        let key = ObjKey {
            region,
            local_id: u.local_id,
        };
        let by = if self
            .entries
            .get(&u.full_id)
            .is_some_and(|e| matches!(e.kind, Kind::Object | Kind::Resident))
        {
            Some(u.full_id)
        } else if u.parent_id != 0 && !u.is_avatar() {
            // (an avatar sitting on it stays, as markDead's getOffObject)
            self.hidden
                .get(&ObjKey {
                    region,
                    local_id: u.parent_id,
                })
                .map(|h| h.by)
        } else {
            None
        };
        let Some(by) = by else {
            // it was hidden with a parent it has left (unlinked, detached)
            self.hidden.remove(&key);
            return None;
        };
        let update = Some(u.clone());
        let newly = self.hidden.insert(key, Hidden { by, update }).is_none();
        Some(Dropped { key, by, newly })
    }

    /// Objects taken out of the world by `by`, with what they looked like.
    pub fn hide(&mut self, by: Uuid, objects: Vec<(ObjKey, ObjectUpdate)>) {
        for (key, update) in objects {
            self.hidden.insert(key, Hidden { by, update: Some(update) });
        }
    }

    /// Objects listed before their name came (ObjectPropertiesFamily asked
    /// for at the derender).
    pub fn unnamed(&self) -> Vec<Uuid> {
        self.entries
            .iter()
            .filter(|(_, e)| e.kind == Kind::Object && e.name.is_empty())
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn rename(&mut self, id: &Uuid, name: &str) {
        let Some(e) = self.entries.get_mut(id).filter(|e| e.name != name) else {
            return;
        };
        e.name = name.to_owned();
        let permanent = e.permanent;
        self.generation += 1;
        if permanent {
            self.save();
        }
    }

    /// The simulator killed it: its local id may be given to another object.
    pub fn forget_key(&mut self, key: &ObjKey) {
        if !self.hidden.is_empty() {
            self.hidden.remove(key);
        }
    }

    /// The region left the world.
    pub fn forget_region(&mut self, region: RegionHandle) {
        if !self.hidden.is_empty() {
            self.hidden.retain(|k, _| k.region != region);
        }
    }

    /// FSAssetBlacklist::removeItemsFromBlacklist: what they hid comes back.
    pub fn remove(&mut self, ids: &[Uuid]) -> Restore {
        self.remove_rows(&ids.iter().copied().map(RowId::whole).collect::<Vec<_>>())
    }

    /// Remove only selected categories; aggregate multiple rows before saving.
    /// FSAssetBlacklist::removeFlagsFromItem (fsassetblacklist.cpp).
    pub fn remove_rows(&mut self, rows: &[RowId]) -> Restore {
        let mut flags_by_id = HashMap::<Uuid, (u8, bool)>::new();
        for row in rows {
            let removal = flags_by_id.entry(row.id).or_default();
            removal.0 |= row.flag;
            removal.1 |= row.flag == 0;
        }
        let mut save = false;
        let mut changed = false;
        let mut gone = Vec::new();
        for (id, (flags, full)) in flags_by_id {
            if !full {
                let Some(e) = self.entries.get_mut(&id) else {
                    continue;
                };
                let Kind::Sound(current) = e.kind else {
                    continue;
                };
                let remaining = current & !flags;
                if current == remaining {
                    continue;
                }
                save |= e.permanent;
                changed = true;
                if remaining != 0 {
                    e.kind = Kind::Sound(remaining);
                    continue;
                }
            }
            if let Some(e) = self.entries.remove(&id) {
                save |= e.permanent;
                changed = true;
                gone.push(id);
            }
        }
        if !changed {
            return Restore::default();
        }
        self.generation = self.generation.wrapping_add(1);
        if save {
            self.save();
        }
        self.release(&gone)
    }

    /// LLViewerObjectList::resetDerenderList: the temporary entries go (at a
    /// teleport, FSTempDerenderUntilTeleport, or "Clear temporary").
    pub fn clear_temporary(&mut self) -> Restore {
        let temporary: Vec<Uuid> = self.entries.iter().filter(|(_, e)| !e.permanent).map(|(id, _)| *id).collect();
        self.remove(&temporary)
    }

    fn release(&mut self, ids: &[Uuid]) -> Restore {
        let mut restore = Restore::default();
        let keys: Vec<ObjKey> = self.hidden.iter().filter(|(_, h)| ids.contains(&h.by)).map(|(k, _)| *k).collect();
        for key in keys {
            let Some(h) = self.hidden.remove(&key) else {
                continue;
            };
            if let Some(u) = h.update {
                restore.updates.push((key.region, u));
            }
            restore.requests.entry(key.region).or_default().push(key.local_id);
        }
        for (region, ids) in &mut restore.requests {
            ids.sort_unstable();
            self.requests.push((*region, ids.clone()));
        }
        restore
    }

    /// RequestMultipleObjects for what came back, taken by the app.
    pub fn take_requests(&mut self) -> Vec<(RegionHandle, Vec<u32>)> {
        std::mem::take(&mut self.requests)
    }

    #[cfg(test)]
    pub fn hidden_count(&self) -> usize {
        self.hidden.len()
    }
}

impl super::World {
    /// derenderObject / LLAvatarActions::onDerenderAvatarNameLookup: the
    /// entry is listed and the object (an avatar, a linkset, something
    /// worn) leaves the world with what hangs from it. Returns true when
    /// our avatar sat on it and was taken off (the app stands it up).
    pub fn derender(&mut self, id: Uuid, entry: Entry) -> bool {
        if id == self.agent_id || id.is_nil() {
            return false;
        }
        self.derender.add(id, entry);
        match self.objects.index_of_uuid(&id) {
            Some(idx) => self.take_out(idx, id),
            None => false,
        }
    }

    /// Entries removed from the list (the Asset blacklist window).
    pub fn underender(&mut self, ids: &[Uuid]) {
        let restore = self.derender.remove(ids);
        self.put_back(restore);
    }

    pub fn remove_blacklist_rows(&mut self, rows: &[RowId]) {
        let restore = self.derender.remove_rows(rows);
        self.put_back(restore);
    }

    /// LLViewerObjectList::resetDerenderList: at a teleport
    /// (FSTempDerenderUntilTeleport) or by "Clear temporary".
    pub fn clear_temporary_derenders(&mut self) {
        let restore = self.derender.clear_temporary();
        self.put_back(restore);
    }

    /// An update dropped for key `key` hid it for the first time: what
    /// already was in the world under it goes too (an attachment that came
    /// before its derendered wearer, after a relog).
    pub(super) fn hide_key(&mut self, key: ObjKey, by: Uuid) -> bool {
        if let Some(idx) = self.objects.index_of(&key) {
            return self.take_out(idx, by);
        }
        let mut unseated = false;
        for child in self.objects.children_of(&key).to_vec() {
            if self.objects.get(child).is_some_and(|o| !o.is_avatar()) {
                unseated |= self.take_out(child, by);
            }
        }
        unseated
    }

    /// Take object `idx` and what hangs from it out of the world, keeping
    /// how they looked to put them back. Avatars sitting on it stay, where
    /// they are drawn (LLViewerObject::markDead: getOffObject).
    fn take_out(&mut self, idx: usize, by: Uuid) -> bool {
        let unseated = self.get_off_linkset(idx);
        let tree = self.objects.subtree(idx);
        let mut kept = Vec::with_capacity(tree.len());
        for &i in &tree {
            if let Some(o) = self.objects.get(i) {
                self.scripted_camera.clear(o.full_id);
                kept.push((o.key, crate::build::replace::demo_update(o)));
            }
        }
        log::info!("derender {by}: {} objects out of the world", kept.len());
        self.objects.remove_idx(idx);
        self.derender.hide(by, kept);
        self.forget_gone_look_ats();
        unseated
    }

    /// Objects back from the list: their last look at once, then what the
    /// simulator sends for them (the requests the app takes).
    fn put_back(&mut self, restore: Restore) {
        let mut by_region: HashMap<RegionHandle, Vec<ObjectUpdate>> = HashMap::new();
        for (region, u) in restore.updates {
            by_region.entry(region).or_default().push(u);
        }
        for (handle, objects) in by_region {
            if self.regions.contains_key(&handle) {
                let _ = self.apply(aurora_net::NetEvent::ObjectUpdates { handle, objects });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(local_id: u32, full_id: u128, parent_id: u32) -> ObjectUpdate {
        use glam::{Quat, Vec3};
        ObjectUpdate {
            local_id,
            full_id: Uuid::from_u128(full_id),
            parent_id,
            pcode: aurora_prim::params::LL_PCODE_VOLUME,
            state: 0,
            crc: 0,
            material: 3,
            click_action: 0,
            scale: Vec3::ONE,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            acceleration: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            update_flags: 0,
            owner_id: Uuid::nil(),
            volume: Default::default(),
            texture_entry: None,
            extra: Default::default(),
            name_values: String::new(),
            text: String::new(),
            text_color: [0; 4],
            media_url: String::new(),
            tree_species: None,
            texture_anim: None,
            foot_plane: None,
            particles: Default::default(),
            sound: Default::default(),
        }
    }

    fn entry(kind: Kind, permanent: bool) -> Entry {
        Entry {
            name: "Lantern".into(),
            region: "Plaza".into(),
            kind,
            date: 1_700_000_000,
            permanent,
        }
    }

    #[test]
    fn the_file_masks_uuids_like_the_legacy_viewer() {
        // the key of the Legacy Viewer's file for a nil-masked id is MAGIC_ID
        assert_eq!(mask(Uuid::nil()), MAGIC_ID);
        let id = Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        assert_eq!(mask(mask(id)), id);
        assert_eq!(format_date(1_700_000_000), "2023-11-14 22:13:20");
        assert_eq!(parse_date("2023-11-14 22:13:20"), 1_700_000_000);
        // LLDate keeps hundredths of a second
        assert_eq!(parse_date("2023-11-14 22:13:20.53"), 1_700_000_000);
        assert_eq!(parse_date("garbage"), 0);
    }

    #[test]
    fn animation_entries_persist_as_the_legacy_asset_type_and_never_derender() {
        let dir = std::env::temp_dir().join(format!("aurora-animation-blacklist-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        let anim = Uuid::from_u128(404);
        let mut list = Derender::load(path.clone());
        list.add(anim, entry(Kind::Animation, true));
        let saved = aurora_llsd::from_xml(&std::fs::read(&path).expect("saved")).expect("llsd");
        assert_eq!(saved.get(&mask(anim).to_string()).get("asset_type").as_i32(), AT_ANIMATION);
        let mut list = Derender::load(path.clone());
        assert!(list.animation_blocked(&anim));
        assert!(!list.animation_blocked(&Uuid::from_u128(405)));
        assert!(!list.contains(&anim), "an animation never hides an object");
        assert!(!list.sound_blocked(anim, Uuid::nil(), 0));
        list.remove(&[anim]);
        assert!(!Derender::load(path).animation_blocked(&anim));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sound_explorer_sound_flags_persist_merge_and_never_derender_the_owner() {
        let dir = std::env::temp_dir().join(format!("aurora-sound-blacklist-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        let owner = Uuid::from_u128(101);
        let asset = Uuid::from_u128(202);
        let mut list = Derender::load(path.clone());
        list.add_sound(owner, entry(Kind::Sound(1), true));
        list.add_sound(owner, entry(Kind::Sound(4), true));
        list.add_sound(asset, entry(Kind::Sound(0), true));
        let mut list = Derender::load(path);
        assert!(list.sound_blocked(asset, Uuid::nil(), 0));
        assert!(list.sound_blocked(Uuid::nil(), owner, 1));
        assert!(list.sound_blocked(Uuid::nil(), owner, 4));
        assert!(!list.sound_blocked(Uuid::nil(), owner, 2));
        assert!(
            !list.contains(&owner),
            "sound categories never affect water effects or derender checks"
        );
        assert!(list.filter(1, &update(11, 101, 0)).is_none());
        assert!(list.remove(&[owner, asset]).updates.is_empty());
        assert!(list.is_empty());
        assert!(Derender::load(dir.join(FILE)).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn avatar_sound_category_switches_set_and_clear_independent_persistent_bits() {
        let dir = std::env::temp_dir().join(format!("aurora-avatar-sound-categories-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        let owner = Uuid::from_u128(101);
        let asset = Uuid::from_u128(202);
        let mut list = Derender::load(path.clone());
        list.add_sound(asset, entry(Kind::Sound(0), true));
        for flag in [1, 2, 4] {
            list.set_sound_category(owner, flag, true, entry(Kind::Sound(flag), true));
        }
        assert_eq!(list.owner_sound_flags(owner), 7);
        let generation = list.generation;
        list.set_sound_category(owner, 1, true, entry(Kind::Sound(1), true));
        assert_eq!(list.generation, generation, "setting an existing state is idempotent");
        list.set_sound_category(owner, 2, false, entry(Kind::Sound(2), true));
        let mut list = Derender::load(path.clone());
        assert_eq!(list.owner_sound_flags(owner), 5);
        assert!(!list.sound_blocked(Uuid::nil(), owner, 2));
        assert!(list.sound_blocked(Uuid::nil(), owner, 1) && list.sound_blocked(Uuid::nil(), owner, 4));
        assert!(!list.contains(&owner), "muting categories never hides the avatar");
        for flag in [1, 4] {
            list.set_sound_category(owner, flag, false, entry(Kind::Sound(flag), true));
        }
        let mut list = Derender::load(path);
        assert_eq!(list.owner_sound_flags(owner), 0);
        assert!(list.entries().all(|(id, _)| *id == asset), "only the unrelated sound asset remains");
        for (owner, flag) in [(owner, 0), (owner, 3), (Uuid::nil(), 1)] {
            list.set_sound_category(owner, flag, true, entry(Kind::Sound(flag), true));
        }
        assert_eq!(list.entries().count(), 1);
        let hidden = Uuid::from_u128(303);
        list.add(hidden, entry(Kind::Resident, false));
        list.set_sound_category(hidden, 1, true, entry(Kind::Sound(1), true));
        assert!(list.contains(&hidden), "sound switches cannot replace a derender entry");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn blacklist_sound_categories_can_be_removed_independently_across_relogs() {
        let dir = std::env::temp_dir().join(format!("aurora-blacklist-rows-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        let owner = Uuid::from_u128(101);
        let asset = Uuid::from_u128(202);
        let mut list = Derender::load(path.clone());
        list.add_sounds([(owner, entry(Kind::Sound(7), true)), (asset, entry(Kind::Sound(0), true))]);
        let rezzed = RowId { id: owner, flag: 2 };
        assert!(list.has_row(rezzed));
        let restore = list.remove_rows(&[rezzed]);
        assert!(restore.updates.is_empty() && restore.requests.is_empty());
        let mut list = Derender::load(path.clone());
        assert!(!list.has_row(rezzed));
        assert!(!list.sound_blocked(Uuid::nil(), owner, 2));
        assert!(list.sound_blocked(Uuid::nil(), owner, 1) && list.sound_blocked(Uuid::nil(), owner, 4));
        assert!(list.sound_blocked(asset, Uuid::nil(), 0));
        let worn = RowId { id: owner, flag: 1 };
        let gesture = RowId { id: owner, flag: 4 };
        list.remove_rows(&[worn, gesture, worn]);
        let mut list = Derender::load(path);
        assert!(
            !list.entries.contains_key(&owner),
            "last selected category removes the saved owner entry"
        );
        assert!(list.has_row(RowId::whole(asset)), "an unrelated blocked asset stays");
        list.add(owner, entry(Kind::Object, true));
        list.remove_rows(&[rezzed]);
        assert!(list.contains(&owner), "a stale category cannot remove a replacement object entry");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sound_explorer_owner_categories_follow_linked_attachments_and_gestures() {
        let (mut world, _) = demo_world();
        let owner = Uuid::from_u128(999);
        let asset = Uuid::from_u128(777);
        let worn = world.objects.iter().find(|(_, o)| o.key.local_id == 9100).unwrap().1.full_id;
        let rezzed = world.objects.iter().find(|(_, o)| o.key.local_id == 101).unwrap().1.full_id;
        world.derender.add_sound(owner, entry(Kind::Sound(1), false));
        assert!(world.sound_blacklisted(asset, worn, owner));
        assert!(!world.sound_blacklisted(asset, rezzed, owner));
        assert!(!world.sound_blacklisted(asset, owner, owner));
        world.derender.add_sound(owner, entry(Kind::Sound(4), false));
        assert!(world.sound_blacklisted(asset, owner, owner));
        assert!(!world.sound_blacklisted(asset, rezzed, owner));
        world.derender.add_sound(owner, entry(Kind::Sound(2), false));
        assert!(world.sound_blacklisted(asset, rezzed, owner));
        world.derender.remove(&[owner]);
        assert!(!world.sound_blacklisted(asset, worn, owner));
    }

    #[test]
    fn permanent_entries_survive_a_relog_and_others_are_kept_untouched() {
        let dir = std::env::temp_dir().join(format!("aurora-blacklist-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        // a sound entry written by the Legacy Viewer, which Aurora keeps
        let sound = Uuid::from_u128(77);
        let mut legacy = aurora_llsd::Map::new();
        let mut data = aurora_llsd::Map::new();
        data.insert("asset_name".into(), Llsd::from("Loud"));
        data.insert("asset_type".into(), Llsd::from(1));
        data.insert("asset_date".into(), Llsd::from("2024-01-02 03:04:05"));
        legacy.insert(mask(sound).to_string(), Llsd::Map(data));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(&path, aurora_llsd::to_xml(&Llsd::Map(legacy))).expect("write");

        let mut list = Derender::load(path.clone());
        assert!(list.sound_blocked(sound, Uuid::nil(), 0));
        let (avatar, object) = (Uuid::from_u128(1), Uuid::from_u128(2));
        list.add(avatar, entry(Kind::Resident, true));
        list.add(object, entry(Kind::Object, false));

        let again = Derender::load(path.clone());
        assert!(again.contains(&avatar));
        assert!(!again.contains(&object), "a temporary entry is not written");
        assert_eq!(again.entries.get(&avatar), Some(&entry(Kind::Resident, true)));
        let raw = aurora_llsd::from_xml(&std::fs::read(&path).expect("read")).expect("llsd");
        assert!(raw.has(&mask(sound).to_string()), "the sound entry is written back");
        assert!(raw.has(&mask(avatar).to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_derendered_object_and_what_hangs_from_it_are_dropped_until_removed() {
        let mut list = Derender::default();
        // nothing listed: everything passes
        assert!(list.filter(1, &update(10, 100, 0)).is_none());
        let avatar = Uuid::from_u128(100);
        list.add(avatar, entry(Kind::Resident, false));
        assert!(matches!(list.filter(1, &update(10, 100, 0)), Some(Dropped { newly: true, .. })));
        // its attachment, then a prim linked to the attachment
        assert!(matches!(list.filter(1, &update(11, 101, 10)), Some(Dropped { by, .. }) if by == avatar));
        assert!(list.filter(1, &update(12, 102, 11)).is_some());
        // the same key again is not new
        assert!(matches!(list.filter(1, &update(10, 100, 0)), Some(Dropped { newly: false, .. })));
        // another region's object with the same local id is not hidden
        assert!(list.filter(2, &update(11, 201, 0)).is_none());
        assert_eq!(list.hidden_count(), 3);

        let restore = list.remove(&[avatar]);
        assert_eq!(restore.updates.len(), 3);
        assert_eq!(restore.requests.get(&1), Some(&vec![10, 11, 12]));
        assert_eq!(list.take_requests(), vec![(1, vec![10, 11, 12])]);
        assert!(list.is_empty());
        assert!(list.filter(1, &update(10, 100, 0)).is_none());
    }

    /// The demo's plaza in a world: Violet Wolf (9001) wears a hat (9100)
    /// and an orb (9101).
    fn demo_world() -> (super::super::World, RegionHandle) {
        use aurora_net::NetEvent;
        let mut world = super::super::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        let mut handle = 0;
        for ev in crate::demo::events() {
            if let NetEvent::ObjectUpdates { handle: h, .. } = &ev {
                handle = *h;
                world.main_region = Some(*h);
                world.ensure_region(*h, (256, 256));
            }
            if matches!(ev, NetEvent::ObjectUpdates { .. }) {
                let _ = world.apply(ev);
            }
        }
        (world, handle)
    }

    #[test]
    fn derendering_an_avatar_takes_what_it_wears_and_putting_it_back_restores_both() {
        use aurora_net::NetEvent;
        let (mut world, handle) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        let hat = crate::demo::wolf_hat();
        let wolf_key = world
            .objects
            .get(world.objects.index_of_uuid(&wolf).expect("wolf"))
            .expect("wolf")
            .key;
        let worn = |w: &super::super::World| {
            w.objects
                .index_of_uuid(&wolf)
                .and_then(|i| w.objects.get(i))
                .map_or(0, |o| w.objects.children_of(&o.key).len())
        };
        assert_eq!(worn(&world), 2);
        assert!(!world.derender(wolf, entry(Kind::Resident, false)));
        assert!(world.objects.index_of_uuid(&wolf).is_none());
        assert!(world.objects.index_of_uuid(&hat).is_none());
        // the simulator sends them again: dropped
        let resent = world.derender.hidden.values().filter_map(|h| h.update.clone()).collect::<Vec<_>>();
        assert_eq!(resent.len(), 3);
        let _ = world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: resent.clone(),
        });
        assert!(world.objects.index_of_uuid(&wolf).is_none());
        assert!(world.objects.index_of_uuid(&hat).is_none());
        // a teleport: the temporary entry goes, everything comes back at
        // once and is asked for again
        world.begin_teleport(false);
        assert_eq!(worn(&world), 2);
        assert_eq!(
            world.objects.get(world.objects.index_of_uuid(&wolf).expect("back")).map(|o| o.key),
            Some(wolf_key)
        );
        let requests = world.derender.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1.len(), 3);
    }

    #[test]
    fn a_worn_object_alone_can_be_derendered_and_a_relog_keeps_it_out() {
        use aurora_net::NetEvent;
        let (mut world, handle) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        let hat = crate::demo::wolf_hat();
        let hat_update = {
            let o = world.objects.get(world.objects.index_of_uuid(&hat).expect("hat")).expect("hat");
            crate::build::replace::demo_update(o)
        };
        assert!(!world.derender(hat, entry(Kind::Object, true)));
        assert!(world.objects.index_of_uuid(&hat).is_none());
        assert!(world.objects.index_of_uuid(&wolf).is_some(), "its wearer stays");
        // after a relog the hat comes first, then its wearer
        world.derender.hidden.clear();
        let _ = world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![hat_update],
        });
        assert!(world.objects.index_of_uuid(&hat).is_none());
        world.underender(&[hat]);
        assert!(world.objects.index_of_uuid(&hat).is_some());
    }

    #[test]
    fn an_attachment_that_came_before_its_derendered_wearer_goes_with_it() {
        use aurora_net::NetEvent;
        let (mut world, handle) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        let hat = crate::demo::wolf_hat();
        let wolf_update =
            crate::build::replace::demo_update(world.objects.get(world.objects.index_of_uuid(&wolf).expect("wolf")).expect("wolf"));
        // listed (a relog with a permanent entry) while the hat is already here
        world.derender.add(wolf, entry(Kind::Resident, false));
        let key = world
            .objects
            .get(world.objects.index_of_uuid(&wolf).expect("wolf"))
            .expect("wolf")
            .key;
        world.objects.remove_idx(world.objects.index_of(&key).expect("wolf"));
        // (the hat stays an orphan until its wearer's update)
        let hat_update = {
            let mut u = wolf_update.clone();
            u.full_id = hat;
            u.local_id = 9100;
            u.parent_id = key.local_id;
            u.pcode = aurora_prim::params::LL_PCODE_VOLUME;
            u
        };
        let _ = world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![hat_update],
        });
        assert!(world.objects.index_of_uuid(&hat).is_some());
        let _ = world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![wolf_update],
        });
        assert!(world.objects.index_of_uuid(&wolf).is_none());
        assert!(world.objects.index_of_uuid(&hat).is_none());
    }

    #[test]
    fn a_teleport_clears_only_the_temporary_entries() {
        let mut list = Derender::default();
        let (temporary, permanent) = (Uuid::from_u128(5), Uuid::from_u128(6));
        list.add(temporary, entry(Kind::Object, false));
        list.add(permanent, entry(Kind::Object, true));
        list.hide(temporary, vec![(ObjKey { region: 1, local_id: 50 }, update(50, 5, 0))]);
        list.hide(permanent, vec![(ObjKey { region: 1, local_id: 60 }, update(60, 6, 0))]);
        let restore = list.clear_temporary();
        assert_eq!(restore.requests.get(&1), Some(&vec![50]));
        assert!(!list.contains(&temporary));
        assert!(list.contains(&permanent));
        assert_eq!(list.hidden_count(), 1);
        // a killed or left object is forgotten
        list.forget_key(&ObjKey { region: 1, local_id: 60 });
        assert_eq!(list.hidden_count(), 0);
    }
}
