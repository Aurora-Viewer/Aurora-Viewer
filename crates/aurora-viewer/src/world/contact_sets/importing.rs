//! Read LGGContactSets::exportToLLSD / exportContactSet
//! (indra/newview/lggcontactsets.cpp). Aurora merges membership instead of
//! replacing existing sets; local appearance and nicknames take precedence.
use super::*;

/// `from_xml`'s answer to a file with no set in it.
pub const EMPTY_MAP: &str = "empty contact sets map";

#[derive(Clone)]
pub struct ImportData(ContactSets);

impl ImportData {
    /// What the file holds, counted as [`Self::adds`] counts: what it would
    /// add to an account without contact sets. The import window shows it,
    /// so a first import says the same number when it is done.
    pub fn count(&self) -> usize {
        self.adds(&ContactSets::default())
    }

    /// What a merge into `sets` would add: sets, members, residents kept
    /// outside the friends and nicknames (none: the file is already in).
    /// The one count of an import, before ([`Self::count`]) and after.
    pub fn adds(&self, sets: &ContactSets) -> usize {
        let members = self.0.sets.iter().map(|(name, set)| match sets.sets.get(name) {
            Some(ours) => set.friends.difference(&ours.friends).count(),
            None => 1 + set.friends.len(),
        });
        members.sum::<usize>()
            + self.0.extra.difference(&sets.extra).count()
            + self.0.pseudonyms.keys().filter(|id| !sets.pseudonyms.contains_key(*id)).count()
    }

    pub fn from_xml(bytes: &[u8]) -> Result<Self, &'static str> {
        Self::from_llsd(aurora_llsd::from_xml(bytes).map_err(|_| "invalid LLSD XML")?)
    }

    /// A Legacy Viewer's own contact sets file (the lists' import): an entry
    /// that `from_xml` would refuse is left out instead of the whole file
    /// (a set without a set's fields, a resident that is not an id, a
    /// nickname that is not text), and a colour that cannot be read takes
    /// the default. The data and the number of entries left out; an error
    /// when nothing in the file can be read.
    pub fn from_legacy_xml(bytes: &[u8]) -> Result<(Self, usize), &'static str> {
        let data = aurora_llsd::from_xml(bytes).map_err(|_| "invalid LLSD XML")?;
        let map = data.as_map().ok_or("expected a contact sets map")?;
        if map.is_empty() {
            return Err(EMPTY_MAP);
        }
        let mut skipped = 0;
        let mut kept = aurora_llsd::Map::new();
        for (name, value) in map {
            let Some(fields) = value.as_map().filter(|_| !name.trim().is_empty()) else {
                skipped += 1;
                continue;
            };
            let mut fields = fields.clone();
            match name.as_str() {
                GLOBAL_SETTINGS => {
                    if fields.get("defaultColor").is_none_or(|c| validate_color(c).is_err()) {
                        fields.insert("defaultColor".into(), color_llsd(GREY));
                    }
                }
                EXTRA_AVS | PSEUDONYM => {
                    let before = fields.len();
                    fields.retain(|key, value| valid_resident(key, value, name == PSEUDONYM));
                    skipped += before - fields.len();
                }
                name if ContactSets::is_internal(name) => {
                    skipped += 1;
                    continue;
                }
                _ => {
                    if !["color", "friends", "notify"].iter().any(|k| fields.contains_key(*k)) {
                        skipped += 1;
                        continue;
                    }
                    if fields.get("color").is_some_and(|c| validate_color(c).is_err()) {
                        fields.remove("color");
                        skipped += 1;
                    }
                    if let Some(friends) = fields.get_mut("friends") {
                        match friends {
                            Llsd::Map(members) => {
                                let before = members.len();
                                members.retain(|key, value| valid_resident(key, value, false));
                                skipped += before - members.len();
                            }
                            _ => {
                                fields.remove("friends");
                                skipped += 1;
                            }
                        }
                    }
                }
            }
            kept.insert(name.clone(), Llsd::Map(fields));
        }
        if kept.is_empty() {
            return Err("no contact set could be read");
        }
        Ok((Self::from_llsd(Llsd::Map(kept))?, skipped))
    }

    fn from_llsd(mut data: Llsd) -> Result<Self, &'static str> {
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
            return Err(EMPTY_MAP);
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
        if !valid_resident(key, value, aliases) {
            return Err("invalid resident");
        }
    }
    Ok(())
}

