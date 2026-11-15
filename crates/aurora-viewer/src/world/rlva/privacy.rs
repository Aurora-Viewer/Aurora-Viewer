//! Display-only identity and location filtering. Canonical IDs, name cache
//! records and region coordinates stay available to protocol handlers.
use super::super::{
    World,
    contact_sets::Appearance,
    names::{AvatarName, AvatarNames},
};
use std::borrow::Cow;
use uuid::Uuid;
mod matcher;

/// The stand-ins of a hidden name, by their message: the `anonyms` list of
/// the Legacy Viewer's rlva_strings.xml, in its order, which
/// RlvStrings::getAnonym indexes.
const ANONYMS: [&str; 28] = [
    "rlva-anonym-a-resident",
    "rlva-anonym-this-resident",
    "rlva-anonym-that-resident",
    "rlva-anonym-an-individual",
    "rlva-anonym-this-individual",
    "rlva-anonym-that-individual",
    "rlva-anonym-a-person",
    "rlva-anonym-this-person",
    "rlva-anonym-that-person",
    "rlva-anonym-a-stranger",
    "rlva-anonym-this-stranger",
    "rlva-anonym-that-stranger",
    "rlva-anonym-a-being",
    "rlva-anonym-this-being",
    "rlva-anonym-that-being",
    "rlva-anonym-an-agent",
    "rlva-anonym-this-agent",
    "rlva-anonym-that-agent",
    "rlva-anonym-a-soul",
    "rlva-anonym-this-soul",
    "rlva-anonym-that-soul",
    "rlva-anonym-somebody",
    "rlva-anonym-some-people",
    "rlva-anonym-someone",
    "rlva-anonym-mysterious-one",
    "rlva-anonym-an-unknown-being",
    "rlva-anonym-unidentified-one",
    "rlva-anonym-an-unknown-person",
];

/// RlvStrings::getAnonym (rlvcommon.cpp): the bytes of the name summed in a
/// U32 as the signed `char` they are there, so that a name outside ASCII
/// lands on the same stand-in, modulo the list.
pub fn anonym_index(name: &str) -> usize {
    let hash = name.bytes().fold(0u32, |hash, byte| hash.wrapping_add(byte as i8 as u32));
    hash as usize % ANONYMS.len()
}

/// The stand-in of the resident whose legacy name ("First Last", as
/// LLAvatarName::getLegacyName) is `name`: the same one as in the Legacy
/// Viewer, so that two viewers side by side tell the same strangers apart.
pub fn anonymous_named(name: &str) -> &'static AvatarName {
    static NAMES: aurora_i18n::Localized<Vec<AvatarName>> = aurora_i18n::Localized::new();
    let names = NAMES.get(|| {
        ANONYMS
            .iter()
            .map(|key| {
                let label = aurora_i18n::text(key);
                AvatarName {
                    username: label.to_owned(),
                    display_name: label.to_owned(),
                    legacy_first: label.to_owned(),
                    is_default: true,
                    ..Default::default()
                }
            })
            .collect()
    });
    &names[anonym_index(name)]
}

/// The stand-in of a resident whose name is not known yet: by the id as
/// text, which is what RlvUtil::filterMentions (rlvcommon.cpp) hashes for
/// a name missing from the cache. With a name cache at hand, ask
/// `World::rlva_anonymous` instead: it follows the legacy name.
pub fn anonymous(id: Uuid) -> &'static AvatarName {
    anonymous_named(&id.to_string())
}

/// What a resident outside every contact set looks like: the one shown for
/// a resident whose name is hidden.
static PLAIN_APPEARANCE: std::sync::LazyLock<std::sync::Arc<Appearance>> = std::sync::LazyLock::new(Default::default);

#[derive(Default, Clone)]
pub struct TextFilter {
    replacements: Vec<(String, String)>,
    matcher: std::sync::OnceLock<std::sync::Arc<matcher::Matcher>>,
    private_names: bool,
    allowed_names: std::collections::HashSet<Uuid>,
}

