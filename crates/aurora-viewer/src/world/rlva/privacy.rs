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

    /// Longest first, so that a name is replaced before a part of it.
    fn sort(&mut self) {
        self.replacements
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)).then_with(|| a.1.cmp(&b.1)));
        self.replacements.dedup();
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
        let nearby = if private { self.rlva_nearby_avatars() } else { Default::default() };
        self.social.avatar_names.set_rlva_nearby(nearby);
        let mut filter = self.rlva_text_filter(false);
        filter.reuse_matcher(&self.rlva_privacy);
        let mut places = TextFilter::default();
        self.rlva_hide_places(&mut places);
        places.sort();
        places.reuse_matcher(&self.rlva_place_privacy);
        for notification in &mut self.notifications.list {
            let filter = if notification.kind.names() == NameScope::Roster {
                &places
            } else {
                &filter
            };
            filter.redact_notice(
                &self.social.avatar_names,
                &mut notification.title,
                &mut notification.body,
                &mut notification.name_links,
            );
        }
        self.rlva_privacy = filter;
        self.rlva_place_privacy = places;
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

    /// The mark of a resident (`world::avatar_marks`) as the world, the maps
    /// and the lists may show it: none for a resident whose name is hidden,
    /// whose mark would tell who they are. LLNetMap::draw (llnetmap.cpp)
    /// drops it from the dot (`canShowName(…) ? color : map_avatar_color`);
    /// stricter than the Legacy Viewer on the name tag, which keeps the
    /// mark's colour there (LLVOAvatar::getNameTagColor).
    pub fn rlva_shown_mark(&self, id: &Uuid) -> Option<super::super::avatar_marks::Rgb> {
        self.avatar_marks.get(id).filter(|_| self.rlva_can_name(*id))
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
        self.rlva_hide_places(&mut filter);
        filter.sort();
        filter
    }

    /// What `@showloc` hides of a text: the regions around and the parcel.
    fn rlva_hide_places(&self, filter: &mut TextFilter) {
        if self.rlva.has("showloc") {
            for region in self.regions.values() {
                filter.add(&region.name, aurora_i18n::tr!("rlva-hidden-region"));
                filter.add(&aurora_net::land::uri_escape(&region.name), aurora_i18n::tr!("rlva-hidden-region"));
            }
            filter.add(&self.parcel_name, aurora_i18n::tr!("rlva-hidden-parcel"));
        }
    }

    pub fn rlva_filter_text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        self.rlva_filter_text_in(NameScope::World, text)
    }

    /// `rlva_filter_text` for a text shown in `scope`: a place that names
    /// its residents (`NameScope::Roster`: an IM session, RlvUtil::filterNames
    /// is never run on its lines) keeps the names of the people around, and
    /// hides the regions and the parcel like the others.
    pub fn rlva_filter_text_in<'a>(&self, scope: NameScope, text: &'a str) -> Cow<'a, str> {
        if self.rlva.enabled() {
            self.rlva_filter_in(scope).apply(text)
        } else {
            Cow::Borrowed(text)
        }
    }

    /// The filter of the texts shown in `scope`: the places alone where the
    /// residents are named (`NameScope::Roster`; a notice's scope is
    /// `notifications::Kind::names`).
    pub fn rlva_filter_in(&self, scope: NameScope) -> &TextFilter {
        if scope == NameScope::Roster {
            &self.rlva_place_privacy
        } else {
            &self.rlva_privacy
        }
    }
}

pub use super::super::names::NameScope;

/// `@shownames` by the place a name is shown in (issue #3547): the Legacy
/// Viewer hides the residents of the world around and the owners and
/// creators its object windows name, and leaves the lists of people known
/// otherwise alone.
impl World {
    /// The window or list `scope` shows the real name of `id`: the one rule
    /// behind every scoped lookup (`NameScope::shows`, world/names.rs), as
    /// `refresh_rlva_privacy` last fed it. What gives a name
    /// (`AvatarNames::get_in`, `Social::name_in`) follows it by itself; ask
    /// it for what goes with a shown name, such as opening a profile.
    pub fn rlva_shows_name(&self, scope: NameScope, id: Uuid) -> bool {
        self.social.avatar_names.shows(scope, &id)
    }