/// A resident's entry: a real id, and text when it is a nickname.
fn valid_resident(key: &str, value: &Llsd, alias: bool) -> bool {
    Uuid::parse_str(key).is_ok_and(|id| !id.is_nil()) && (!alias || matches!(value, Llsd::String(_)))
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

/// An import whose file a save thread writes (saves.rs): the sets in use
/// change only once it is on the disk ([`ContactSets::finish_import`]).
pub struct Saving {
    next: ContactSets,
    data: ImportData,
    /// The generation of the sets the import was merged into.
    from: u64,
    count: usize,
    written: std::sync::mpsc::Receiver<std::io::Result<()>>,
}

/// How [`ContactSets::begin_import`] went.
pub enum Begun {
    /// Made at once (no file: the demo, the tests).
    Done(usize),
    Saving(Box<Saving>),
}

impl ContactSets {
    /// The sets with `data` merged in, and how many sets it held.
    fn merged(&self, data: ImportData, is_friend: impl Fn(&Uuid) -> bool) -> (ContactSets, usize) {
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
        next.generation += 1;
        next.rebuild_appearances();
        next.rebuild_chat_sections();
        (next, count)
    }

    /// Validate before this call. Publish the complete merged file before
    /// changing live state, so a failed save leaves both versions intact.
    /// Written here: for the demo and the tests; the window goes through
    /// [`Self::begin_import`].
    pub fn merge_import(&mut self, data: ImportData, is_friend: impl Fn(&Uuid) -> bool) -> std::io::Result<usize> {
        let (next, count) = self.merged(data, is_friend);
        if let Some(path) = &next.path {
            write_file(path, &aurora_llsd::to_xml(&next.export()))?;
        }
        *self = next;
        Ok(count)
    }

    /// [`Self::merge_import`] for the lists' import, which has to know at
    /// once whether it worked: the merged file is published in one step
    /// without a disk flush (a refused write changes nothing), then queued
    /// again behind any save of the file still under way or waiting
    /// (`queue_save`), flushed by the save thread, so an older save can
    /// never land after it and undo the import.
    pub fn merge_import_queued(&mut self, data: ImportData, is_friend: impl Fn(&Uuid) -> bool) -> std::io::Result<usize> {
        let (next, count) = self.merged(data, is_friend);
        if let Some(path) = &next.path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            crate::atomic_file::publish(path, &aurora_llsd::to_xml(&next.export()))?;
        }
        *self = next;
        self.queue_save();
        Ok(count)
    }

    /// [`Self::merge_import`] with the file written by a save thread: the
    /// sets in use stay as they are until [`Self::finish_import`] hears
    /// that the file is on the disk.
    pub fn begin_import(&mut self, data: ImportData, is_friend: impl Fn(&Uuid) -> bool) -> std::io::Result<Begun> {
        let Some(path) = self.path.clone() else {
            return self.merge_import(data, is_friend).map(Begun::Done);
        };
        let (next, count) = self.merged(data.clone(), is_friend);
        let xml = aurora_llsd::to_xml(&next.export());
        let (tx, written) = std::sync::mpsc::channel();
        crate::saves::replace(path.clone(), move || {
            let _ = tx.send(write_file(&path, &xml));
        });
        Ok(Begun::Saving(Box::new(Saving {
            next,
            data,
            from: self.generation,
            count,
            written,
        })))
    }

    /// The import of `saving`, once its file is written: the merged sets
    /// are taken and the count returned. A refused save changes nothing
    /// (its error is returned). When the sets changed while the file was
    /// written, or a newer save of the file took the import's place, the
    /// import is merged again into the sets as they are now and saved with
    /// them. `None` while the save thread has not answered.
    pub fn finish_import(&mut self, saving: &mut Option<Box<Saving>>, is_friend: impl Fn(&Uuid) -> bool) -> Option<std::io::Result<usize>> {
        let answer = match saving.as_ref()?.written.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Ok(answer) => Some(answer),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        let saving = *saving.take()?;
        match answer {
            Some(Err(e)) => Some(Err(e)),
            Some(Ok(())) if self.generation == saving.from => {
                *self = saving.next;
                Some(Ok(saving.count))
            }
            _ => {
                let (next, count) = self.merged(saving.data, is_friend);
                *self = next;
                self.queue_save();
                Some(Ok(count))
            }
        }
    }
}

