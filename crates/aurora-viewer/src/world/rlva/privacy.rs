//! Display-only identity and location filtering. Canonical IDs, name cache
//! records and region coordinates stay available to protocol handlers.
use super::super::{World, names::AvatarName};
use std::borrow::Cow;
use uuid::Uuid;
mod matcher;

pub fn anonymous(id: Uuid) -> &'static AvatarName {
    static NAMES: aurora_i18n::Localized<Vec<AvatarName>> = aurora_i18n::Localized::new();
    let names = NAMES.get(|| {
        (1..=32)
            .map(|number| {
                let label = aurora_i18n::tr!("rlva-anonymous", number = number.to_string()).to_owned();
                AvatarName {
                    username: label.clone(),
                    display_name: label.clone(),
                    legacy_first: label,
                    is_default: true,
                    ..Default::default()
                }
            })
            .collect()
    });
    let hash = id
        .as_bytes()
        .iter()
        .fold(0u8, |hash, byte| hash.wrapping_mul(31).wrapping_add(*byte));
    &names[hash as usize % names.len()]
}

#[derive(Default, Clone)]
pub struct TextFilter {
    replacements: Vec<(String, String)>,
    matcher: std::sync::OnceLock<std::sync::Arc<matcher::Matcher>>,
    private_names: bool,
    allowed_names: std::collections::HashSet<Uuid>,
}

impl TextFilter {
    pub fn redact_notice(&self, title: &mut String, body: &mut String, links: &mut [super::super::notification_history::NameLink]) {
        if !self.private_names && self.replacements.is_empty() {
            return;
        }
        let mut filter = self.clone();
        for link in links {
            if let super::super::notification_history::NameTarget::Avatar(id) = link.target
                && self.private_names
                && !self.allowed_names.contains(&id)
            {
                let replacement = &anonymous(id).display_name;
                if *title == link.name {
                    *title = replacement.clone();
                }
                let original = link.name.to_lowercase();
                if !filter
                    .replacements
                    .iter()
                    .any(|(name, value)| *name == original && value == replacement)
                {
                    filter.add(&link.name, replacement);
                }
                link.name = replacement.clone();
            } else if let Cow::Owned(text) = self.apply(&link.name) {
                link.name = text;
            }
        }
        filter.replacements.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
        if let Cow::Owned(text) = filter.apply(title) {
            *title = text;
        }
        if let Cow::Owned(text) = filter.apply(body) {
            *body = text;
        }
    }
    fn add(&mut self, original: &str, replacement: &str) {
        if original.chars().count() > 2 && !original.eq_ignore_ascii_case(replacement) {
            let needle = original.to_lowercase();
            self.replacements.push((needle, replacement.to_owned()));
            let _ = self.matcher.take();
        }
    }

    fn add_identity(&mut self, original: &str, replacement: &str) {
        self.add(original, replacement);
        // Visible Unicode URLs may escape only spaces, while canonical
        // application links escape every non-ASCII UTF-8 byte.
        self.add(&original.replace(' ', "%20"), replacement);
        self.add(&aurora_net::land::uri_escape(original), replacement);
    }

    fn reuse_matcher(&mut self, previous: &Self) {
        if self.replacements == previous.replacements
            && let Some(matcher) = previous.matcher.get()
        {
            let _ = self.matcher.set(matcher.clone());
        }
    }

    pub fn apply<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.replacements.is_empty() {
            return Cow::Borrowed(text);
        }
        let matcher = self
            .matcher
            .get_or_init(|| std::sync::Arc::new(matcher::Matcher::new(&self.replacements)));
        let mut out: Option<String> = None;
        let mut copied = 0;
        let mut offset = 0;
        while offset < text.len() {
            let remaining = &text[offset..];
            let boundary = offset == 0 || !text[..offset].chars().next_back().is_some_and(matcher::word);
            if let Some((count, index)) = boundary.then(|| matcher.at(remaining)).flatten() {
                let out = out.get_or_insert_with(|| String::with_capacity(text.len()));
                out.push_str(&text[copied..offset]);
                out.push_str(&self.replacements[index].1);
                offset += count;
                copied = offset;
            } else {
                let c = remaining.chars().next().expect("nonempty suffix");
                offset += c.len_utf8();
            }
        }
        match out {
            Some(mut out) => {
                out.push_str(&text[copied..]);
                Cow::Owned(out)
            }
            None => Cow::Borrowed(text),
        }
    }
}

