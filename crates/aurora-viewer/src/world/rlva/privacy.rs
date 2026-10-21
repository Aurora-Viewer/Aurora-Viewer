//! Display-only identity and location filtering. Canonical IDs, name cache
//! records and region coordinates stay available to protocol handlers.
use super::super::{World, names::AvatarName};
use std::borrow::Cow;
use uuid::Uuid;

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
                filter.add(&link.name, replacement);
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
            self.replacements.push((original.to_lowercase(), replacement.to_owned()));
        }
    }

    pub fn apply<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.replacements.is_empty() {
            return Cow::Borrowed(text);
        }
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut out = String::new();
        let mut offset = 0;
        let mut changed = false;
        while offset < text.len() {
            let remaining = &text[offset..];
            let boundary = offset == 0 || !text[..offset].chars().next_back().is_some_and(word);
            let replacement = boundary
                .then(|| {
                    self.replacements.iter().find_map(|(needle, replacement)| {
                        // Unicode lowercase can expand a scalar (for example İ).
                        // Compare scalar streams while retaining original byte ends.
                        let mut expected = needle.chars().peekable();
                        for (byte, c) in remaining.char_indices() {
                            for lower in c.to_lowercase() {
                                if expected.next() != Some(lower) {
                                    return None;
                                }
                            }
                            if expected.peek().is_none() {
                                let end = byte + c.len_utf8();
                                if !remaining[end..].chars().next().is_some_and(word) {
                                    return Some((end, replacement));
                                }
                                return None;
                            }
                        }
                        None
                    })
                })
                .flatten();
            if let Some((count, replacement)) = replacement {
                out.push_str(replacement);
                offset += count;
                changed = true;
            } else {
                let c = remaining.chars().next().expect("nonempty suffix");
                out.push(c);
                offset += c.len_utf8();
            }
        }
        if changed { Cow::Owned(out) } else { Cow::Borrowed(text) }
    }
}

impl World {
    pub fn refresh_rlva_privacy(&mut self) {
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
        let filter = self.rlva_text_filter(false);
        for notification in &mut self.notifications.list {
            filter.redact_notice(&mut notification.title, &mut notification.body, &mut notification.name_links);
        }
        self.rlva_privacy = filter;
        self.rlva_hover_privacy = if self.rlva.has("shownearby") {
            self.rlva_text_filter(true)
        } else {
            self.rlva_privacy.clone()
        };
    }

    pub fn rlva_can_name(&self, id: Uuid) -> bool {
        !self.rlva.enabled() || id == self.agent_id || self.rlva.allows("shownames", &id.to_string())
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
                let replacement = if clear_nearby {
                    String::new()
                } else {
                    anonymous(id).display_name.clone()
                };
                if let Some(name) = self.social.names.get(&id) {
                    filter.add(name, &replacement);
                }
                if let Some(name) = self
                    .objects
                    .index_of_uuid(&id)
                    .and_then(|i| self.objects.get(i))
                    .and_then(|o| o.display_name())
                {
                    filter.add(&name, &replacement);
                }
                if let Some(name) = self.social.avatar_names.raw(&id) {
                    filter.add(&name.display_name, &replacement);
                    filter.add(&name.user_name(), &replacement);
                    filter.add(&name.username, &replacement);
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
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
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
    use crate::world::rlva::{Command, Rule};

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
        // Privacy commands stay unadvertised while their remaining UI entry
        // points are audited; exercise the display policy independently.
        for text in ["shownames=n", "showloc=n"] {
            world
                .rlva
                .rules
                .entry(Uuid::from_u128(1))
                .or_default()
                .insert(Rule::from(&Command::parse(text).unwrap()));
        }
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
