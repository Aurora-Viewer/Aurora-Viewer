//! Contact sets ("Cercles"), port of Firestorm's LGGContactSets
//! (indra/newview/lggcontactsets.cpp, originally LGPL 2.1): named sets of
//! friends (and non-friends, kept in the "extraAvs" list) with a color,
//! plus aliases (pseudonyms) that replace an avatar's display name. Saved
//! per account in the same LLSD file as Firestorm
//! (settings_friends_groups.xml), so a Firestorm file can be copied over.

use aurora_llsd::Llsd;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use uuid::Uuid;

pub const CONTACT_SETS_FILE: &str = "settings_friends_groups.xml";

/// Internal set names (CS_SET_*): pseudo-sets of the set combo and keys of
/// the file that are not sets.
pub const ALL_SETS: &str = "All Sets";
pub const NO_SETS: &str = "No Sets";
pub const EXTRA_AVS: &str = "extraAvs";
pub const PSEUDONYM: &str = "Pseudonyms";
const GLOBAL_SETTINGS: &str = "globalSettings";
/// Alias meaning "display name removed" (CS_PSEUDONYM).
pub const DN_REMOVED: &str = "--- ---";

/// Alpha of a non-friend's color (COLOR_DAMPENING).
const COLOR_DAMPENING: f32 = 0.8;
/// LLColor4::grey.
const GREY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

#[derive(Debug, Clone, PartialEq)]
pub struct ContactSet {
    pub name: String,
    pub friends: BTreeSet<Uuid>,
    /// Online / offline notices for this set (mNotify).
    pub notify: bool,
    pub sort_by_online: bool,
    /// RGBA, 0..1.
    pub color: [f32; 4],
    /// Per-set automatic responses: kept as read so that the file round-trips
    /// (not used by Aurora yet).
    autoresponse: BTreeMap<String, Llsd>,
}

impl ContactSet {
    fn new(name: &str, color: [f32; 4]) -> ContactSet {
        ContactSet {
            name: name.to_owned(),
            friends: BTreeSet::new(),
            notify: false,
            sort_by_online: true,
            color,
            autoresponse: BTreeMap::new(),
        }
    }
}

/// Which avatars a set selection lists (FSPanelContactSets::generateAvatarList).
#[derive(Debug, Default)]
pub struct ContactSets {
    sets: BTreeMap<String, ContactSet>,
    pub default_color: [f32; 4],
    /// Non-friends added to a set or given an alias (mExtraAvatars).
    extra: BTreeSet<Uuid>,
    /// Aliases, unquoted (mPseudonyms); DN_REMOVED hides the display name.
    pseudonyms: BTreeMap<Uuid, String>,
    path: Option<PathBuf>,
    /// Bumped on every change (lists and names refresh).
    pub generation: u64,
}

fn color_from(v: &Llsd, default: [f32; 4]) -> [f32; 4] {
    if v.as_array().len() >= 3 {
        let a = v.as_array();
        let alpha = if a.len() >= 4 { a[3].as_f32() } else { 1.0 };
        [a[0].as_f32(), a[1].as_f32(), a[2].as_f32(), alpha]
    } else {
        default
    }
}

fn color_llsd(c: [f32; 4]) -> Llsd {
    let mut a = Llsd::new_array();
    for x in c {
        a.push(x as f64);
    }
    a
}

/// Keys of a set's automatic responses in the file.
const AUTORESPONSE_KEYS: [&str; 6] = [
    "autoresponse_busy_enabled",
    "autoresponse_busy",
    "autoresponse_mode_enabled",
    "autoresponse_mode",
    "autoresponse_nonfriends_enabled",
    "autoresponse_nonfriends",
];

impl ContactSets {
    pub fn is_internal(name: &str) -> bool {
        name.is_empty() || [EXTRA_AVS, PSEUDONYM, NO_SETS, ALL_SETS, GLOBAL_SETTINGS].contains(&name)
    }

