//! Display-only identity and location filtering. Canonical IDs, name cache
//! records and region coordinates stay available to protocol handlers.
use super::super::{
    ChatKind, ChatLine, Origin, Speaker, World,
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
    /// `body`: none for a body shown as written (`notifications::Kind::text`).
    pub fn redact_notice(
        &self,
        names: &AvatarNames,
        title: &mut String,
        body: Option<&mut String>,
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
        if let Some(body) = body
            && let Cow::Owned(text) = filter.apply(body)
        {
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

    /// A resident's name, as RlvUtil::filterNames (rlvcommon.cpp) takes
    /// it: more than two bytes long (`std::string::length`), and as it is
    /// written, not as an address would escape it.
    fn add_name(&mut self, original: &str, replacement: &str) {
        if original.len() > 2 {
            self.add(original, replacement);
        }
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
        let mut names = self.rlva_name_filter(false);
        names.sort();
        names.reuse_matcher(&self.rlva_name_privacy);
        let mut filter = self.rlva_text_filter(false);
        filter.reuse_matcher(&self.rlva_privacy);
        let mut places = TextFilter::default();
        self.rlva_hide_places(&mut places);
        places.sort();
        places.reuse_matcher(&self.rlva_place_privacy);
        // The notices that came since the last pass are rewritten now, once
        // (`Notifications::settle`): one already shown keeps its text when a
        // restriction comes or goes, as the Legacy Viewer's notifications
        // do. (A notice that names its resident keeps them: `name_resident`.)
        let fresh = self.notifications.settle();
        let mut mentions = Vec::new();
        for (text, notification) in self.notifications.with_text_scopes() {
            // a script's prompt stays as received, its owner's name included
            if !fresh.contains(&notification.id) || notification.kind.as_received() {
                continue;
            }
            let filter = if text.names == NameScope::Roster { &places } else { &filter };
            filter.redact_notice(
                &self.social.avatar_names,
                &mut notification.title,
                text.rewritten.then_some(&mut notification.body),
                &mut notification.name_links,
            );
            if text.mentions != NameScope::Roster {
                mentions.push((notification.id, text.rewritten));
            }
        }
        // (RlvUtil::filterNames ends with filterMentions)
        for (id, body) in mentions {
            let Some(at) = self.notifications.list.iter().position(|n| n.id == id) else {
                continue;
            };
            let title = self.rlva_freeze_mentions(&self.notifications.list[at].title).into_owned();
            let text = body.then(|| self.rlva_freeze_mentions(&self.notifications.list[at].body).into_owned());
            let notification = &mut self.notifications.list[at];
            notification.title = title;
            if let Some(text) = text {
                notification.body = text;
            }
        }
        self.rlva_privacy = filter;
        self.rlva_name_privacy = names;
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

    /// What an `rlvanonym` link shows, one the viewer did not write itself
    /// (a text that holds `secondlife:///app/agent/<id>/rlvanonym`).
    /// LLUrlEntryAgentRLVAnonymizedName (indra/llui/llurlentry.cpp) is an
    /// LLUrlEntryAgentName: it waits for the name of the id, like any
    /// avatar link, then shows RlvStrings::getAnonym of it, the stand-in of
    /// the legacy name; and getAnonym gives LLTrans "Unknown" while RLVa is
    /// off. Never the stand-in of the id: the link shows the same words as
    /// the resident's tag and chat lines do.
    pub fn rlva_anonym_label(&self, id: Uuid) -> String {
        let names = &self.social.avatar_names;
        names.want(&id);
        match names.raw(&id) {
            _ if !self.rlva.enabled() => aurora_i18n::tr!("status-unknown").to_owned(),
            Some(name) => anonymous_named(&name.legacy_name()).display_name.clone(),
            None => "…".to_owned(),
        }
    }

    /// The name of `id` where the Legacy Viewer tests
    /// `hasBehaviour(RLV_BHVR_SHOWNAMES)` and no one in particular: while
    /// `@shownames` is set everyone is their stand-in there, oneself and an
    /// `@shownames:<uuid>` exception included, around us or not. The
    /// questions of LLAvatarActions::landEjectMultiple, landFreezeMultiple
    /// and the estate ones (llavataractions.cpp), mean_name_callback and
    /// the bump lines (llviewermessage.cpp), the sound explorer
    /// (NACLfloaterexploresounds.cpp), FSRadar::radarAlertMsg.
    pub fn rlva_name_unless_hidden(&self, id: Uuid) -> String {
        if self.rlva.has("shownames") {
            self.rlva_anonymous(id).display_name.clone()
        } else {
            self.social.name_in(NameScope::Roster, &id)
        }
    }

    /// How `@shownames` leaves `id` right now (`Speaker`).
    pub fn rlva_speaker(&self, id: Uuid) -> Speaker {
        if !self.rlva_can_name(id) {
            Speaker::Hidden
        } else if self.rlva.has("shownames") && self.social.avatar_names.rlva_around(&id) {
            Speaker::Around
        } else {
            Speaker::Named
        }
    }

    /// Who a line of `kind` that arrives now in a conversation shown in
    /// `scope` names as its sender, `from` being the name it came with
    /// (process_chat_from_simulator, indra/newview/llviewermessage.cpp,
    /// under `!canShowName(SNC_DEFAULT)`): the stand-in of a resident the
    /// nearby chat hides, the name of an object rewritten. An IM session
    /// names its people (FSFloaterIM has no RLVa test), and the name of an
    /// object's IM follows its owner (`World::on_im`).
    pub(crate) fn rlva_sender(&self, scope: NameScope, kind: ChatKind, source: Uuid, from: String) -> (Speaker, String) {
        if !self.rlva.has("shownames") {
            return (Speaker::Named, from);
        }
        match kind {
            ChatKind::Local(_) | ChatKind::Im => {
                let speaker = self.rlva_speaker(source);
                if speaker == Speaker::Hidden && scope == NameScope::World && matches!(kind, ChatKind::Local(_)) {
                    (speaker, self.rlva_anonymous(source).display_name.clone())
                } else {
                    (speaker, from)
                }
            }
            ChatKind::Object(_) if scope == NameScope::World => (Speaker::Named, self.rlva_object_name(&from).into_owned()),
            _ => (Speaker::Named, from),
        }
    }

    /// The name of an object that speaks, as it is kept with its line and
    /// in its `objectim` link: under `@shownames` the names of the residents
    /// around are their stand-ins in it, and nothing else changes, the
    /// places `@showloc` hides included. `RlvUtil::filterNames(chat.mFromName)`
    /// in process_chat_from_simulator (indra/newview/llviewermessage.cpp)
    /// and `RlvUtil::filterNames(name)` for IM_FROM_TASK
    /// (indra/newview/llimprocessing.cpp): RlvUtil::filterLocation runs on
    /// the text of the line alone
    /// (LLFloaterIMNearbyChatHandler::processChat).
    pub fn rlva_object_name<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self.rlva.has("shownames") {
            self.rlva_name_privacy.apply(name)
        } else {
            Cow::Borrowed(name)
        }
    }

    /// A line enters a conversation shown in `scope`: what `@shownames` and
    /// `@showloc` hide of it is decided now and kept with it, as the Legacy
    /// Viewer does (`ChatLine::wording`, `Speaker`). Its sender; the text of
    /// what no resident typed (an object's line, a notice of the viewer),
    /// rewritten; and in the nearby chat the mentions of the residents
    /// whose name is hidden, a muted line's left as they are
    /// (LLFloaterIMNearbyChatHandler::processChat hands it over before its
    /// filters).
    pub(crate) fn rlva_freeze_line(&self, scope: NameScope, line: &mut ChatLine) {
        if line.origin != Origin::Live || !self.rlva.enabled() {
            return;
        }
        let (speaker, from) = self.rlva_sender(scope, line.kind, line.source, std::mem::take(&mut line.from));
        line.speaker = speaker;
        line.from = from;
        if line.kind.is_automated()
            && let Cow::Owned(text) = self.rlva_filter_text_in(line.kind.text_in(scope), &line.text)
        {
            line.text = text;
        }
        if scope == NameScope::World
            && !line.blocked
            && let Cow::Owned(text) = self.rlva_freeze_mentions(&line.text)
        {
            line.text = text;
        }
    }

    /// RlvUtil::filterMentions (indra/newview/rlvcommon.cpp): under
    /// `@shownames`, the `/mention` link of a resident whose name is hidden
    /// becomes "@" and their stand-in, in plain text. As there, only a link
    /// that ends its line, or that `/` or `?` follows, is one
    /// (`/mention(?=/|\?|$)`): a mention followed by a space or by more
    /// text stays a link, which names its resident
    /// (LLUrlEntryAgentMention has no RLVa test).
    pub fn rlva_freeze_mentions<'a>(&self, text: &'a str) -> Cow<'a, str> {
        const PREFIX: &str = "secondlife:///app/agent/";
        const SUFFIX: &str = "/mention";
        if !self.rlva.has("shownames") || !text.contains(SUFFIX) {
            return Cow::Borrowed(text);
        }
        let end = PREFIX.len() + 36 + SUFFIX.len();
        let mut out = String::with_capacity(text.len());
        let mut changed = false;
        let mut rest = text;
        while let Some(i) = rest.find(PREFIX) {
            let link = &rest[i..];
            let id = link.get(PREFIX.len()..PREFIX.len() + 36).and_then(|u| Uuid::parse_str(u).ok());
            let ends = link.get(PREFIX.len() + 36..end) == Some(SUFFIX)
                && matches!(link[end..].chars().next(), None | Some('/' | '?' | '\n' | '\r'));
            match id {
                Some(id) if ends && !self.rlva_can_name(id) => {
                    out.push_str(&rest[..i]);
                    out.push('@');
                    out.push_str(&self.rlva_anonymous(id).display_name);
                    rest = &link[end..];
                    changed = true;
                }
                _ => {
                    out.push_str(&rest[..i + PREFIX.len()]);
                    rest = &link[PREFIX.len()..];
                }
            }
        }
        if !changed {
            return Cow::Borrowed(text);
        }
        out.push_str(rest);
        Cow::Owned(out)
    }

    /// The contact-set appearance of the sender of `line`: none for one
    /// whose name was hidden when the line arrived
    /// (`rlva_contact_appearance`, decided then: `Speaker`).
    pub fn rlva_line_appearance(&self, line: &ChatLine) -> &std::sync::Arc<Appearance> {
        if line.speaker == Speaker::Hidden {
            &PLAIN_APPEARANCE
        } else {
            self.contact_sets.appearance(&line.source)
        }
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

    /// The mark of a resident (`world::avatar_marks`) as the maps and the
    /// lists may show it: none for a resident whose name is hidden, whose
    /// mark would tell who they are. LLNetMap::draw
    /// (indra/newview/llnetmap.cpp) drops it from the dot
    /// (`canShowName(…) ? color : map_avatar_color`). The name tag reads
    /// `avatar_marks` itself: LLVOAvatar::getNameTagColor
    /// (indra/newview/llvoavatar.cpp) keeps the mark's colour there
    /// (`ui::colors::tag_color`).
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
        let mut filter = self.rlva_name_filter(clear_nearby);
        self.rlva_hide_places(&mut filter);
        filter.sort();
        filter
    }

    /// `rlva_text_filter` without the places: what RlvUtil::filterNames
    /// (indra/newview/rlvcommon.cpp) alone replaces. Not sorted.
    fn rlva_name_filter(&self, clear_nearby: bool) -> TextFilter {
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
                // RlvUtil::filterNames: the display name and the legacy
                // name of each avatar around, nothing else (not the
                // account name "jane.doe", nor a name escaped in an
                // address). Until the name service answers, the legacy
                // name the avatar's object came with stands for both.
                if let Some(name) = self.social.avatar_names.raw(&id) {
                    // the whole legacy name, "Resident" included:
                    // "Jane Resident" is one stand-in, not "<stand-in> Resident"
                    filter.add_name(&name.legacy_name(), &replacement);
                    filter.add_name(&name.display_name, &replacement);
                } else {
                    if let Some(name) = self.social.names.get(&id) {
                        filter.add_name(name, &replacement);
                    }
                    if let Some(name) = self
                        .objects
                        .index_of_uuid(&id)
                        .and_then(|i| self.objects.get(i))
                        .and_then(|o| o.display_name())
                    {
                        filter.add_name(&name, &replacement);
                    }
                }
            }
        }
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
    /// its residents (`NameScope::Roster`: a notice about a conversation)
    /// keeps the names of the people around, and hides the regions and the
    /// parcel like the others. A text shown as written
    /// (`TextScope::verbatim`: what a resident typed, a script's dialog) is
    /// left alone.
    pub fn rlva_filter_text_in<'a>(&self, scope: impl Into<TextScope>, text: &'a str) -> Cow<'a, str> {
        let scope = scope.into();
        if self.rlva.enabled() && scope.rewritten {
            self.rlva_filter_in(scope.names).apply(text)
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

pub use super::super::names::{NameScope, TextScope};

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

    /// `rlva_shows_name` where the Legacy Viewer names oneself alone, an
    /// `@shownames:<uuid>` exception hidden as anyone
    /// (LLSidepanelTaskInfo::refreshAll, llsidepaneltaskinfo.cpp).
    pub fn rlva_shows_name_but_exceptions(&self, scope: NameScope, id: Uuid) -> bool {
        self.social.avatar_names.shows_but_exceptions(scope, &id, &self.agent_id)
    }

    /// The notice of what `from` gives us, or of their answer to what we
    /// gave them, names them. inventory_offer_handler and IM_INVENTORY_ACCEPTED
    /// / IM_INVENTORY_DECLINED in process_improved_im
    /// (indra/newview/llimprocessing.cpp) hide a resident RLVa's `@shownames`
    /// hides only while they stand in a region around (RlvUtil::isNearbyAgent)
    /// and neither a conversation with them nor their profile is open
    /// (RlvUIEnabler::hasOpenIM and hasOpenProfile, indra/newview/rlvui.cpp;
    /// `Rlva::open_profiles`); the owner of an object that gives
    /// (`from_object`) is hidden whenever they stand around. Asked once,
    /// when the notice arrives.
    pub fn rlva_names_giver(&self, from: Uuid, from_object: bool) -> bool {
        !self.rlva.enabled()
            || self.rlva_can_name(from)
            || !self.rlva_nearby_avatars().contains(&from)
            || (!from_object && (self.social.has_session(&from) || self.rlva.open_profiles.contains(&from)))
    }

    /// The avatars of every region around, oneself left out
    /// (RlvUtil::isNearbyAgent over LLWorld::getAvatars, rlvcommon.cpp):
    /// the loaded ones and the coarse locations, whatever their distance.
    /// RlvUtil::isNearbyAgent: `id` is someone else in a region around.
    pub fn rlva_is_nearby(&self, id: Uuid) -> bool {
        !id.is_nil()
            && id != self.agent_id
            && (self.coarse.values().flatten().any(|(other, _)| *other == id)
                || self.objects.avatars().any(|(_, o)| o.full_id == id && !self.is_mirror(&id)))
    }

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
            .redact_notice(&world.social.avatar_names, &mut title, Some(&mut body), &mut links);
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
        filter.add_name("Élodie Meadows", "anonymous");
        // two bytes or less: not a name RlvUtil::filterNames replaces
        filter.add_name("Jo", "anonymous");
        assert_eq!(
            filter.apply("SECRET REGION ANNEX / Secret Region / ÉLODIE MEADOWS"),
            "annex / hidden / anonymous"
        );
        // a name is replaced as it is written, not as an address escapes it
        assert!(matches!(filter.apply("Élodie%20Meadows and Jo"), Cow::Borrowed(_)));
        assert!(matches!(filter.apply("%C3%89lodie%20Meadows"), Cow::Borrowed(_)));
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
    fn shownames_leaves_a_friend_around_us_named_in_the_list_with_their_status() {
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
        // the row says of them what it says of anyone: the Legacy Viewer's
        // friends list has no `@shownames` test (FSFloaterContacts)
        use crate::ui::presence::{self, Status};
        let told = presence::of(&world, &near);
        assert_ne!(told, Status::Unknown);
        assert_eq!(presence::of_in(&world, NameScope::Roster, &near), told);
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
            speaker: Default::default(),
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
        let mut said = line(ChatKind::Im, near, "Near Tester", "Near Tester is here");
        world.rlva_freeze_line(NameScope::Roster, &mut said);
        assert_eq!(sender_name(&world, "Me", &said), "Near Tester");
        assert_eq!(said.text, "Near Tester is here");
        let mut heard = line(ChatKind::Local(aurora_net::ChatType::Normal), near, "Near Tester", "hello");
        world.rlva_freeze_line(NameScope::World, &mut heard);
        assert!(!sender_name(&world, "Me", &heard).contains("Tester"));
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
        assert_eq!(line_names(&world, &ChatKind::Im), NameScope::Roster);
        assert_eq!(line_names(&world, &ChatKind::Invite(1)), NameScope::Roster);
        // a payment names its resident, as its card does
        assert_eq!(line_names(&world, &ChatKind::Payment(1)), NameScope::Roster);
        for kind in [
            ChatKind::Local(aurora_net::ChatType::Normal),
            ChatKind::Object(aurora_net::ChatType::Normal),
            ChatKind::OWN,
            ChatKind::System,
            ChatKind::ObjectIm,
            // (an offer that is not listed, or that hid its giver)
            ChatKind::InventoryOffer(1),
        ] {
            assert_eq!(line_names(&world, &kind), NameScope::World, "{kind:?}");
        }
    }

    #[test]
    fn shownames_leaves_the_places_hidden_in_a_notice_that_names_its_resident() {
        let (mut world, _) = shownames_world();
        restrict(&mut world, "showloc=n");
        world.refresh_rlva_privacy();
        let region = world.region_name();
        assert!(!region.is_empty());
        let text = format!("Near Tester waits in {region}");
        let nearby = world.rlva_filter_text(&text);
        assert!(!nearby.contains("Tester") && !nearby.contains(&region), "{nearby}");
        // a notice about a conversation (a teleport offer, a group's notice):
        // the places alone. The lines of the conversation itself are shown as
        // typed (the next test).
        let notice = world.rlva_filter_text_in(NameScope::Roster, &text);
        assert!(notice.starts_with("Near Tester waits in ") && !notice.contains(&region), "{notice}");
        // `@showloc` alone: the names are no one's to hide
        restrict(&mut world, "shownames=y");
        world.refresh_rlva_privacy();
        assert_eq!(world.rlva_filter_text(&text), notice);
    }

    /// What `@shownames` hides of a chat line is decided when the line
    /// arrives and kept with it (process_chat_from_simulator writes the
    /// stand-in in `chat.mFromName` and sets `mRlvNamesFiltered`): a line
    /// said under the restriction keeps its stand-in once it is lifted, one
    /// said without it keeps its name when it comes back.
    #[test]
    fn a_chat_line_keeps_what_shownames_made_of_it_when_it_arrived() {
        use crate::ui::chat::sender_name;
        use crate::world::Speaker;
        let (mut world, near) = shownames_world();
        let stand_in = world.rlva_anonymous(near).display_name.clone();
        let chat = |world: &mut World, source_type, source_id: Uuid, name: &str, text: &str| {
            world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
                from_name: name.into(),
                source_id,
                owner_id: near,
                source_type,
                chat_type: aurora_net::ChatType::Normal,
                region: None,
                position: glam::Vec3::ZERO,
                message: text.into(),
            }));
        };
        let object = Uuid::from_u128(0x0B1E_C7ED);
        let agent = aurora_net::ChatSourceType::Agent;
        let thing = aurora_net::ChatSourceType::Object;
        chat(&mut world, agent, near, "Near Tester", "said while hidden");
        chat(
            &mut world,
            thing,
            object,
            "Greeter of Near Tester",
            "Near Tester waves while hidden",
        );
        restrict(&mut world, "shownames=y");
        world.refresh_rlva_privacy();
        chat(&mut world, agent, near, "Near Tester", "said while named");
        chat(&mut world, thing, object, "Greeter of Near Tester", "Near Tester waves while named");
        let check = |world: &World| {
            let line = |text: &str| world.chat.iter().find(|l| l.text.ends_with(text)).cloned().expect("line");
            let hidden = line("said while hidden");
            assert_eq!(hidden.speaker, Speaker::Hidden);
            assert!(hidden.anonymous());
            assert_eq!(hidden.from, stand_in);
            assert_eq!(sender_name(world, "Me", &hidden), stand_in);
            assert!(world.rlva_line_appearance(&hidden).chat_text.is_none());
            let named = line("said while named");
            assert_eq!(named.speaker, Speaker::Named);
            assert!(!named.anonymous());
            assert_eq!(sender_name(world, "Me", &named), "Near Tester");
            // an object's line: its name, its text and the owner its
            // inspector hides (`rlv_shownames`)
            let hidden = line("waves while hidden");
            assert_eq!(hidden.text, format!("{stand_in} waves while hidden"));
            assert_eq!(hidden.from, format!("Greeter of {stand_in}"));
            assert_eq!(sender_name(world, "Me", &hidden), hidden.from);
            assert!(world.chat_object(&hidden).is_some_and(|o| o.owner_hidden));
            let named = line("waves while named");
            assert_eq!(named.text, "Near Tester waves while named");
            assert_eq!(named.from, "Greeter of Near Tester");
            assert!(world.chat_object(&named).is_some_and(|o| !o.owner_hidden));
            // nothing rewrites a line when it is drawn
            for l in [&hidden, &named] {
                assert!(!l.wording(NameScope::World).rewritten);
            }
        };
        // with the restriction lifted, then back
        check(&world);
        restrict(&mut world, "shownames=n");
        world.refresh_rlva_privacy();
        check(&world);
    }

    /// RlvUtil::filterMentions: the pattern ends with `(?=/|\?|$)`, so a
    /// mention is rewritten at the end of its line, or before `/` or `?`,
    /// and left as a link (which names its resident) before anything else.
    #[test]
    fn a_mention_is_rewritten_where_the_legacy_pattern_matches_it() {
        let (mut world, near) = shownames_world();
        let stand_in = world.rlva_anonymous(near).display_name.clone();
        let link = format!("secondlife:///app/agent/{near}/mention");
        assert_eq!(world.rlva_freeze_mentions(&format!("ask {link}")), format!("ask @{stand_in}"));
        assert_eq!(
            world.rlva_freeze_mentions(&format!("ask {link}\nnow")),
            format!("ask @{stand_in}\nnow")
        );
        assert_eq!(
            world.rlva_freeze_mentions(&format!("{link}?x {link}/y")),
            format!("@{stand_in}?x @{stand_in}/y")
        );
        for kept in [format!("ask {link} now"), format!("{link}, hello"), format!("{link}s")] {
            assert_eq!(world.rlva_freeze_mentions(&kept), kept);
        }
        // oneself, an exception, and no restriction
        let me = format!("ask secondlife:///app/agent/{}/mention", world.agent_id);
        assert_eq!(world.rlva_freeze_mentions(&me), me);
        restrict(&mut world, &format!("shownames:{near}=add"));
        world.refresh_rlva_privacy();
        let text = format!("ask {link}");
        assert_eq!(world.rlva_freeze_mentions(&text), text);
        restrict(&mut world, "shownames=y");
        world.refresh_rlva_privacy();
        assert_eq!(world.rlva_freeze_mentions(&text), text);
    }

    /// A notice is settled when it comes: one already shown keeps its text
    /// when `@shownames` comes, one that came under it keeps its stand-in
    /// once it is lifted.
    #[test]
    fn a_notice_keeps_what_shownames_made_of_it_when_it_came() {
        use crate::world::notifications::{Data, Kind};
        let (mut world, near) = shownames_world();
        let stand_in = world.rlva_anonymous(near).display_name.clone();
        let under = world
            .notifications
            .push(Kind::Info, "Near Tester waves", "Near Tester is here", Data::None);
        world.refresh_rlva_privacy();
        restrict(&mut world, "shownames=y");
        world.refresh_rlva_privacy();
        let without = world
            .notifications
            .push(Kind::Info, "Near Tester waves", "Near Tester is here", Data::None);
        world.refresh_rlva_privacy();
        restrict(&mut world, "shownames=n");
        world.refresh_rlva_privacy();
        let notice = |id: u64| world.notifications.list.iter().find(|n| n.id == id).expect("notice");
        assert_eq!(notice(under).title, format!("{stand_in} waves"));
        assert_eq!(notice(under).body, format!("{stand_in} is here"));
        assert_eq!(notice(without).title, "Near Tester waves");
        assert_eq!(notice(without).body, "Near Tester is here");
        for id in [under, without] {
            assert!(!world.notifications.text_of(notice(id)).rewritten);
        }
    }

    /// `@shownames` and `@showloc` rewrite what an object or the system
    /// says, never what a resident typed: LLFloaterIMNearbyChatHandler::
    /// processChat runs RlvUtil::filterNames and filterLocation only when
    /// `CHAT_SOURCE_AGENT != mSourceType`, process_chat_from_simulator
    /// anonymises the name of an avatar's line alone, and no IM window
    /// filters anything. A `/mention` is rewritten whoever wrote it
    /// (RlvUtil::filterMentions).
    #[test]
    fn what_a_resident_typed_is_shown_as_typed_and_what_an_object_says_is_rewritten() {
        use crate::ui::chat::resolved_text_in;
        use crate::world::ChatKind;
        use aurora_net::ChatType::Normal;
        let (mut world, near) = shownames_world();
        restrict(&mut world, "showloc=n");
        world.refresh_rlva_privacy();
        let region = world.region_name();
        assert!(!region.is_empty());
        let text = format!("Near Tester waits in {region}");
        // a resident's line, ours included, in the nearby chat and in a session
        for (kind, names) in [
            (ChatKind::Local(Normal), NameScope::World),
            (ChatKind::Own(Normal), NameScope::World),
            (ChatKind::Im, NameScope::Roster),
            (ChatKind::OWN, NameScope::Roster),
        ] {
            let scope = kind.text_in(names);
            assert_eq!(scope, TextScope::verbatim(names), "{kind:?}");
            assert_eq!(world.rlva_filter_text_in(scope, &text), text, "{kind:?}");
            assert_eq!(resolved_text_in(&world, scope, &text), text, "{kind:?}");
        }
        // what no resident typed: an object's line and IM, the notices, the cards
        for kind in [
            ChatKind::Object(Normal),
            ChatKind::ObjectIm,
            ChatKind::System,
            ChatKind::Invite(1),
            ChatKind::InventoryOffer(1),
            ChatKind::Payment(1),
        ] {
            let scope = kind.text_in(NameScope::World);
            assert_eq!(scope, NameScope::World.into(), "{kind:?}");
            let shown = world.rlva_filter_text_in(scope, &text);
            assert!(!shown.contains("Tester") && !shown.contains(&region), "{kind:?}: {shown}");
        }
        // a mention a resident typed still gives the stand-in
        let mention = format!("hello secondlife:///app/agent/{near}/mention");
        let shown = resolved_text_in(&world, ChatKind::Local(Normal).text_in(NameScope::World), &mention);
        assert!(shown.starts_with("hello ") && !shown.contains("Tester"), "{shown}");
        // as the lines arrive: the speaker of a resident's line is hidden,
        // its text kept; an object's text is rewritten for good
        let chat = |source_type, source: Uuid, from: &str| {
            aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
                from_name: from.into(),
                source_id: source,
                owner_id: source,
                source_type,
                chat_type: Normal,
                region: None,
                position: glam::Vec3::ZERO,
                message: text.clone(),
            })
        };
        world.apply(chat(aurora_net::ChatSourceType::Agent, near, "Near Tester"));
        let said = world.chat.back().expect("the resident's line");
        assert_eq!(said.text, text);
        assert!(!said.from.contains("Tester"), "{}", said.from);
        world.apply(chat(aurora_net::ChatSourceType::Object, Uuid::from_u128(0xB0C5), "Box"));
        let said = world.chat.back().expect("the object's line");
        assert!(matches!(said.kind, ChatKind::Object(_)), "{:?}", said.kind);
        assert!(!said.text.contains("Tester") && !said.text.contains(&region), "{}", said.text);
    }

    /// process_chat_from_simulator (indra/newview/llviewermessage.cpp) and
    /// IM_FROM_TASK (indra/newview/llimprocessing.cpp) run
    /// RlvUtil::filterNames alone over the name of an object that speaks,
    /// and only under `@shownames`: the places `@showloc` hides stay in it,
    /// in the line and in the `objectim` link its inspector shows.
    #[test]
    fn the_name_of_an_object_that_speaks_loses_the_names_and_keeps_the_places() {
        use crate::world::ChatKind;
        let (mut world, near) = shownames_world();
        restrict(&mut world, "showloc=n");
        world.refresh_rlva_privacy();
        let region = world.region_name();
        assert!(!region.is_empty());
        let both = world.rlva_filter_text(&format!("Near Tester in {region}")).into_owned();
        assert!(!both.contains("Tester") && !both.contains(&region), "a text loses both: {both}");
        let name = format!("Near Tester and the box of {region}");
        let kept = format!(" and the box of {region}");
        let says = |source: u128| {
            aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
                from_name: name.clone(),
                source_id: Uuid::from_u128(source),
                owner_id: near,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::Normal,
                region: None,
                position: glam::Vec3::ZERO,
                message: "hello".into(),
            })
        };
        world.apply(says(0xB0C5));
        let said = world.chat.back().expect("the object line").clone();
        assert!(matches!(said.kind, ChatKind::Object(_)), "{:?}", said.kind);
        let who = said
            .from
            .strip_suffix(&kept)
            .unwrap_or_else(|| panic!("the place stays: {}", said.from));
        assert!(is_anonym(who, anonym_index("Near Tester")), "{who}");
        assert_eq!(world.chat_object(&said).expect("its origin").name, said.from);
        // its IM, the owner standing around
        world.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: near,
            from_name: name.clone(),
            to_agent_id: world.agent_id,
            dialog: 19,
            session_id: Uuid::from_u128(0xB0C6),
            message: "hello".into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }));
        let said = world.chat.back().expect("the object IM").clone();
        assert_eq!(said.kind, ChatKind::ObjectIm);
        let who = said
            .from
            .strip_suffix(&kept)
            .unwrap_or_else(|| panic!("the place stays: {}", said.from));
        assert!(is_anonym(who, anonym_index("Near Tester")), "{who}");
        assert_eq!(world.chat_object(&said).expect("its origin").name, said.from);
        // under `@showloc` alone the name is as the object has it
        restrict(&mut world, "shownames=y");
        world.refresh_rlva_privacy();
        world.apply(says(0xB0C7));
        assert_eq!(world.chat.back().expect("the object line").from, name);
    }

    /// LLUrlEntryAgentRLVAnonymizedName (indra/llui/llurlentry.cpp): an
    /// `rlvanonym` link found in a text shows the stand-in of the legacy
    /// name of its resident, the one their tag and chat lines show, once
    /// the name is known; never the stand-in of the id. RLVa off, it is
    /// LLTrans "Unknown" (RlvStrings::getAnonym).
    #[test]
    fn an_rlvanonym_link_shows_the_stand_in_of_the_name_once_it_is_known() {
        use crate::ui::chat::resolved_text_in;
        let (mut world, _) = shownames_world();
        let (id, name) = [(FRIEND, "Friend Tester"), (MEMBER, "Member Tester"), (MAKER, "Maker Tester")]
            .into_iter()
            .find(|(id, name)| anonym_index(name) != anonym_index(&id.to_string()))
            .expect("a resident whose id and name give two stand-ins");
        let label = world.rlva_anonym_label(id);
        assert!(is_anonym(&label, anonym_index(name)), "{label}");
        let text = format!("see secondlife:///app/agent/{id}/rlvanonym now");
        let shown = resolved_text_in(&world, NameScope::World, &text);
        let label = shown.strip_prefix("see ").and_then(|s| s.strip_suffix(" now")).expect("plain text");
        assert!(is_anonym(label, anonym_index(name)), "{shown}");
        // a name still to come: the link waits for it, like any avatar link
        assert_eq!(world.rlva_anonym_label(Uuid::from_u128(0xABCD)), "\u{2026}");
        world.rlva.configure(false);
        let off = world.rlva_anonym_label(id);
        assert!(
            ["en", "fr", "ja"]
                .iter()
                .any(|language| aurora_i18n::text_in(language, "status-unknown") == off),
            "{off}"
        );
    }

    /// A resident whose last name is "Resident", named in full by an object
    /// (llKey2Name gives "Jane Resident"): the whole legacy name is one
    /// stand-in, as RlvUtil::filterNames replaces
    /// LLAvatarName::getLegacyName before the display name. The filter knew
    /// "Jane" alone and left "<stand-in> Resident" (seen on the grid,
    /// 2026-11-18, for residents known by their name only).
    #[test]
    fn a_resident_named_in_full_is_one_stand_in() {
        let (mut world, near) = shownames_world();
        world.social.avatar_names.on_legacy(near, "Solo", "Resident");
        world.refresh_rlva_privacy();
        let stand_in = world.rlva_anonymous(near).display_name.clone();
        assert_eq!(
            world.rlva_filter_text("Here:\nSolo Resident\nSolo waves"),
            format!("Here:\n{stand_in}\n{stand_in} waves")
        );
    }

    /// A script's prompt (its dialog or text box, its permission question,
    /// its link prompt) is shown as the simulator sent it, the name of the
    /// object's owner included: process_script_dialog,
    /// process_script_question and process_load_url have no RLVa test on
    /// what they show (indra/newview/llviewermessage.cpp). The text of a
    /// message on its card is as typed too.
    #[test]
    fn a_script_s_prompt_is_shown_as_received_with_its_owner_s_name() {
        use crate::world::notifications::{Data, Kind};
        let (mut world, near) = shownames_world();
        restrict(&mut world, "showloc=n");
        world.refresh_rlva_privacy();
        let region = world.region_name();
        let body = format!("Pick someone in {region}:\n1. Near Tester");
        for kind in [Kind::Script, Kind::Permissions, Kind::Url] {
            assert!(kind.as_received(), "{kind:?}");
            assert_eq!(kind.text(), TextScope::as_received(NameScope::World), "{kind:?}");
            let id = world.notifications.push(kind, "Box (Near Tester)", body.clone(), Data::None);
            world
                .notifications
                .list
                .last_mut()
                .unwrap()
                .name_links
                .push(crate::world::notification_history::NameLink {
                    name: "Near Tester".into(),
                    target: crate::world::notification_history::NameTarget::Avatar(near),
                });
            world.refresh_rlva_privacy();
            let n = world.notifications.list.iter().find(|n| n.id == id).unwrap();
            assert_eq!(n.body, body, "{kind:?}");
            assert_eq!(n.title, "Box (Near Tester)", "{kind:?}");
            assert_eq!(n.name_links[0].name, "Near Tester", "{kind:?}");
            // a click on that name still follows the world's rule: the
            // Legacy Viewer writes it as plain text
            assert_eq!(kind.names(), NameScope::World, "{kind:?}");
            assert!(!world.rlva_shows_name(kind.names(), near));
        }
        // the card of a message names its sender and keeps what they typed
        assert!(!Kind::Message.as_received());
        assert_eq!(Kind::Message.text(), TextScope::verbatim(NameScope::Roster));
        // the other notices are rewritten, by the names they show
        for kind in [Kind::Info, Kind::Alert, Kind::Inventory] {
            assert!(!kind.as_received(), "{kind:?}");
            assert_eq!(kind.text(), NameScope::World.into(), "{kind:?}");
        }
        for kind in [Kind::Teleport, Kind::Friendship, Kind::Group, Kind::Presence, Kind::Call] {
            assert!(!kind.as_received(), "{kind:?}");
            assert_eq!(kind.text(), NameScope::Roster.into(), "{kind:?}");
        }
    }

    /// A `/mention` link is rewritten where RlvUtil::filterMentions
    /// (rlvcommon.cpp) runs: on every line of the nearby chat
    /// (LLFloaterIMNearbyChatHandler::processChat) and at the end of
    /// RlvUtil::filterNames, so in what an object or the system says.
    /// Nothing runs it on a script's prompt nor on an IM session:
    /// LLUrlEntryAgentMention (indra/llui/llurlentry.cpp) has no RLVa test
    /// and writes "@" and the name of the resident there. A bare avatar
    /// link names its resident everywhere (LLUrlEntryAgent).
    #[test]
    fn a_mention_is_rewritten_in_the_nearby_chat_and_names_its_resident_in_a_script_s_prompt() {
        use crate::ui::chat::{resolved_text_in, text_with_links};
        use crate::world::ChatKind;
        use crate::world::notifications::Kind;
        use aurora_net::ChatType::Normal;
        let (world, near) = shownames_world();
        let stand_in = world.rlva_anonymous(near).display_name.clone();
        let text = format!("ask secondlife:///app/agent/{near}/mention or secondlife:///app/agent/{near}/about");
        let named = "ask @Near Tester or Near Tester";
        // a script's prompt, and a session where every name is shown
        for kind in [Kind::Script, Kind::Permissions, Kind::Url] {
            assert_eq!(kind.text().mentions, NameScope::Roster, "{kind:?}");
            assert_eq!(resolved_text_in(&world, kind.text(), &text), named, "{kind:?}");
        }
        assert_eq!(resolved_text_in(&world, ChatKind::Im.text_in(NameScope::Roster), &text), named);
        assert_eq!(resolved_text_in(&world, Kind::Message.text(), &text), named);
        // the nearby chat, whoever speaks: the mention alone is rewritten in
        // what a resident typed
        for kind in [ChatKind::Local(Normal), ChatKind::Own(Normal)] {
            let scope = kind.text_in(NameScope::World);
            assert_eq!(scope.mentions, NameScope::World, "{kind:?}");
            assert_eq!(
                resolved_text_in(&world, scope, &text),
                format!("ask @{stand_in} or Near Tester"),
                "{kind:?}"
            );
        }
        // what an object or the system says, and the viewer's own notices
        for scope in [
            ChatKind::Object(Normal).text_in(NameScope::World),
            ChatKind::System.text_in(NameScope::World),
            Kind::Info.text(),
            Kind::Alert.text(),
        ] {
            assert_eq!(scope.mentions, NameScope::World, "{scope:?}");
            let shown = resolved_text_in(&world, scope, &text);
            assert_eq!(shown, format!("ask @{stand_in} or Near Tester"), "{scope:?}");
        }
        // as a notice draws it
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let drawn = |scope: TextScope| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                    ..Default::default()
                },
                |ui| text_with_links(ui, &p, &world, scope, &text, &[], 12.0, p.ink),
            );
            output.textures_delta.clear();
            let mut shown = String::new();
            for clipped in &output.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    shown.push_str(t.galley.text());
                    shown.push('|');
                }
            }
            shown
        };
        let prompt = drawn(Kind::Script.text());
        assert!(prompt.contains("@Near Tester|") && !prompt.contains(&stand_in), "{prompt}");
        let notice = drawn(Kind::Info.text());
        assert!(
            notice.contains(&format!("@{stand_in}|")) && !notice.contains("@Near Tester"),
            "{notice}"
        );
        assert!(notice.contains("Near Tester|"), "a bare avatar link names its resident: {notice}");
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
        let hidden = [Kind::Inventory, Kind::Info, Kind::Alert];
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
            // (a body shown as written keeps what it was written with)
            assert_eq!(
                n.body.contains("Near Tester"),
                shown || !kind.text().rewritten,
                "{kind:?}: {}",
                n.body
            );
            assert_eq!(n.name_links[0].name == "Near Tester", shown, "{kind:?}");
            // what the notification centre keeps of it follows the same rule
            let mut kept = (n.title.clone(), n.body.clone(), [link()]);
            world
                .rlva_filter_in(kind.names())
                .redact_notice(&world.social.avatar_names, &mut kept.0, Some(&mut kept.1), &mut kept.2);
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

    /// The rule behind the greyed entries of an avatar's menu
    /// (`Facts::name_hidden`): RlvActions::canShowName(SNC_DEFAULT, id),
    /// where only oneself and a `@shownames:<uuid>` exception are named.
    #[test]
    fn the_name_of_a_resident_is_shown_only_to_oneself_and_exceptions() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (stranger, excepted) = (Uuid::from_u128(0x35471), Uuid::from_u128(0x35472));
        world.rlva.configure(true);
        assert!(world.rlva_can_name(stranger), "no restriction: every name is shown");
        restrict(&mut world, "shownames=n");
        assert!(!world.rlva_can_name(stranger));
        assert!(!world.rlva_can_name(excepted));
        assert!(world.rlva_can_name(world.agent_id), "never ourselves");
        restrict(&mut world, &format!("shownames:{excepted}=add"));
        assert!(world.rlva_can_name(excepted));
        assert!(!world.rlva_can_name(stranger));
    }

    /// The label of an avatar link is text someone wrote: `@shownames` and
    /// `@showloc` filter it like the text around it (RlvUtil::filterNames and
    /// filterLocation run on the raw text of the chat, rlvcommon.cpp). The
    /// address of the link stays, so a bare link still resolves to its
    /// resident (LLUrlEntryAgent has no RLVa test).
    #[test]
    fn the_label_of_an_avatar_link_is_filtered_like_the_text_around_it() {
        use crate::world::names::NameScope;
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        // someone around us: only the residents in the scene are rewritten
        let agent = world.agent_id;
        let id = world.objects.avatars().map(|(_, o)| o.full_id).find(|id| *id != agent).unwrap();
        let name = aurora_net::AvatarNameData {
            id,
            username: "jane.doe".into(),
            display_name: "Jane Doe".into(),
            legacy_first: "Jane".into(),
            legacy_last: "Doe".into(),
            ..Default::default()
        };
        world.social.avatar_names.on_display_names(&[name], &[], None);
        let handle = world.main_region.unwrap();
        world.regions.get_mut(&handle).unwrap().name = "Secret Region".into();
        world.rlva.configure(true);
        restrict(&mut world, "shownames=n,showloc=n");
        world.refresh_rlva_privacy();
        let address = format!("secondlife:///app/agent/{id}/about");
        for verb in ["about", "inspect", "im", "mention"] {
            let link = format!("[secondlife:///app/agent/{id}/{verb} Jane Doe in Secret Region]");
            let filtered = world.rlva_filter_text_in(NameScope::World, &link);
            assert!(
                !filtered.contains("Jane Doe"),
                "{verb}: the name of the label is hidden: {filtered}"
            );
            assert!(
                !filtered.contains("Secret Region"),
                "{verb}: the place of the label is hidden: {filtered}"
            );
            assert!(filtered.contains(&id.to_string()), "{verb}: the address stays: {filtered}");
        }
        let text = format!("Jane Doe points at {address} from Secret Region");
        let filtered = world.rlva_filter_text_in(NameScope::World, &text);
        assert!(filtered.contains(&address), "a bare link stays: {filtered}");
        assert!(!filtered.contains("Jane Doe") && !filtered.contains("Secret Region"), "{filtered}");
        // a conversation names its people and still hides the places
        let said = format!("[{address} Jane Doe in Secret Region]");
        let filtered = world.rlva_filter_text_in(NameScope::Roster, &said);
        assert!(filtered.contains("Jane Doe") && !filtered.contains("Secret Region"), "{filtered}");
    }

    /// A profile opened from a conversation names its resident
    /// (`NameScope::Roster`): the line under the display name keeps the
    /// legacy name while `@shownames` swaps it for the stand-in in the world.
    #[test]
    fn a_profile_opened_from_a_conversation_keeps_the_legacy_name() {
        use crate::world::names::NameScope;
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let stranger = Uuid::from_u128(0x35473);
        world.social.names.insert(stranger, "Jane Doe".into());
        world.rlva.configure(true);
        restrict(&mut world, "shownames=n");
        assert_ne!(world.legacy_name_in(NameScope::World, &stranger).as_deref(), Some("Jane Doe"));
        assert_eq!(world.legacy_name_in(NameScope::Roster, &stranger).as_deref(), Some("Jane Doe"));
        assert_eq!(
            world.legacy_name(&stranger).as_deref(),
            Some("Jane Doe"),
            "the cached name wins, as before"
        );
    }
}
