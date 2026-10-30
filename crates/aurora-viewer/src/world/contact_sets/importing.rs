//! Read LGGContactSets::exportToLLSD / exportContactSet
//! (indra/newview/lggcontactsets.cpp). Aurora merges membership instead of
//! replacing existing sets; local appearance and nicknames take precedence.
use super::*;

pub struct ImportData(ContactSets);

impl ImportData {
    pub fn from_xml(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut data = aurora_llsd::from_xml(bytes).map_err(|_| "invalid LLSD XML")?;
        if data.has("groupname") && !data["groupname"].is_map() {
            let Llsd::String(name) = &data["groupname"] else {
                return Err("invalid set name");
            };
            if ContactSets::is_internal(name) || name.trim().is_empty() {
                return Err("reserved set name");
            }
            if !data.has("color") && !data.has("friends") && !data.has("notices") {
                return Err("not a contact set export");
            }
            let name = name.clone();
            // The individual export calls the full file's `notify` field
            // `notices`. Empty sets may omit `friends` entirely.
            data.insert("notify", data["notices"].clone());
            let mut full = Llsd::new_map();
            full.insert(name, data);
            data = full;
        }
        let map = data.as_map().ok_or("expected a contact sets map")?;
        if map.is_empty() {
            return Err("empty contact sets map");
        }
        for (name, value) in map {
            if name.trim().is_empty() || !value.is_map() {
                return Err("invalid contact set");
            }
            match name.as_str() {
                GLOBAL_SETTINGS => {
                    if !value.has("defaultColor") {
                        return Err("missing default color");
                    }
                    validate_color(&value["defaultColor"])?;
                }
                EXTRA_AVS | PSEUDONYM => validate_ids(value, name == PSEUDONYM)?,
                name if ContactSets::is_internal(name) => return Err("reserved set name"),
                _ => {
                    if !value.has("color") && !value.has("friends") && !value.has("notify") {
                        return Err("not a contact set");
                    }
                    if value.has("color") {
                        validate_color(&value["color"])?;
                    }
                    if value.has("friends") {
                        validate_ids(&value["friends"], false)?;
                    }
                }
            }
        }
        let mut sets = ContactSets {
            default_color: color_from(&data[GLOBAL_SETTINGS]["defaultColor"], GREY),
            ..Default::default()
        };
        sets.import(&data);
        Ok(Self(sets))
    }
}

fn validate_ids(data: &Llsd, aliases: bool) -> Result<(), &'static str> {
    for (key, value) in data.as_map().ok_or("expected a resident map")? {
        if Uuid::parse_str(key).map_or(true, |id| id.is_nil()) || (aliases && !matches!(value, Llsd::String(_))) {
            return Err("invalid resident");
        }
    }
    Ok(())
}

fn validate_color(data: &Llsd) -> Result<(), &'static str> {
    let values = data.as_array();
    if !(3..=4).contains(&values.len())
        || values
            .iter()
            .any(|v| !matches!(v, Llsd::Real(_) | Llsd::Integer(_)) || !v.as_f32().is_finite() || !(0.0..=1.0).contains(&v.as_f32()))
    {
        return Err("invalid color");
    }
    Ok(())
}