/// The sets' file, whole (atomic_file.rs).
pub(super) fn write_file(path: &std::path::Path, xml: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::atomic_file::write(path, xml)
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
    fn adds_counts_what_a_merge_would_bring_and_nothing_once_it_is_in() {
        let mut cs = ContactSets::default();
        assert_eq!(parse(&exported("Business", id(1))).adds(&cs), 2, "the set and its member");
        cs.merge_import(parse(&exported("Business", id(1))), |_| false).expect("merge");
        assert_eq!(parse(&exported("Business", id(1))).adds(&cs), 0);
        assert_eq!(parse(&exported("Business", id(2))).adds(&cs), 1);
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
    fn a_legacy_file_leaves_out_its_bad_entries_and_keeps_the_rest() {
        let xml = format!(
            "<llsd><map>\
             <key>Work</key><map><key>color</key><array><real>2.5</real><real>0</real><real>0</real></array>\
             <key>friends</key><map><key>{good}</key><string></string><key>not-an-id</key><string></string>\
             <key>{nil}</key><string></string></map></map>\
             <key>Broken</key><string>not a set</string>\
             <key>Empty</key><map><key>other</key><integer>1</integer></map>\
             <key>{extra}</key><map><key>{good}</key><string></string><key>bad</key><string></string></map>\
             <key>{pseudo}</key><map><key>{good}</key><string>Nick</string><key>{other}</key><integer>3</integer></map>\
             </map></llsd>",
            good = id(1),
            other = id(2),
            nil = Uuid::nil(),
            extra = EXTRA_AVS,
            pseudo = PSEUDONYM,
        );
        assert!(ImportData::from_xml(xml.as_bytes()).is_err(), "the window's import stays strict");
        let (data, skipped) = ImportData::from_legacy_xml(xml.as_bytes()).expect("the good entries");
        // the colour, two members, two sets, an extra resident, a nickname
        assert_eq!(skipped, 7);
        let mut cs = ContactSets::default();
        cs.merge_import(data, |_| true).expect("merge");
        assert_eq!(cs.set_names(), ["Work"]);
        let work = cs.set("Work").expect("set");
        assert_eq!(work.friends.iter().copied().collect::<Vec<_>>(), [id(1)]);
        assert_eq!(cs.pseudonym(&id(1)), Some("Nick"));
        assert_eq!(cs.pseudonym(&id(2)), None);
    }

    #[test]
    fn a_legacy_file_with_nothing_readable_is_an_error() {
        let xml = b"<llsd><map><key>Broken</key><string>x</string></map></llsd>";
        assert!(ImportData::from_legacy_xml(xml).is_err());
        assert_eq!(ImportData::from_legacy_xml(b"<llsd><map></map></llsd>").err(), Some(EMPTY_MAP));
    }

    #[test]
    fn a_queued_import_is_what_the_file_holds_once_the_saves_are_written() {
        let root = std::env::temp_dir().join(format!("aurora-contact-import-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let path = root.join(CONTACT_SETS_FILE);
        let mut cs = ContactSets::load(path.clone());
        // an older state still waiting for the save thread
        cs.add_set("Old");
        assert_eq!(cs.merge_import_queued(parse(&exported("Work", id(1))), |_| false).expect("save"), 1);
        crate::saves::wait_for(&path, crate::saves::BEFORE_READ);
        assert_eq!(ContactSets::load(path.clone()).export(), cs.export());
        assert!(cs.is_valid("Work") && cs.is_valid("Old"));

        cs.path = Some(root.clone()); // a directory cannot be replaced by a file
        let before = cs.export();
        assert!(cs.merge_import_queued(parse(&exported("Other", id(2))), |_| false).is_err());
        assert_eq!(cs.export(), before);
        std::fs::remove_dir_all(root).expect("cleanup");
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