    /// loadFromDisk: the per-account file, or a new empty one.
    pub fn load(path: PathBuf) -> ContactSets {
        let mut cs = ContactSets {
            default_color: GREY,
            ..Default::default()
        };
        match std::fs::read(&path) {
            Ok(bytes) => match aurora_llsd::from_xml(&bytes) {
                Ok(v) => cs.import(&v),
                Err(e) => log::warn!("contact sets unreadable ({e}): {}", path.display()),
            },
            Err(_) => log::info!("no contact sets yet: {}", path.display()),
        }
        cs.path = Some(path);
        // a fresh load is a change for the name cache
        cs.generation = 1;
        cs
    }

    /// importFromLLSD.
    fn import(&mut self, data: &Llsd) {
        let Some(map) = data.as_map() else {
            return;
        };
        for (key, value) in map.iter() {
            match key.as_str() {
                GLOBAL_SETTINGS => self.default_color = color_from(&value["defaultColor"], GREY),
                EXTRA_AVS => {
                    if let Some(m) = value.as_map() {
                        self.extra.extend(m.keys().filter_map(|k| Uuid::parse_str(k).ok()));
                    }
                }
                PSEUDONYM => {
                    if let Some(m) = value.as_map() {
                        for (k, v) in m.iter() {
                            if let Ok(id) = Uuid::parse_str(k) {
                                self.pseudonyms.insert(id, v.as_str().to_owned());
                            }
                        }
                    }
                }
                k if Self::is_internal(k) => {}
                name => {
                    let mut set = ContactSet::new(name, color_from(&value["color"], self.default_color));
                    set.notify = value["notify"].as_bool();
                    set.sort_by_online = !value.has("sort_by_online_status") || value["sort_by_online_status"].as_bool();
                    for k in AUTORESPONSE_KEYS {
                        if value.has(k) {
                            set.autoresponse.insert(k.to_owned(), value[k].clone());
                        }
                    }
                    if let Some(m) = value["friends"].as_map() {
                        set.friends.extend(m.keys().filter_map(|k| Uuid::parse_str(k).ok()));
                    }
                    self.sets.insert(name.to_owned(), set);
                }
            }
        }
    }

    /// exportToLLSD.
    fn export(&self) -> Llsd {
        let mut out = Llsd::new_map();
        let mut global = Llsd::new_map();
        global.insert("defaultColor", color_llsd(self.default_color));
        out.insert(GLOBAL_SETTINGS, global);
        let mut extra = Llsd::new_map();
        for id in &self.extra {
            extra.insert(id.to_string(), "");
        }
        out.insert(EXTRA_AVS, extra);
        let mut ps = Llsd::new_map();
        for (id, p) in &self.pseudonyms {
            ps.insert(id.to_string(), p.clone());
        }
        out.insert(PSEUDONYM, ps);
        for (name, set) in &self.sets {
            let mut s = Llsd::new_map();
            s.insert("color", color_llsd(set.color));
            s.insert("notify", set.notify);
            s.insert("sort_by_online_status", set.sort_by_online);
            for (k, v) in &set.autoresponse {
                s.insert(k.clone(), v.clone());
            }
            let mut friends = Llsd::new_map();
            for id in &set.friends {
                friends.insert(id.to_string(), "");
            }
            s.insert("friends", friends);
            out.insert(name.clone(), s);
        }
        out
    }