impl World {
    pub fn refresh_rlva_privacy(&mut self) {
        self.rlva_refresh_location_history();
        let private = self.rlva.has("shownames");
        let mut allowed = std::collections::HashSet::new();
        if private {
            allowed.insert(self.agent_id);
            for (_, option) in self.rlva.rules_for("shownames", Uuid::nil()) {
                if let Ok(id) = Uuid::parse_str(option)
                    && self.rlva_can_name(id)
                {
                    allowed.insert(id);
                }
            }
        }
        self.social.avatar_names.set_rlva_privacy(private, allowed);
        let mut filter = self.rlva_text_filter(false);
        filter.reuse_matcher(&self.rlva_privacy);
        for notification in &mut self.notifications.list {
            filter.redact_notice(&mut notification.title, &mut notification.body, &mut notification.name_links);
        }
        self.rlva_privacy = filter;
        self.rlva_hover_privacy = if self.rlva.has("shownearby") {
            let mut filter = self.rlva_text_filter(true);
            filter.reuse_matcher(&self.rlva_hover_privacy);
            filter
        } else {
            self.rlva_privacy.clone()
        };
    }

    pub fn rlva_can_name(&self, id: Uuid) -> bool {
        !self.rlva.enabled() || id == self.agent_id || self.rlva.allows("shownames", &id.to_string())
    }

    pub fn rlva_private_region(&self, region: &str) -> bool {
        self.rlva.has("showloc")
            && self
                .regions
                .values()
                .any(|r| !r.name.is_empty() && r.name.eq_ignore_ascii_case(region))
    }

    /// LLAvatarActions::canOfferTeleport permits a location reveal only to
    /// a permissive tplure exception or an online friend granted map rights.
    pub fn rlva_can_offer_teleport(&self, id: Uuid) -> bool {
        !self.rlva.has("showloc")
            || self
                .rlva
                .rules_for("tplure", Uuid::nil())
                .any(|(_, option)| Uuid::parse_str(option).ok() == Some(id))
            || self
                .social
                .friends
                .iter()
                .any(|f| f.id == id && f.online && f.rights_given & super::super::social::rights::MAP_LOCATION != 0)
    }

    /// LLAvatarActions::offerTeleport filters the recipients separately, so
    /// an ineligible resident does not cancel invitations to eligible ones.
    pub fn rlva_filter_teleport_offer(&self, recipients: &mut Vec<Uuid>) {
        if self.rlva.has("showloc") {
            recipients.retain(|id| self.rlva_can_offer_teleport(*id));
        }
    }

    fn rlva_refresh_location_history(&mut self) {
        let hidden = self.rlva.has("showloc");
        if hidden == self.rlva_location_hidden {
            return;
        }
        self.rlva_location_hidden = hidden;
        if hidden {
            if let Some(current) = self.tp_history.replace_current_region(aurora_i18n::tr!("rlva-hidden-region")) {
                let global = glam::DVec3::new(
                    (current.handle >> 32) as f64 + f64::from(current.position.x),
                    f64::from(current.handle as u32) + f64::from(current.position.y),
                    f64::from(current.position.z),
                );
                if let Some(last) = self.tp_storage.items.last()
                    && last.title.ends_with(&current.region)
                    && last.global.distance(global) < 5.0
                {
                    self.tp_storage.remove(self.tp_storage.items.len() - 1);
                }
            }
        } else {
            let _ = self.tp_history.replace_current_region(&self.region_name());
            if let Some(global) = self.agent_global() {
                self.tp_record = Some((global, std::time::Instant::now()));
                self.record_arrival();
            }
        }
    }

    /// RlvUIEnabler permits land/estate administration under showloc only
    /// through actual ownership or group powers, without a god-mode shortcut.
    pub fn rlva_can_view_parcel(&self) -> bool {
        if !self.rlva.has("showloc") {
            return true;
        }
        let parcel = match &self.land.sel {
            Some(s) => s.parcel.as_ref(),
            None => self.parcel.as_ref(),
        };
        parcel.is_some_and(|p| {
            p.owner_id == self.agent_id
                || self
                    .groups
                    .groups
                    .iter()
                    .any(|g| g.id == p.owner_id && g.powers & ((1 << 33) | (1 << 34) | (1 << 48)) != 0)
        })
    }

    pub fn rlva_can_view_region(&self) -> bool {
        !self.rlva.has("showloc")
            || self
                .main()
                .and_then(|r| r.info.as_ref())
                .is_some_and(|r| r.owner == self.agent_id || r.is_estate_manager)
    }

    pub fn rlva_region_label(&self) -> String {
        if self.rlva.has("showloc") {
            aurora_i18n::tr!("rlva-hidden-region").to_owned()
        } else {
            self.region_name()
        }
    }