impl TextFilter {
    /// `names`: the name cache, for the stand-in of each hidden resident.
    pub fn redact_notice(
        &self,
        names: &AvatarNames,
        title: &mut String,
        body: &mut String,
        links: &mut [super::super::notification_history::NameLink],
    ) {
        if !self.private_names && self.replacements.is_empty() {
            return;
        }
        let mut filter = self.clone();
        for link in links {
            if let super::super::notification_history::NameTarget::Avatar(id) = link.target
                && self.private_names
                && !self.allowed_names.contains(&id)
            {
                let replacement = &names.anonymous(&id).display_name;
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
            filter.redact_notice(
                &self.social.avatar_names,
                &mut notification.title,
                &mut notification.body,
                &mut notification.name_links,
            );
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
        // asked for every tag, dot and chat line of a frame: the id is
        // written on the stack
        !self.rlva.enabled()
            || id == self.agent_id
            || self
                .rlva
                .allows("shownames", id.hyphenated().encode_lower(&mut Uuid::encode_buffer()))
    }

    /// The stand-in shown for a resident whose name is hidden
    /// (RlvStrings::getAnonym over the cached name, rlvcommon.h). The name
    /// is asked for like LLAvatarNameCache::get does there: the stand-in
    /// follows it, and is the id's own until it comes.
    pub fn rlva_anonymous(&self, id: Uuid) -> &'static AvatarName {
        let names = &self.social.avatar_names;
        names.want(&id);
        names.anonymous(&id)
    }

    /// The contact-set appearance to show for `id`: none for a resident
    /// whose name is hidden, whose sets would tell who they are
    /// (LGGContactSets::hasFriendColorThatShouldShow, lggcontactsets.cpp).
    /// The settings windows that edit the sets read `contact_sets` itself.
    pub fn rlva_contact_appearance(&self, id: &Uuid) -> &std::sync::Arc<Appearance> {
        if self.rlva_can_name(*id) {
            self.contact_sets.appearance(id)
        } else {
            &PLAIN_APPEARANCE
        }
    }

    /// A friend shown as one: never a resident whose name is hidden
    /// (`is_friend = fRlvShowAvName && isBuddy()` in
    /// LLVOAvatar::idleUpdateNameTagText, llvoavatar.cpp; the same test in
    /// LGGContactSets::colorize for the chat, the tags and the mini-map).
    pub fn rlva_shown_friend(&self, id: &Uuid) -> bool {
        self.rlva_can_name(*id) && self.social.is_friend(id)
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
                    self.social.avatar_names.anonymous(&id).display_name.clone()
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
            region: None,
            position: glam::Vec3::ZERO,
            message: format!("@{commands}"),
        }));
    }

    /// The stand-in `index` in any language: another test may change the
    /// interface language while this one runs.
    fn is_anonym(text: &str, index: usize) -> bool {
        ["en", "fr", "ja"]
            .iter()
            .any(|language| aurora_i18n::text_in(language, ANONYMS[index]) == text)
    }

    #[test]
    fn rlva_anonyms_are_the_legacy_list_picked_by_the_sum_of_the_name() {
        let english: Vec<&str> = ANONYMS.iter().map(|key| aurora_i18n::text_in("en", key)).collect();
        assert_eq!(
            english,
            [
                "A resident",
                "This resident",
                "That resident",
                "An individual",
                "This individual",
                "That individual",
                "A person",
                "This person",
                "That person",
                "A stranger",
                "This stranger",
                "That stranger",
                "A being",
                "This being",
                "That being",
                "An agent",
                "This agent",
                "That agent",
                "A soul",
                "This soul",
                "That soul",
                "Somebody",
                "Some people",
                "Someone",
                "Mysterious one",
                "An unknown being",
                "Unidentified one",
                "An unknown person",
            ]
        );
        for language in ["fr", "ja"] {
            let translated: std::collections::HashSet<&str> = ANONYMS.iter().map(|key| aurora_i18n::text_in(language, key)).collect();
            assert_eq!(translated.len(), ANONYMS.len(), "{language}: two strangers must not read the same");
            assert!(ANONYMS.iter().all(|key| !translated.contains(key)), "{language}: a missing message");
        }
        assert_eq!(anonym_index(""), 0, "a name not known yet is \"A resident\" there");
        assert_eq!(anonym_index("A"), 65 % 28);
        // 74 + 97 + 110 + 101 + 32 + 68 + 111 + 101 = 694
        assert_eq!(anonym_index("Jane Doe"), 694 % 28);
        assert_eq!(anonym_index("eoD enaJ"), anonym_index("Jane Doe"), "a sum, whatever the order");
        // a signed char: 0xC3 0x89 is -61 - 119, which wraps in the U32
        assert_eq!(anonym_index("É"), (-180i32 as u32 % 28) as usize);
        assert_eq!(anonym_index("É"), 20);
        let id = Uuid::from_u128(42);
        assert!(is_anonym(&anonymous(id).display_name, anonym_index(&id.to_string())));
    }

    #[test]
    fn rlva_hidden_names_use_the_stand_in_of_their_legacy_name_everywhere() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let agent = world.agent_id;
        let id = world.objects.avatars().map(|(_, o)| o.full_id).find(|id| *id != agent).unwrap();
        // not in the scene: the stand-in only needs the cached name
        let resident = Uuid::from_u128(0x50f0);
        let cache = |world: &mut World, id: Uuid, first: &str, last: &str| {
            let name = aurora_net::AvatarNameData {
                id,
                username: format!("{first}.{last}").to_lowercase(),
                display_name: format!("{first} {last}"),
                legacy_first: first.into(),
                legacy_last: last.into(),
                ..Default::default()
            };
            world.social.avatar_names.on_display_names(&[name], &[], None);
        };
        cache(&mut world, id, "Jane", "Doe");
        // LLAvatarName::getLegacyName keeps the last name of a "Resident"
        cache(&mut world, resident, "Solo", "Resident");
        world.rlva.configure(true);
        restrict(&mut world, "shownames=n");
        world.refresh_rlva_privacy();
        let jane = anonym_index("Jane Doe");
        assert_ne!(jane, anonym_index("Jane"));
        assert!(is_anonym(&world.social.avatar_names.get(&id).unwrap().display_name, jane));
        assert!(is_anonym(&world.rlva_anonymous(id).display_name, jane));
        assert!(is_anonym(&world.avatar_name(&id).unwrap(), jane));
        let filtered = world.rlva_filter_text("Jane Doe waves");
        assert!(
            is_anonym(filtered.strip_suffix(" waves").expect("the rest is kept"), jane),
            "{filtered}"
        );
        let solo = anonym_index("Solo Resident");
        assert_ne!(solo, anonym_index("Solo"));
        assert!(is_anonym(&world.rlva_anonymous(resident).display_name, solo));
        // a notification's link: by the cached name, not by what the link shows
        let mut title = String::from("Janey");
        let mut body = String::from("Janey sent a message.");
        let mut links = [crate::world::notification_history::NameLink {
            name: "Janey".into(),
            target: crate::world::notification_history::NameTarget::Avatar(id),
        }];
        world
            .rlva_privacy
            .redact_notice(&world.social.avatar_names, &mut title, &mut body, &mut links);
        assert!(is_anonym(&title, jane) && is_anonym(&links[0].name, jane), "{title}");
        assert!(!body.contains("Janey"));
        // not cached: by the id, until the name comes
        let unknown = Uuid::from_u128(0xdead);
        assert!(is_anonym(
            &world.rlva_anonymous(unknown).display_name,
            anonym_index(&unknown.to_string())
        ));
        cache(&mut world, unknown, "Jane", "Doe");
        assert!(is_anonym(&world.rlva_anonymous(unknown).display_name, jane));
    }

    #[test]
    fn rlva_hidden_names_have_neither_contact_set_nor_friendship_to_show() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let friend = Uuid::from_u128(0xf00d);
        world.social.friends.push(crate::world::social::Friend {
            id: friend,
            online: true,
            rights_given: 0,
            rights_has: 0,
        });
        let sets = &mut world.contact_sets;
        sets.add_set("Team");
        sets.add_to_set(&[friend], "Team", |_| false);
        sets.set_color("Team", [0.6, 0.2, 0.8, 1.0]);
        sets.set_visuals(
            "Team",
            crate::world::contact_sets::Visuals {
                border: true,
                minimap: true,
                ..Default::default()
            },
        );
        world.rlva.configure(true);
        let shown = world.contact_sets.appearance(&friend).clone();
        assert_ne!(*shown, Appearance::default());
        assert_eq!(world.rlva_contact_appearance(&friend), &shown);
        assert!(world.rlva_shown_friend(&friend));
        restrict(&mut world, "shownames=n");
        assert_eq!(**world.rlva_contact_appearance(&friend), Appearance::default());
        assert!(!world.rlva_shown_friend(&friend));
        assert_eq!(world.contact_sets.appearance(&friend), &shown, "the set itself is untouched");
        restrict(&mut world, &format!("shownames:{friend}=add"));
        assert_eq!(world.rlva_contact_appearance(&friend), &shown, "an exception is named again");
        assert!(world.rlva_shown_friend(&friend));
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
            grid: None,
        });
        world.tp_history.arrived(TpEntry {
            region: name.clone(),
            handle,
            position: world.agent.position,
            parcel: String::new(),
            at: 0,
            grid: None,
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