    /// saveToDisk, after every change.
    fn changed(&mut self) {
        self.generation += 1;
        let Some(path) = &self.path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&self.export())) {
            log::warn!("contact sets not saved ({e}): {}", path.display());
        }
    }

    /// getAllContactSets (sorted by name, like the std::map).
    pub fn set_names(&self) -> Vec<String> {
        self.sets.keys().cloned().collect()
    }

    pub fn set(&self, name: &str) -> Option<&ContactSet> {
        self.sets.get(name)
    }

    pub fn is_valid(&self, name: &str) -> bool {
        self.sets.contains_key(name)
    }

    pub fn add_set(&mut self, name: &str) -> bool {
        let name = name.trim();
        if Self::is_internal(name) || self.is_valid(name) {
            return false;
        }
        self.sets.insert(name.to_owned(), ContactSet::new(name, self.default_color));
        self.changed();
        true
    }

    pub fn rename_set(&mut self, name: &str, new_name: &str) -> bool {
        let new_name = new_name.trim();
        if Self::is_internal(name) || Self::is_internal(new_name) || self.is_valid(new_name) {
            return false;
        }
        let Some(mut set) = self.sets.remove(name) else {
            return false;
        };
        set.name = new_name.to_owned();
        self.sets.insert(new_name.to_owned(), set);
        self.changed();
        true
    }

    /// removeSet: non-friends that were only in this set (and have no
    /// alias) leave the extra list.
    pub fn remove_set(&mut self, name: &str, is_friend: impl Fn(&Uuid) -> bool) {
        let Some(set) = self.sets.remove(name) else {
            return;
        };
        for id in &set.friends {
            if !is_friend(id) && self.friend_sets(id).is_empty() && !self.pseudonyms.contains_key(id) {
                self.extra.remove(id);
            }
        }
        self.changed();
    }

    pub fn set_color(&mut self, name: &str, color: [f32; 4]) {
        if let Some(s) = self.sets.get_mut(name) {
            s.color = color;
            self.changed();
        }
    }

    pub fn set_default_color(&mut self, color: [f32; 4]) {
        self.default_color = color;
        self.changed();
    }

    pub fn set_sort_by_online(&mut self, name: &str, on: bool) {
        if let Some(s) = self.sets.get_mut(name) {
            s.sort_by_online = on;
            self.changed();
        }
    }

    /// getFriendSets.
    pub fn friend_sets(&self, id: &Uuid) -> Vec<String> {
        self.sets
            .values()
            .filter(|s| s.friends.contains(id))
            .map(|s| s.name.clone())
            .collect()
    }

    pub fn in_any_set(&self, id: &Uuid) -> bool {
        self.sets.values().any(|s| s.friends.contains(id))
    }

    /// addToSet: non-friends go to the extra list.
    pub fn add_to_set(&mut self, ids: &[Uuid], name: &str, is_friend: impl Fn(&Uuid) -> bool) {
        for id in ids {
            if !is_friend(id) {
                self.extra.insert(*id);
            }
            if let Some(s) = self.sets.get_mut(name) {
                s.friends.insert(*id);
            }
        }
        self.changed();
    }

    /// handleRemoveAvatarFromSetCallback: out of the set (or of a
    /// pseudo-set), and a non-friend left in no set without alias is
    /// forgotten.
    pub fn remove_from_set(&mut self, ids: &[Uuid], name: &str, is_friend: impl Fn(&Uuid) -> bool) {
        for id in ids {
            match name {
                EXTRA_AVS => self.forget_non_friend(id, &is_friend),
                PSEUDONYM => self.drop_pseudonym(id, &is_friend),
                _ => {
                    if let Some(s) = self.sets.get_mut(name) {
                        s.friends.remove(id);
                    }
                }
            }
            if !is_friend(id) && self.friend_sets(id).is_empty() && !self.pseudonyms.contains_key(id) {
                self.extra.remove(id);
            }
        }
        self.changed();
    }

    /// LLAvatarActions::moveToContactSet: out of `from`, into `to`.
    pub fn move_to_set(&mut self, ids: &[Uuid], from: &str, to: &str, is_friend: impl Fn(&Uuid) -> bool) {
        if !self.is_valid(to) {
            return;
        }
        for id in ids {
            if let Some(s) = self.sets.get_mut(from) {
                s.friends.remove(id);
            }
            if let Some(s) = self.sets.get_mut(to) {
                s.friends.insert(*id);
            }
            if !is_friend(id) {
                self.extra.insert(*id);
            }
        }
        self.changed();
    }

    /// removeNonFriendFromList.
    fn forget_non_friend(&mut self, id: &Uuid, is_friend: &impl Fn(&Uuid) -> bool) {
        if self.extra.remove(id) && !is_friend(id) {
            self.pseudonyms.remove(id);
            for s in self.sets.values_mut() {
                s.friends.remove(id);
            }
        }
    }

    fn drop_pseudonym(&mut self, id: &Uuid, is_friend: &impl Fn(&Uuid) -> bool) {
        if self.pseudonyms.remove(id).is_some() && !is_friend(id) && self.friend_sets(id).is_empty() {
            self.extra.remove(id);
        }
    }

    pub fn is_non_friend(&self, id: &Uuid, is_friend: impl Fn(&Uuid) -> bool) -> bool {
        !is_friend(id) && self.extra.contains(id)
    }

    /// The avatars a combo choice lists (generateAvatarList).
    pub fn members(&self, choice: &str, friends: &[Uuid], is_friend: impl Fn(&Uuid) -> bool) -> Vec<Uuid> {
        match choice {
            ALL_SETS => {
                let all: BTreeSet<Uuid> = self.sets.values().flat_map(|s| s.friends.iter().copied()).collect();
                all.into_iter().collect()
            }
            NO_SETS => friends.iter().filter(|id| !self.in_any_set(id)).copied().collect(),
            PSEUDONYM => self.pseudonyms.keys().copied().collect(),
            EXTRA_AVS => self.extra.iter().filter(|id| !is_friend(id)).copied().collect(),
            name => self.sets.get(name).map(|s| s.friends.iter().copied().collect()).unwrap_or_default(),
        }
    }

    /// Lists of the pseudo-sets and of sets with "sort by online status"
    /// put online avatars first (FSPanelContactSets::shouldSortByOnlineStatus).
    pub fn sorts_by_online(&self, choice: &str) -> bool {
        Self::is_internal(choice) || self.sets.get(choice).is_some_and(|s| s.sort_by_online)
    }

    /// getFriendColor: the color of the smallest set the avatar is in
    /// (toned down for a non-friend); None = the default color.
    pub fn friend_color(&self, id: &Uuid, is_friend: impl Fn(&Uuid) -> bool) -> Option<[f32; 4]> {
        let set = self
            .sets
            .values()
            .filter(|s| s.friends.contains(id))
            .min_by_key(|s| s.friends.len())?;
        let mut c = set.color;
        if self.is_non_friend(id, &is_friend) {
            c[3] = COLOR_DAMPENING;
        }
        (c != self.default_color).then_some(c)
    }

    /// hasFriendColorThatShouldShow: only for friends and listed
    /// non-friends.
    pub fn color_to_show(&self, id: &Uuid, is_friend: impl Fn(&Uuid) -> bool) -> Option<[f32; 4]> {
        if !is_friend(id) && !self.extra.contains(id) {
            return None;
        }
        self.friend_color(id, is_friend)
    }

    /// The raw alias (DN_REMOVED when the display name is hidden).
    pub fn pseudonym(&self, id: &Uuid) -> Option<&str> {
        self.pseudonyms.get(id).map(String::as_str)
    }

    pub fn pseudonyms(&self) -> &BTreeMap<Uuid, String> {
        &self.pseudonyms
    }

    pub fn has_display_name_removed(&self, id: &Uuid) -> bool {
        self.pseudonym(id) == Some(DN_REMOVED)
    }

    /// setPseudonym (handleSetAvatarPseudonymCallback); an empty alias is
    /// refused like the notification's empty text.
    pub fn set_pseudonym(&mut self, ids: &[Uuid], alias: &str, is_friend: impl Fn(&Uuid) -> bool) {
        let alias = alias.trim();
        if alias.is_empty() {
            return;
        }
        for id in ids {
            if !is_friend(id) {
                self.extra.insert(*id);
            }
            self.pseudonyms.insert(*id, alias.to_owned());
        }
        self.changed();
    }

    /// clearPseudonym.
    pub fn clear_pseudonym(&mut self, ids: &[Uuid], is_friend: impl Fn(&Uuid) -> bool) {
        for id in ids {
            self.drop_pseudonym(id, &is_friend);
        }
        self.changed();
    }

    /// removeDisplayName: the legacy name replaces the display name.
    pub fn remove_display_name(&mut self, ids: &[Uuid], is_friend: impl Fn(&Uuid) -> bool) {
        self.set_pseudonym(ids, DN_REMOVED, is_friend);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn sets_and_pseudo_sets() {
        let friends = [id(1), id(2), id(3)];
        let is_friend = |x: &Uuid| friends.contains(x);
        let mut cs = ContactSets::default();
        assert!(cs.add_set("Famille"));
        assert!(!cs.add_set("Famille"));
        assert!(!cs.add_set(ALL_SETS));
        cs.add_set("Travail");
        cs.add_to_set(&[id(1), id(9)], "Famille", is_friend);
        cs.add_to_set(&[id(1), id(2)], "Travail", is_friend);
        assert_eq!(cs.members("Famille", &friends, is_friend), vec![id(1), id(9)]);
        assert_eq!(cs.members(ALL_SETS, &friends, is_friend), vec![id(1), id(2), id(9)]);
        assert_eq!(cs.members(NO_SETS, &friends, is_friend), vec![id(3)]);
        // a non-friend added to a set is listed with the non-friends
        assert_eq!(cs.members(EXTRA_AVS, &friends, is_friend), vec![id(9)]);
        // removing the set forgets the non-friend that was only there
        cs.remove_set("Famille", is_friend);
        assert!(cs.members(EXTRA_AVS, &friends, is_friend).is_empty());
        assert_eq!(cs.set_names(), vec!["Travail".to_owned()]);
    }

    #[test]
    fn smallest_set_gives_the_color() {
        let is_friend = |_: &Uuid| true;
        let mut cs = ContactSets {
            default_color: GREY,
            ..Default::default()
        };
        cs.add_set("Grand");
        cs.add_set("Petit");
        cs.set_color("Grand", [1.0, 0.0, 0.0, 1.0]);
        cs.set_color("Petit", [0.0, 0.0, 1.0, 1.0]);
        cs.add_to_set(&[id(1), id(2), id(3)], "Grand", is_friend);
        cs.add_to_set(&[id(1)], "Petit", is_friend);
        assert_eq!(cs.friend_color(&id(1), is_friend), Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(cs.friend_color(&id(2), is_friend), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(cs.friend_color(&id(4), is_friend), None);
        // a set left at the default color colors nothing
        cs.add_set("Gris");
        cs.add_to_set(&[id(5)], "Gris", is_friend);
        assert_eq!(cs.friend_color(&id(5), is_friend), None);
    }

    #[test]
    fn aliases_and_removed_display_names() {
        let is_friend = |x: &Uuid| *x == id(1);
        let mut cs = ContactSets::default();
        cs.set_pseudonym(&[id(1), id(7)], " Bob ", is_friend);
        assert_eq!(cs.pseudonym(&id(1)), Some("Bob"));
        assert_eq!(cs.members(EXTRA_AVS, &[id(1)], is_friend), vec![id(7)]);
        cs.remove_display_name(&[id(1)], is_friend);
        assert!(cs.has_display_name_removed(&id(1)));
        // clearing the alias of a non-friend in no set forgets them
        cs.clear_pseudonym(&[id(7)], is_friend);
        assert!(cs.members(EXTRA_AVS, &[id(1)], is_friend).is_empty());
        assert_eq!(cs.members(PSEUDONYM, &[id(1)], is_friend), vec![id(1)]);
    }

    #[test]
    fn firestorm_file_round_trip() {
        let xml = br#"<?xml version="1.0" ?><llsd><map>
            <key>globalSettings</key><map><key>defaultColor</key><array><real>0.5</real><real>0.5</real><real>0.5</real><real>1</real></array></map>
            <key>extraAvs</key><map><key>00000000-0000-0000-0000-000000000009</key><string /></map>
            <key>Pseudonyms</key><map><key>00000000-0000-0000-0000-000000000001</key><string>Bob</string></map>
            <key>Amis proches</key><map>
              <key>color</key><array><real>1</real><real>0</real><real>0</real><real>1</real></array>
              <key>notify</key><boolean>1</boolean>
              <key>autoresponse_busy</key><string>Busy</string>
              <key>friends</key><map><key>00000000-0000-0000-0000-000000000001</key><string /></map>
            </map>
        </map></llsd>"#;
        let mut cs = ContactSets::default();
        cs.import(&aurora_llsd::from_xml(xml).expect("valid xml"));
        let set = cs.set("Amis proches").expect("set");
        assert!(set.notify && set.sort_by_online);
        assert_eq!(set.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(cs.pseudonym(&id(1)), Some("Bob"));
        let mut again = ContactSets::default();
        again.import(&aurora_llsd::from_xml(&aurora_llsd::to_xml(&cs.export())).expect("valid xml"));
        assert_eq!(again.set("Amis proches"), cs.set("Amis proches"));
        assert_eq!(again.extra, cs.extra);
        assert_eq!(again.pseudonyms, cs.pseudonyms);
    }
}