    /// The avatars of every region around, oneself left out
    /// (RlvUtil::isNearbyAgent over LLWorld::getAvatars, rlvcommon.cpp):
    /// the loaded ones and the coarse locations, whatever their distance.
    fn rlva_nearby_avatars(&self) -> std::collections::HashSet<Uuid> {
        let loaded = self.objects.avatars().map(|(_, o)| o.full_id).filter(|id| !self.is_mirror(id));
        let coarse = self.coarse.values().flatten().map(|(id, _)| *id);
        loaded.chain(coarse).filter(|id| !id.is_nil() && *id != self.agent_id).collect()
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

    // ---- `@shownames` by the place a name is shown in (issue #3547)

    const FRIEND: Uuid = Uuid::from_u128(0xF1);
    const MEMBER: Uuid = Uuid::from_u128(0xF2);
    const MAKER: Uuid = Uuid::from_u128(0xF3);

    /// The demo's world under `@shownames=n`: an avatar around us, and a
    /// friend, a group member and a creator who are in no region around.
    fn shownames_world() -> (World, Uuid) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let near = world
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != world.agent_id)
            .map(|(_, o)| o.full_id)
            .unwrap();
        for (id, first) in [(near, "Near"), (FRIEND, "Friend"), (MEMBER, "Member"), (MAKER, "Maker")] {
            world.social.avatar_names.on_legacy(id, first, "Tester");
        }
        world.social.friends.push(crate::world::social::Friend {
            id: FRIEND,
            online: true,
            rights_given: 0,
            rights_has: 0,
        });
        world.rlva.configure(true);
        restrict(&mut world, "shownames=n");
        world.refresh_rlva_privacy();
        (world, near)
    }

    fn named(world: &World, scope: NameScope, id: Uuid) -> bool {
        let shown = world.rlva_shows_name(scope, id);
        // the name read in that place follows the rule, in every form
        let names = &world.social.avatar_names;
        assert_eq!(world.social.name_in(scope, &id).contains("Tester"), shown);
        assert_eq!(world.social.display_name_in(scope, &id).contains("Tester"), shown);
        assert_eq!(names.get_in(scope, &id).unwrap().user_name().contains("Tester"), shown);
        assert_eq!(names.complete_in(scope, &id).unwrap().contains("Tester"), shown);
        shown
    }

    #[test]
    fn shownames_rule_by_scope_exception_and_presence() {
        let id = Uuid::from_u128(7);
        let other = Uuid::from_u128(8);
        let creator = |owner| NameScope::Creator { owner };
        for nearby in [false, true] {
            // an exception (or oneself) is named everywhere
            for scope in [NameScope::World, NameScope::Owner, creator(id), creator(other), NameScope::Roster] {
                assert!(scope.shows(&id, true, || nearby), "{scope:?}");
            }
            assert!(!NameScope::World.shows(&id, false, || nearby));
            assert!(!NameScope::Owner.shows(&id, false, || nearby));
            assert!(NameScope::Roster.shows(&id, false, || nearby));
            // the creator who owns the object is its owner
            assert!(!creator(id).shows(&id, false, || nearby));
            assert_eq!(creator(other).shows(&id, false, || nearby), !nearby);
            assert_eq!(creator(Uuid::nil()).shows(&id, false, || nearby), !nearby);
        }
    }

    #[test]
    fn shownames_hides_an_avatar_around_us_in_the_world() {
        let (mut world, near) = shownames_world();
        assert!(!named(&world, NameScope::World, near));
        // a lookup naming no place is the world's
        assert!(!world.social.name_of(&near).contains("Tester"));
        assert!(!world.social.avatar_names.get(&near).unwrap().display_name.contains("Tester"));
        // strict default: so is someone the world names while they are away
        assert!(!named(&world, NameScope::World, FRIEND));
        assert!(!named(&world, NameScope::World, MEMBER));
        // ourselves, always
        let me = world.agent_id;
        assert!(world.rlva_shows_name(NameScope::World, me));
        // nothing is hidden without the restriction, nor without RLVa
        restrict(&mut world, "shownames=y");
        world.refresh_rlva_privacy();
        assert!(named(&world, NameScope::World, near));
        restrict(&mut world, "shownames=n");
        world.refresh_rlva_privacy();
        assert!(!named(&world, NameScope::World, near));
        world.rlva.configure(false);
        world.refresh_rlva_privacy();
        assert!(named(&world, NameScope::World, near));
        assert!(named(&world, NameScope::Owner, near));
    }

    #[test]
    fn shownames_leaves_a_friend_who_is_not_around_named_in_the_friends_list() {
        let (world, _) = shownames_world();
        assert!(named(&world, NameScope::Roster, FRIEND));
        assert_eq!(world.social.name_in(NameScope::Roster, &FRIEND), "Friend Tester");
        // the list alone: the same friend stays anonymous anywhere else
        assert!(!named(&world, NameScope::World, FRIEND));
        assert!(!named(&world, NameScope::Owner, FRIEND));
    }

    #[test]
    fn shownames_leaves_a_friend_around_us_named_in_the_list_and_untold_in_the_world() {
        let (mut world, near) = shownames_world();
        world.social.friends.push(crate::world::social::Friend {
            id: near,
            online: true,
            rights_given: 0,
            rights_has: 0,
        });
        let handle = world.main_region.unwrap();
        world.coarse.entry(handle).or_default().push((near, glam::Vec3::ONE));
        world.refresh_rlva_privacy();
        assert!(named(&world, NameScope::Roster, near));
        assert!(!named(&world, NameScope::World, near));
        // the row tells nothing of where they stand: around us they would
        // read online, the list knows no status of theirs
        use crate::ui::presence::{self, Status};
        let told = presence::of(&world, &near);
        assert_ne!(told, Status::Unknown);
        assert_eq!(presence::of_in(&world, NameScope::Roster, &near), Status::Unknown);
        // once named in the world (an exception), the row says it again
        restrict(&mut world, &format!("shownames:{near}=add"));
        world.refresh_rlva_privacy();
        assert_eq!(presence::of_in(&world, NameScope::Roster, &near), told);
    }

    #[test]
    fn shownames_leaves_a_group_member_named_in_the_member_list() {
        let (world, near) = shownames_world();
        // no friend of ours, around or not
        assert!(named(&world, NameScope::Roster, MEMBER));
        assert_eq!(world.social.display_name_in(NameScope::Roster, &MEMBER), "Member Tester");
        assert!(named(&world, NameScope::Roster, near));
        assert!(!named(&world, NameScope::World, MEMBER));
    }

    #[test]
    fn shownames_hides_an_object_owner_around_us_or_not() {
        let (world, near) = shownames_world();
        assert!(!named(&world, NameScope::Owner, near));
        // "nearby only" would name these two: the Legacy Viewer hides them
        assert!(!named(&world, NameScope::Owner, FRIEND));
        assert!(!named(&world, NameScope::Owner, MAKER));
        let me = world.agent_id;
        assert!(world.rlva_shows_name(NameScope::Owner, me));
    }

    #[test]
    fn shownames_hides_an_object_creator_who_owns_it_or_is_around_us() {
        let (mut world, near) = shownames_world();
        let creator = |owner| NameScope::Creator { owner };
        // away, and someone else's object (or a group's, or nobody's)
        assert!(named(&world, creator(FRIEND), MAKER));
        assert!(named(&world, creator(near), MAKER));
        assert!(named(&world, creator(Uuid::nil()), MAKER));
        // their own object
        assert!(!named(&world, creator(MAKER), MAKER));
        // around us, loaded as an avatar
        assert!(!named(&world, creator(FRIEND), near));
        // around us, known by the region's coarse locations alone
        let handle = world.main_region.unwrap();
        world.coarse.entry(handle).or_default().push((MAKER, glam::Vec3::ONE));
        world.refresh_rlva_privacy();
        assert!(!named(&world, creator(FRIEND), MAKER));
        world.coarse.clear();
        world.refresh_rlva_privacy();
        assert!(named(&world, creator(FRIEND), MAKER));
    }

    #[test]
    fn a_shownames_exception_names_its_resident_in_every_place() {
        let (mut world, near) = shownames_world();
        let creator = |owner| NameScope::Creator { owner };
        for id in [near, MAKER] {
            restrict(&mut world, &format!("shownames:{id}=add"));
            world.refresh_rlva_privacy();
            for scope in [NameScope::World, NameScope::Owner, creator(id), creator(FRIEND), NameScope::Roster] {
                assert!(named(&world, scope, id), "{scope:?}");
            }
            assert!(world.social.name_of(&id).contains("Tester"));
            // the exception names its resident alone
            assert!(!named(&world, NameScope::World, FRIEND));
            assert!(!named(&world, NameScope::Owner, FRIEND));
            restrict(&mut world, &format!("shownames:{id}=rem"));
            world.refresh_rlva_privacy();
            assert!(!named(&world, NameScope::World, id));
            assert!(!named(&world, NameScope::Owner, id));
            assert!(!named(&world, creator(id), id));
        }
    }

    #[test]
    fn shownames_opens_a_profile_from_a_list_that_names_its_resident_only() {
        let (mut world, near) = shownames_world();
        let mut profiles = crate::ui::profile::ProfileUi::default();
        // from the world: nothing opens, around us or not
        profiles.open(&mut world, near);
        profiles.open(&mut world, FRIEND);
        profiles.open_in(&mut world, NameScope::Owner, FRIEND);
        profiles.open_in(&mut world, NameScope::Creator { owner: MAKER }, MAKER);
        assert_eq!(profiles.opened(), Vec::<(Uuid, NameScope)>::new());
        // from the friends list or a group's members
        profiles.open_in(&mut world, NameScope::Roster, FRIEND);
        profiles.open_in(&mut world, NameScope::Roster, near);
        assert_eq!(profiles.opened(), vec![(FRIEND, NameScope::Roster), (near, NameScope::Roster)]);
        // a later world request does not turn the window into a world one
        profiles.open(&mut world, FRIEND);
        assert_eq!(profiles.opened().len(), 2);
        // ours opens from anywhere
        let me = world.agent_id;
        profiles.open(&mut world, me);
        assert_eq!(profiles.opened().last(), Some(&(me, NameScope::World)));
    }

    // ---- the places that name their resident besides the lists: IM
    // sessions, the notices of messages and offers (follow-up of #3547)

    fn line(kind: crate::world::ChatKind, source: Uuid, from: &str, text: &str) -> crate::world::ChatLine {
        crate::world::ChatLine {
            time: std::time::SystemTime::now(),
            from: from.into(),
            text: text.into(),
            kind,
            source,
            origin: crate::world::Origin::Live,
            blocked: false,
            beyond_range: false,
        }
    }

    #[test]
    fn shownames_names_those_around_in_an_im_session_and_hides_them_in_the_nearby_chat() {
        use crate::ui::chat::{conversation_names, line_names, resolved_text_in, sender_name};
        use crate::world::ChatKind;
        let (world, near) = shownames_world();
        let session = Some(near);
        assert_eq!(conversation_names(None), NameScope::World);
        assert_eq!(conversation_names(session.as_ref()), NameScope::Roster);
        // the sender of a line, by the conversation it is read in
        let said = line(ChatKind::Im, near, "Near Tester", "Near Tester is here");
        assert_eq!(sender_name(&world, NameScope::Roster, "Me", &said), "Near Tester");
        assert!(!sender_name(&world, NameScope::World, "Me", &said).contains("Tester"));
        // its text: the session keeps the names it was written with
        assert_eq!(resolved_text_in(&world, NameScope::Roster, &said.text), said.text);
        assert!(!resolved_text_in(&world, NameScope::World, &said.text).contains("Tester"));
        assert_eq!(world.rlva_filter_text_in(NameScope::Roster, &said.text), said.text);
        assert_eq!(
            world.rlva_filter_text_in(NameScope::Owner, &said.text),
            world.rlva_filter_text(&said.text)
        );
        // the conversation itself, in the list and on its window
        assert_eq!(world.session_title(&near), "Near Tester");
        assert_eq!(
            world.social.display_name_in(conversation_names(session.as_ref()), &near),
            "Near Tester"
        );
        // outside its conversation (the on-screen chat), by the line's kind
        assert_eq!(line_names(&ChatKind::Im), NameScope::Roster);
        assert_eq!(line_names(&ChatKind::Invite(1)), NameScope::Roster);
        for kind in [
            ChatKind::Local(aurora_net::ChatType::Normal),
            ChatKind::Object(aurora_net::ChatType::Normal),
            ChatKind::OWN,
            ChatKind::System,
            ChatKind::ObjectIm,
            ChatKind::InventoryOffer(1),
            ChatKind::Payment(1),
        ] {
            assert_eq!(line_names(&kind), NameScope::World, "{kind:?}");
        }
    }

    #[test]
    fn shownames_leaves_the_places_hidden_in_an_im_session() {
        let (mut world, _) = shownames_world();
        restrict(&mut world, "showloc=n");
        world.refresh_rlva_privacy();
        let region = world.region_name();
        assert!(!region.is_empty());
        let text = format!("Near Tester waits in {region}");
        let nearby = world.rlva_filter_text(&text);
        assert!(!nearby.contains("Tester") && !nearby.contains(&region), "{nearby}");
        // stricter than the Legacy Viewer, whose IM windows filter nothing
        let im = world.rlva_filter_text_in(NameScope::Roster, &text);
        assert!(im.starts_with("Near Tester waits in ") && !im.contains(&region), "{im}");
        // `@showloc` alone: the names are no one's to hide
        restrict(&mut world, "shownames=y");
        world.refresh_rlva_privacy();
        assert_eq!(world.rlva_filter_text(&text), im);
    }

    #[test]
    fn shownames_names_the_sender_of_a_message_or_an_offer_and_hides_a_giver_around() {
        use crate::world::notifications::{Data, Kind};
        let (mut world, near) = shownames_world();
        let named = [
            Kind::Message,
            Kind::Call,
            Kind::Teleport,
            Kind::Friendship,
            Kind::Group,
            Kind::Presence,
        ];
        let hidden = [Kind::Inventory, Kind::Info, Kind::Alert, Kind::Script, Kind::Permissions, Kind::Url];
        let link = || crate::world::notification_history::NameLink {
            name: "Near Tester".into(),
            target: crate::world::notification_history::NameTarget::Avatar(near),
        };
        let mut ids = Vec::new();
        for kind in named.into_iter().chain(hidden) {
            let id = world
                .notifications
                .push(kind, "Near Tester", "Near Tester has something for you.", Data::None);
            world.notifications.list.last_mut().unwrap().name_links.push(link());
            ids.push((kind, id));
        }
        world.refresh_rlva_privacy();
        for (kind, id) in ids {
            let n = world.notifications.list.iter().find(|n| n.id == id).unwrap();
            let shown = named.contains(&kind);
            assert_eq!(kind.names() == NameScope::Roster, shown, "{kind:?}");
            assert_eq!(n.title == "Near Tester", shown, "{kind:?}: {}", n.title);
            assert_eq!(n.body.contains("Near Tester"), shown, "{kind:?}: {}", n.body);
            assert_eq!(n.name_links[0].name == "Near Tester", shown, "{kind:?}");
            // what the notification centre keeps of it follows the same rule
            let mut kept = (n.title.clone(), n.body.clone(), [link()]);
            world
                .rlva_filter_in(kind.names())
                .redact_notice(&world.social.avatar_names, &mut kept.0, &mut kept.1, &mut kept.2);
            assert_eq!(kept.2[0].name == "Near Tester", shown, "{kind:?}");
        }
    }

    #[test]
    fn shownames_names_who_offers_a_teleport_and_hides_who_gives_or_pays_in_the_same_conversation() {
        let (world, near) = shownames_world();
        // the cards of a conversation: an invitation's sender is named,
        // an inventory offer's giver and a payment's other side follow the
        // world's rule
        assert_eq!(world.person_name_in(NameScope::Roster, &near).as_deref(), Some("Near Tester"));
        let told = world.person_name_in(NameScope::World, &near).unwrap();
        assert!(!told.contains("Tester"), "{told}");
        assert_eq!(world.person_name(&near), Some(told));
        // a resident away, whose name is not cached yet: the name they
        // wrote with, never a stand-in
        let away = Uuid::from_u128(0xF9);
        assert_eq!(world.person_name_in(NameScope::Roster, &away), None);
    }

    /// A message of `dialog` from `from`, as the simulator sends it.
    fn message(world: &World, from: Uuid, dialog: u8, text: &str) -> aurora_net::NetEvent {
        aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: from,
            from_name: "Near Tester".into(),
            to_agent_id: world.agent_id,
            dialog,
            session_id: Uuid::from_u128(0x5e55),
            message: text.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        })
    }

    #[test]
    fn shownames_names_who_writes_or_offers_friendship_as_their_message_arrives() {
        use crate::world::notifications::{Kind, im};
        let (mut world, near) = shownames_world();
        // an instant message: its line, its conversation and its toast
        world.apply(message(&world, near, 0, "hello there"));
        let line = |world: &World| {
            let session = world.social.ims.iter().find(|s| s.other == near).expect("the conversation");
            session.lines.back().expect("its line").clone()
        };
        assert_eq!(line(&world).from, "Near Tester");
        assert_eq!(world.session_title(&near), "Near Tester");
        for m in std::mem::take(&mut world.im_messages) {
            world.notify_message(&m, false);
        }
        // a friendship offer
        world.apply(message(&world, near, im::FRIENDSHIP_OFFERED, "be my friend"));
        world.refresh_rlva_privacy();
        for kind in [Kind::Message, Kind::Friendship] {
            let n = world.notifications.list.iter().find(|n| n.kind == kind).expect("the notice");
            assert!(n.title.contains("Near Tester"), "{kind:?}: {}", n.title);
            assert!(n.name_links.iter().any(|l| l.name == "Near Tester"), "{kind:?}");
        }
        // the nearby chat of the same resident stays anonymous
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Near Tester".into(),
            source_id: near,
            owner_id: near,
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: aurora_net::ChatType::Normal,
            region: None,
            position: glam::Vec3::ZERO,
            message: "hello everyone".into(),
        }));
        let said = world.chat.back().expect("the nearby line");
        assert_eq!(said.text, "hello everyone");
        assert!(!said.from.contains("Tester"), "{}", said.from);
    }
}