impl ContactSets {
    /// Validate before this call. Publish the complete merged file before
    /// changing live state, so a failed save leaves both versions intact.
    pub fn merge_import(&mut self, data: ImportData, is_friend: impl Fn(&Uuid) -> bool) -> std::io::Result<usize> {
        let mut next = self.clone();
        let count = data.0.sets.len();
        for (name, set) in data.0.sets {
            match next.sets.entry(name) {
                std::collections::btree_map::Entry::Occupied(mut entry) => entry.get_mut().friends.extend(set.friends),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(set);
                }
            }
        }
        next.extra.extend(data.0.extra);
        for (id, alias) in data.0.pseudonyms {
            next.pseudonyms.entry(id).or_insert(alias);
        }
        // Individual exports have no extraAvs section. Preserve their
        // non-friends just as addToSet does, without a friendship request.
        next.extra.extend(
            next.sets
                .values()
                .flat_map(|s| s.friends.iter())
                .chain(next.pseudonyms.keys())
                .filter(|id| !is_friend(id))
                .copied(),
        );
        if let Some(path) = &next.path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            crate::atomic_file::write(path, &aurora_llsd::to_xml(&next.export()))?;
        }
        next.generation += 1;
        next.rebuild_appearances();
        next.rebuild_chat_sections();
        *self = next;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn exported(name: &str, member: Uuid) -> Llsd {
        let mut data = Llsd::new_map();
        data.insert("groupname", name);
        data.insert("color", color_llsd([0.2, 0.4, 0.6, 1.0]));
        data.insert("notices", true);
        let mut members = Llsd::new_map();
        members.insert(member.to_string(), "");
        data.insert("friends", members);
        data
    }

    fn parse(data: &Llsd) -> ImportData {
        ImportData::from_xml(&aurora_llsd::to_xml(data)).expect("valid export")
    }

    #[test]
    fn contact_set_import_individual_preserves_nonfriends_and_is_idempotent() {
        let mut cs = ContactSets::default();
        let data = exported("Business", id(1));
        cs.merge_import(parse(&data), |_| false).expect("merge");
        assert!(cs.set("Business").expect("set").notify);
        assert!(cs.is_non_friend(&id(1), |_| false));
        let before = cs.export();
        cs.merge_import(parse(&data), |_| false).expect("repeat");
        assert_eq!(before, cs.export());
        assert_eq!(cs.members(ALL_SETS, &[], |_| false), vec![id(1)]);
    }

    #[test]
    fn contact_set_import_full_merges_members_and_preserves_local_settings() {
        let mut source = ContactSets::default();
        source.add_set("Work");
        source.add_set("New");
        source.add_to_set(&[id(2)], "Work", |_| false);
        source.set_pseudonym(&[id(2), id(3)], "Imported", |_| false);
        let mut cs = ContactSets::default();
        cs.add_set("Work");
        cs.set_color("Work", [1.0, 0.0, 0.0, 1.0]);
        cs.set_separate_chats("Work", true);
        cs.add_to_set(&[id(1)], "Work", |_| true);
        cs.set_pseudonym(&[id(2)], "Local", |_| true);
        let local = cs.set("Work").expect("set").clone();
        cs.merge_import(parse(&source.export()), |_| false).expect("merge");
        let mut expected = local;
        expected.friends.insert(id(2));
        assert_eq!(cs.set("Work"), Some(&expected));
        assert!(cs.is_valid("New"));
        assert_eq!(cs.pseudonym(&id(2)), Some("Local"));
        assert_eq!(cs.pseudonym(&id(3)), Some("Imported"));
        assert!(cs.chat_section(&id(2)).is_some());
    }

    #[test]
    fn contact_set_import_rejects_unrelated_and_corrupt_files() {
        for bytes in [
            b"no XML".as_slice(),
            b"<llsd><array /></llsd>",
            b"<llsd><map><key>Settings</key><map><key>Value</key><string>x</string></map></map></llsd>",
        ] {
            assert!(ImportData::from_xml(bytes).is_err());
        }
        let mut data = exported("Work", Uuid::nil());
        assert!(ImportData::from_xml(&aurora_llsd::to_xml(&data)).is_err());
        data.insert("groupname", EXTRA_AVS);
        assert!(ImportData::from_xml(&aurora_llsd::to_xml(&data)).is_err());
    }

    #[test]
    fn contact_set_import_persists_and_failed_save_leaves_live_state_intact() {
        let root = std::env::temp_dir().join(format!("aurora-contact-import-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let path = root.join(CONTACT_SETS_FILE);
        let mut cs = ContactSets::load(path.clone());
        cs.merge_import(parse(&exported("Work", id(1))), |_| false).expect("save");
        let loaded = ContactSets::load(path);
        assert_eq!(cs.export(), loaded.export());
        cs.path = Some(root.clone()); // a directory cannot be replaced by a file
        let before = cs.export();
        let generation = cs.generation;
        assert!(cs.merge_import(parse(&exported("Other", id(2))), |_| false).is_err());
        assert_eq!(cs.export(), before);
        assert_eq!(cs.generation, generation);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