    pub fn rlva_parcel_label(&self) -> String {
        if self.rlva.has("showloc") {
            aurora_i18n::tr!("rlva-hidden-parcel").to_owned()
        } else {
            self.parcel_name.clone()
        }
    }

    pub fn rlva_text_filter(&self, clear_nearby: bool) -> TextFilter {
        let mut filter = TextFilter::default();
        if !self.rlva.enabled() {
            return filter;
        }
        filter.private_names = self.rlva.has("shownames");
        if filter.private_names {
            filter.allowed_names.insert(self.agent_id);
            for (_, option) in self.rlva.rules_for("shownames", Uuid::nil()) {
                if let Ok(id) = Uuid::parse_str(option)
                    && self.rlva_can_name(id)
                {
                    filter.allowed_names.insert(id);
                }
            }
        }
        if self.rlva.has("shownames") || clear_nearby {
            for sighting in super::super::radar::sightings(self) {
                let id = sighting.id;
                if id == self.agent_id || (!clear_nearby && self.rlva_can_name(id)) {
                    continue;
                }
                self.social.avatar_names.want(&id);
                let replacement = if clear_nearby {
                    String::new()
                } else {
                    anonymous(id).display_name.clone()
                };
                if let Some(name) = self.social.names.get(&id) {
                    filter.add_identity(name, &replacement);
                }
                if let Some(name) = self
                    .objects
                    .index_of_uuid(&id)
                    .and_then(|i| self.objects.get(i))
                    .and_then(|o| o.display_name())
                {
                    filter.add_identity(&name, &replacement);
                }
                if let Some(name) = self.social.avatar_names.raw(&id) {
                    filter.add_identity(&name.display_name, &replacement);
                    filter.add_identity(&name.user_name(), &replacement);
                    filter.add_identity(&name.username, &replacement);
                }
            }
        }
        if self.rlva.has("showloc") {
            for region in self.regions.values() {
                filter.add(&region.name, aurora_i18n::tr!("rlva-hidden-region"));
                filter.add(&aurora_net::land::uri_escape(&region.name), aurora_i18n::tr!("rlva-hidden-region"));
            }
            filter.add(&self.parcel_name, aurora_i18n::tr!("rlva-hidden-parcel"));
        }
        filter
            .replacements
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)).then_with(|| a.1.cmp(&b.1)));
        filter.replacements.dedup();
        filter
    }

    pub fn rlva_filter_text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.rlva.enabled() {
            self.rlva_privacy.apply(text)
        } else {
            Cow::Borrowed(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restrict(world: &mut World, commands: &str) {
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Privacy fixture".into(),
            source_id: Uuid::from_u128(1),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: format!("@{commands}"),
        }));
    }

    #[test]
    fn rlva_privacy_matcher_keeps_longest_unicode_matches_and_reuses_unchanged_patterns() {
        let mut filter = TextFilter::default();
        filter.add("Secret Region", "hidden");
        filter.add("Secret Region Annex", "annex");
        filter.add_identity("Élodie Meadows", "anonymous");
        assert_eq!(
            filter.apply("SECRET REGION ANNEX / Secret Region / ÉLODIE MEADOWS"),
            "annex / hidden / anonymous"
        );
        assert_eq!(filter.apply("Élodie%20Meadows"), "anonymous");
        assert_eq!(filter.apply("%C3%89lodie%20Meadows"), "anonymous");
        assert!(matches!(filter.apply("Secret Regional and Élodie Meadowson"), Cow::Borrowed(_)));
        let mut same = TextFilter::default();
        same.replacements.clone_from(&filter.replacements);
        same.reuse_matcher(&filter);
        assert!(std::sync::Arc::ptr_eq(filter.matcher.get().unwrap(), same.matcher.get().unwrap()));
        same.add("Another Place", "elsewhere");
        assert_eq!(same.apply("Another Place"), "elsewhere");
        assert!(!std::sync::Arc::ptr_eq(filter.matcher.get().unwrap(), same.matcher.get().unwrap()));
    }

    #[test]
    fn rlva_location_history_keeps_destinations_and_teleport_offers_require_reference_exceptions() {
        use crate::world::tphistory::{HistoryItem, HistoryStorage, TeleportHistory, TpEntry};
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let handle = world.main_region.unwrap();
        let name = world.region_name();
        let global = world.agent_global().unwrap();
        world.tp_history = TeleportHistory::default();
        world.tp_history.arrived(TpEntry {
            region: "Older destination".into(),
            handle,
            position: glam::Vec3::ONE,
            parcel: String::new(),
            at: 0,
        });
        world.tp_history.arrived(TpEntry {
            region: name.clone(),
            handle,
            position: world.agent.position,
            parcel: String::new(),
            at: 0,
        });
        let old = HistoryItem {
            title: "Older destination".into(),
            global: global + glam::DVec3::X * 100.0,
            date: 1.0,
            slurl: String::new(),
        };
        world.tp_storage = HistoryStorage::in_memory(vec![
            old.clone(),
            HistoryItem {
                title: name,
                global,
                date: 2.0,
                slurl: String::new(),
            },
        ]);
        world.rlva.configure(true);
        restrict(&mut world, "showloc=n");
        assert_eq!(world.tp_storage.items, vec![old]);
        assert_eq!(world.tp_history.previous().unwrap().position, glam::Vec3::ONE);
        let friend = Uuid::from_u128(99);
        assert!(!world.rlva_can_offer_teleport(friend));
        world.social.friends.push(crate::world::social::Friend {
            id: friend,
            online: true,
            rights_given: 2,
            rights_has: 0,
        });
        assert!(world.rlva_can_offer_teleport(friend));
        world.social.friends.last_mut().unwrap().online = false;
        assert!(!world.rlva_can_offer_teleport(friend));
        restrict(&mut world, &format!("tplure:{friend}=n"));
        assert!(world.rlva_can_offer_teleport(friend), "offers use permissive tplure exceptions");
        let mut recipients = vec![friend, Uuid::from_u128(100)];
        world.rlva_filter_teleport_offer(&mut recipients);
        assert_eq!(recipients, vec![friend]);
        restrict(&mut world, "showloc=y");
        assert_eq!(world.tp_storage.items.len(), 2);
        assert_eq!(world.tp_storage.items.last().unwrap().global, global);
    }

    #[test]
    fn rlva_display_names_never_replace_canonical_cache_records_or_identity_lookups() {
        let mut names = crate::world::names::AvatarNames::default();
        let id = Uuid::from_u128(42);
        names.on_legacy(id, "Élodie", "Meadows");
        let canonical = names.get(&id).unwrap().display_name.clone();
        names.set_rlva_privacy(true, std::collections::HashSet::new());
        assert_ne!(names.get(&id).unwrap().display_name, canonical);
        assert_eq!(names.raw(&id).unwrap().display_name, canonical);
        assert_eq!(names.find_id_by_name("Élodie Meadows"), Some(id));
        let mut allowed = std::collections::HashSet::new();
        allowed.insert(id);
        names.set_rlva_privacy(true, allowed);
        assert_eq!(names.get(&id).unwrap().display_name, canonical);
        names.set_rlva_privacy(false, std::collections::HashSet::new());
        assert_eq!(names.get(&id).unwrap().display_name, canonical);
    }

    #[test]
    fn rlva_privacy_filters_unicode_and_word_boundaries_without_changing_world_coordinates() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let id = world
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != world.agent_id)
            .map(|(_, o)| o.full_id)
            .unwrap();
        world.social.names.insert(id, "Élodie Meadows".into());
        world.social.avatar_names.on_legacy(id, "Élodie", "Meadows");
        let handle = world.main_region.unwrap();
        world.regions.get_mut(&handle).unwrap().name = "Secret Region".into();
        world.parcel_name = "Secret Parcel".into();
        let position = world.agent.position;
        world.rlva.configure(true);
        restrict(&mut world, "shownames=n,showloc=n");
        world.refresh_rlva_privacy();
        let text = "ÉLODIE MEADOWS visits Secret Region / Secret Parcel. Élodie Meadowson stays.";
        let filtered = world.rlva_filter_text(text);
        assert!(!filtered.contains("ÉLODIE MEADOWS"));
        assert!(!filtered.contains("Secret Region") && !filtered.contains("Secret Parcel"));
        assert!(filtered.contains("Élodie Meadowson"), "only full names match");
        assert!(
            !world
                .rlva_filter_text("secondlife://Secret%20Region/128/128/30")
                .contains("Secret%20Region")
        );
        assert_eq!(world.agent.position, position);
        assert_eq!(world.regions[&handle].name, "Secret Region");
        assert_eq!(world.social.avatar_names.raw(&id).unwrap().user_name(), "Élodie Meadows");
        assert_eq!(world.rlva_text_filter(true).apply("Élodie Meadows waves").as_ref(), " waves");
        world.rlva.configure(false);
        world.refresh_rlva_privacy();
        assert_eq!(world.region_name(), "Secret Region");
        assert_eq!(world.social.name_of(&id), "Élodie Meadows");
        assert!(matches!(world.rlva_filter_text(text), Cow::Borrowed(_)));
    }
}
