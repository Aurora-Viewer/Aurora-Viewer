//! The Legacy Viewer's conversation options that Aurora lacked, from its
//! Preferences › Chat (skins/default/xui/en/panel_preferences_chat.xml,
//! defaults from app_settings/settings.xml):
//!
//! - FSAutoOrderIMTabs, FSAutoOrderIMTabsPriorities, FSAutoOrderIMTabsAtTop:
//!   where the tab of a new conversation goes
//!   (FSFloaterIMContainer::addFloater, indra/newview/fsfloaterimcontainer.cpp);
//! - FSIMTabNameFormat: a resident's name on their tab
//!   (FSFloaterIM::onAvatarNameCache, fsfloaterim.cpp);
//! - FSHighlightGroupMods, FSModNameStyle, FSModTextStyle: a moderator's
//!   lines (FSFloaterIM::updateMessages, FSChatHistory::appendMessage,
//!   fschathistory.cpp);
//! - FSChatHistoryShowYou: "(You)" for our name (FSUrlEntryAgentSelf::getLabel,
//!   indra/llui/llurlentry.cpp, strings.xml AgentNameSubst);
//! - FSIMSystemMessageBrackets: the system's lines in brackets
//!   (FSChatHistory::appendMessage);
//! - FSIMChatHistoryFade, adapted: the Legacy Viewer fades the instant
//!   messages it copies into the nearby chat (FIRE-2899) and the times of
//!   history lines in its plain-text look, neither of which Aurora has; here
//!   it fades the history lines themselves, which both viewers already grey;
//! - FSOpenIMContainerOnOfflineMessage: the Conversations window opens when
//!   messages kept while we were offline arrive (LLIMMgr::addMessage,
//!   llimview.cpp; the flag is `Social::offline_ims`).
//!
//! The settings are published once per frame (`publish`), as the chat's
//! clock is (`bars::clock::set_chat_format`): the many places that draw a
//! line read them without a new argument each.

use super::sidebar::Kind;
use crate::ui::fonts::TextEmphasis;
use crate::world::names::{AvatarName, NameOptions};
use crate::world::{ChatKind, ChatLine, Origin};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::RwLock;
use uuid::Uuid;

/// The least a history line fades to (FSIMChatHistoryFade's min_val).
pub const FADE_MIN: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// FSAutoOrderIMTabs (off): a new conversation's tab goes among those of
    /// its kind, the kinds in `order`; the workspace's sections follow
    /// `order` too. Off, the tabs keep the order they opened in (the
    /// workspace keeps its private, groups, conferences sections).
    pub auto_order: bool,
    /// FSAutoOrderIMTabsPriorities ("210": groups, conferences, IMs).
    pub order: TabOrder,
    /// FSAutoOrderIMTabsAtTop (off): first of its kind rather than last.
    pub new_first: bool,
    /// FSIMTabNameFormat (0: the display name).
    pub tab_names: TabNames,
    /// FSHighlightGroupMods (on).
    pub highlight_moderators: bool,
    /// FSModNameStyle (1: bold).
    pub moderator_name: FontStyle,
    /// FSModTextStyle (1: bold).
    pub moderator_text: FontStyle,
    /// FSChatHistoryShowYou (off).
    pub show_you: bool,
    /// FSIMSystemMessageBrackets (off).
    pub system_brackets: bool,
    /// How visible the history lines stay, `FADE_MIN` to 1. 1 by default:
    /// the history keeps the grey both viewers give it (see the module).
    pub history_fade: f32,
    /// FSOpenIMContainerOnOfflineMessage (off).
    pub open_on_offline: bool,
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        auto_order: false,
        order: TabOrder::GroupsConferencesIms,
        new_first: false,
        tab_names: TabNames::DisplayName,
        highlight_moderators: true,
        moderator_name: FontStyle::Bold,
        moderator_text: FontStyle::Bold,
        show_you: false,
        system_brackets: false,
        history_fade: 1.0,
        open_on_offline: false,
    };

    /// A fade out of its range (an edited file) comes back into it.
    pub fn sanitize(&mut self) {
        self.history_fade = if self.history_fade.is_finite() {
            self.history_fade.clamp(FADE_MIN, 1.0)
        } else {
            1.0
        };
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// FSAutoOrderIMTabsPriorities' six choices, the kinds from the top.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabOrder {
    /// "210", the default.
    #[default]
    GroupsConferencesIms,
    /// "201"
    GroupsImsConferences,
    /// "120"
    ConferencesGroupsIms,
    /// "021"
    ConferencesImsGroups,
    /// "102"
    ImsGroupsConferences,
    /// "012"
    ImsConferencesGroups,
}

impl TabOrder {
    pub const ALL: [TabOrder; 6] = [
        TabOrder::GroupsConferencesIms,
        TabOrder::GroupsImsConferences,
        TabOrder::ConferencesGroupsIms,
        TabOrder::ConferencesImsGroups,
        TabOrder::ImsGroupsConferences,
        TabOrder::ImsConferencesGroups,
    ];

    /// The kinds from the top.
    pub(super) fn kinds(self) -> [Kind; 3] {
        use Kind::{Conference as C, Group as G, Private as P};
        match self {
            TabOrder::GroupsConferencesIms => [G, C, P],
            TabOrder::GroupsImsConferences => [G, P, C],
            TabOrder::ConferencesGroupsIms => [C, G, P],
            TabOrder::ConferencesImsGroups => [C, P, G],
            TabOrder::ImsGroupsConferences => [P, G, C],
            TabOrder::ImsConferencesGroups => [P, C, G],
        }
    }

    /// The kind's priority: higher goes first, as the digits of the Legacy
    /// setting.
    fn priority(self, kind: Kind) -> usize {
        3 - self.kinds().iter().position(|k| *k == kind).unwrap_or(2)
    }

    pub fn label(self) -> &'static str {
        use aurora_i18n::tr;
        match self {
            TabOrder::GroupsConferencesIms => tr!("ui-options-im-tab-order-groups-conferences-ims"),
            TabOrder::GroupsImsConferences => tr!("ui-options-im-tab-order-groups-ims-conferences"),
            TabOrder::ConferencesGroupsIms => tr!("ui-options-im-tab-order-conferences-groups-ims"),
            TabOrder::ConferencesImsGroups => tr!("ui-options-im-tab-order-conferences-ims-groups"),
            TabOrder::ImsGroupsConferences => tr!("ui-options-im-tab-order-ims-groups-conferences"),
            TabOrder::ImsConferencesGroups => tr!("ui-options-im-tab-order-ims-conferences-groups"),
        }
    }
}

/// FSIMTabNameFormat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabNames {
    /// 0, the default.
    #[default]
    DisplayName,
    /// 1
    Username,
    /// 2: "Display Name (username)", the complete name.
    DisplayAndUsername,
    /// 3: "Username (Display Name)".
    UsernameAndDisplay,
}

/// FSModNameStyle and FSModTextStyle, as the panel offers them (the setting
/// also knows underlined styles, which the panel does not list).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontStyle {
    Normal,
    #[default]
    Bold,
    Italic,
    BoldItalic,
}

impl FontStyle {
    pub(crate) fn emphasis(self) -> TextEmphasis {
        TextEmphasis {
            strong: matches!(self, FontStyle::Bold | FontStyle::BoldItalic),
            italics: matches!(self, FontStyle::Italic | FontStyle::BoldItalic),
        }
    }
}

static CURRENT: RwLock<Settings> = RwLock::new(Settings::DEFAULT);

/// The settings of this frame, for the lines and tabs drawn after it.
pub fn publish(s: Settings) {
    if let Ok(mut current) = CURRENT.write()
        && *current != s
    {
        *current = s;
    }
}

/// The settings published last (the defaults before the first frame).
pub(crate) fn current() -> Settings {
    CURRENT.read().map(|s| *s).unwrap_or(Settings::DEFAULT)
}

/// Where the tab of a new conversation of `new` kind goes among the tabs of
/// `tabs` kinds (the nearby chat's locked tab left out): the loop of
/// FSFloaterIMContainer::addFloater. Last of its kind, or first with
/// `first`; a kind not open yet goes before the first kind of lower
/// priority.
pub(super) fn insertion(tabs: &[Kind], new: Kind, order: TabOrder, first: bool) -> usize {
    let mut found = false;
    for (i, &kind) in tabs.iter().enumerate() {
        if kind == new {
            if first {
                return i;
            }
            found = true;
        } else if found || order.priority(kind) < order.priority(new) {
            return i;
        }
    }
    tabs.len()
}

/// The kinds' sections of the workspace from the top.
pub(super) fn section_kinds(s: &Settings) -> [Kind; 3] {
    if s.auto_order {
        s.order.kinds()
    } else {
        [Kind::Private, Kind::Group, Kind::Conference]
    }
}

/// A resident's name on their tab (FSFloaterIM::onAvatarNameCache): the
/// complete name without display names, else the format chosen.
pub(crate) fn tab_name(name: &AvatarName, o: &NameOptions, format: TabNames) -> String {
    if !o.use_display_names {
        return name.complete(o);
    }
    match format {
        TabNames::DisplayName => name.display_name.clone(),
        TabNames::Username => name.user_name_for_display(o),
        TabNames::DisplayAndUsername => name.complete(o),
        TabNames::UsernameAndDisplay if name.is_default => name.user_name_for_display(o),
        TabNames::UsernameAndDisplay => format!("{} ({})", name.user_name_for_display(o), name.display_name),
    }
}

/// How a moderator's line reads, its name then its text, when the option
/// is on: a resident's live line (ours too, when we moderate) in a
/// conversation whose moderators are `moderators`. History lines keep
/// their look, as FSFloaterIM::updateMessages skips them.
pub(super) fn moderator(
    s: &Settings,
    moderators: Option<&HashSet<Uuid>>,
    line: &ChatLine,
    agent: Uuid,
) -> Option<(TextEmphasis, TextEmphasis)> {
    if !s.highlight_moderators || line.origin != Origin::Live {
        return None;
    }
    let who = match line.kind {
        ChatKind::Own(_) => agent,
        ChatKind::Im => line.source,
        _ => return None,
    };
    moderators
        .is_some_and(|m| !who.is_nil() && m.contains(&who))
        .then(|| (s.moderator_name.emphasis(), s.moderator_text.emphasis()))
}

/// "(You)" in place of our name over one of our live lines. An emote keeps
/// the name, which is its text, and a history line the name it was written
/// with: the Legacy Viewer links neither to our own profile.
pub(super) fn you(s: &Settings, line: &ChatLine) -> Option<&'static str> {
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    (s.show_you && line.kind.is_own() && line.origin == Origin::Live && !emote).then(|| aurora_i18n::tr!("ui-chat-you-name"))
}

/// A system line's text, in brackets with the option.
pub(super) fn system_text<'a>(s: &Settings, text: &'a str) -> Cow<'a, str> {
    if s.system_brackets {
        Cow::Owned(format!("[{text}]"))
    } else {
        Cow::Borrowed(text)
    }
}

/// The colour of a history line, faded.
pub(super) fn faded(s: &Settings, color: egui::Color32) -> egui::Color32 {
    if s.history_fade >= 1.0 {
        color
    } else {
        color.gamma_multiply(s.history_fade.clamp(FADE_MIN, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::{Conference as C, Group as G, Private as P};
    use std::time::SystemTime;

    #[test]
    fn a_new_tab_goes_after_its_kind_in_the_chosen_order() {
        let order = TabOrder::GroupsConferencesIms;
        let tabs = [G, G, C, P, P];
        assert_eq!(insertion(&tabs, G, order, false), 2, "after the last group");
        assert_eq!(insertion(&tabs, C, order, false), 3);
        assert_eq!(insertion(&tabs, P, order, false), 5);
        assert_eq!(insertion(&tabs, G, order, true), 0, "first of the groups");
        assert_eq!(insertion(&tabs, P, order, true), 3);
        assert_eq!(insertion(&[], P, order, false), 0);
    }

    #[test]
    fn a_kind_not_open_yet_goes_before_the_first_lower_one() {
        let tabs = [G, P];
        assert_eq!(insertion(&tabs, C, TabOrder::GroupsConferencesIms, false), 1);
        assert_eq!(insertion(&tabs, C, TabOrder::ConferencesGroupsIms, false), 0);
        assert_eq!(insertion(&tabs, C, TabOrder::GroupsImsConferences, false), 2);
        assert_eq!(insertion(&[P, P], G, TabOrder::ImsGroupsConferences, true), 2);
    }

    #[test]
    fn tabs_out_of_order_keep_the_legacy_loop() {
        // opened before the option: the loop stops at the first tab that
        // ends its kind's run or ranks lower, as the Legacy Viewer's does
        let tabs = [P, G, P, G];
        assert_eq!(insertion(&tabs, G, TabOrder::GroupsConferencesIms, false), 0);
        assert_eq!(insertion(&tabs, P, TabOrder::GroupsConferencesIms, false), 1);
    }

    #[test]
    fn every_order_ranks_its_kinds_from_the_top() {
        for order in TabOrder::ALL {
            let [a, b, c] = order.kinds();
            assert!(
                order.priority(a) > order.priority(b) && order.priority(b) > order.priority(c),
                "{order:?}"
            );
        }
        let mut s = Settings::DEFAULT;
        assert_eq!(section_kinds(&s), [P, G, C], "off: Aurora's sections");
        s.auto_order = true;
        assert_eq!(section_kinds(&s), [G, C, P]);
    }

    fn name(display: &str, first: &str, last: &str, default: bool) -> AvatarName {
        AvatarName {
            username: format!("{first}.{last}").to_lowercase(),
            display_name: display.into(),
            legacy_first: first.into(),
            legacy_last: last.into(),
            is_default: default,
            temporary: false,
            expires: 0.0,
            next_update: 0.0,
        }
    }

    #[test]
    fn tab_names_follow_the_format() {
        let o = NameOptions {
            use_display_names: true,
            show_usernames: true,
            ..Default::default()
        };
        let nova = name("Nova ♪", "Nova", "Example", false);
        assert_eq!(tab_name(&nova, &o, TabNames::DisplayName), "Nova ♪");
        assert_eq!(tab_name(&nova, &o, TabNames::Username), "Nova Example");
        assert_eq!(tab_name(&nova, &o, TabNames::DisplayAndUsername), nova.complete(&o));
        assert_eq!(tab_name(&nova, &o, TabNames::UsernameAndDisplay), "Nova Example (Nova ♪)");
        // no display name chosen: the username alone
        let orion = name("Orion Example", "Orion", "Example", true);
        assert_eq!(tab_name(&orion, &o, TabNames::UsernameAndDisplay), "Orion Example");
        // display names off: the name the viewer shows everywhere
        let off = NameOptions {
            use_display_names: false,
            ..o
        };
        assert_eq!(tab_name(&nova, &off, TabNames::DisplayName), nova.complete(&off));
    }

    fn line(kind: ChatKind, source: Uuid, origin: Origin, text: &str) -> ChatLine {
        ChatLine {
            time: SystemTime::UNIX_EPOCH,
            from: "Someone".into(),
            text: text.into(),
            kind,
            source,
            origin,
            blocked: false,
            beyond_range: false,
        }
    }

    #[test]
    fn moderators_lines_take_their_styles() {
        let (me, boss, member) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let mods: HashSet<Uuid> = [boss].into();
        let mut s = Settings::DEFAULT;
        let theirs = line(ChatKind::Im, boss, Origin::Live, "hello");
        let (name, text) = moderator(&s, Some(&mods), &theirs, me).expect("a moderator's line");
        assert!(name.strong && !name.italics && text.strong && !text.italics, "bold by default");
        assert!(moderator(&s, Some(&mods), &line(ChatKind::Im, member, Origin::Live, "hi"), me).is_none());
        assert!(
            moderator(&s, Some(&mods), &line(ChatKind::Im, boss, Origin::Log, "old"), me).is_none(),
            "history lines keep their look"
        );
        assert!(moderator(&s, None, &theirs, me).is_none(), "no moderators: a private IM");
        let mine = line(ChatKind::OWN, Uuid::nil(), Origin::Live, "mine");
        assert!(moderator(&s, Some(&mods), &mine, me).is_none());
        assert!(moderator(&s, Some(&[me].into()), &mine, me).is_some(), "ours, when we moderate");
        s.moderator_name = FontStyle::Italic;
        s.moderator_text = FontStyle::BoldItalic;
        let (name, text) = moderator(&s, Some(&mods), &theirs, me).expect("still");
        assert!(name.italics && !name.strong && text.italics && text.strong);
        assert!(!FontStyle::Normal.emphasis().strong && !FontStyle::Normal.emphasis().italics);
        s.highlight_moderators = false;
        assert!(moderator(&s, Some(&mods), &theirs, me).is_none());
    }

    #[test]
    fn you_replaces_our_name_on_our_live_lines_only() {
        let mut s = Settings::DEFAULT;
        let mine = line(ChatKind::OWN, Uuid::nil(), Origin::Live, "hello");
        assert_eq!(you(&s, &mine), None, "off by default");
        s.show_you = true;
        assert!(you(&s, &mine).is_some());
        assert!(
            you(&s, &line(ChatKind::OWN, Uuid::nil(), Origin::Live, "/me waves")).is_none(),
            "an emote"
        );
        assert!(you(&s, &line(ChatKind::OWN, Uuid::nil(), Origin::Log, "old")).is_none(), "history");
        assert!(you(&s, &line(ChatKind::Im, Uuid::from_u128(2), Origin::Live, "theirs")).is_none());
    }

    #[test]
    fn system_lines_get_brackets_with_the_option() {
        let mut s = Settings::DEFAULT;
        assert_eq!(system_text(&s, "Nova is online."), "Nova is online.");
        s.system_brackets = true;
        assert_eq!(system_text(&s, "Nova is online."), "[Nova is online.]");
    }

    #[test]
    fn history_fade_stays_in_its_range() {
        let grey = egui::Color32::from_gray(200);
        let mut s = Settings::DEFAULT;
        assert_eq!(faded(&s, grey), grey, "1 by default: no change");
        s.history_fade = 0.5;
        assert_eq!(faded(&s, grey), grey.gamma_multiply(0.5));
        s.history_fade = 0.0;
        assert_eq!(faded(&s, grey), grey.gamma_multiply(FADE_MIN), "never below the range");
        for (from, to) in [(0.0, FADE_MIN), (3.0, 1.0), (f32::NAN, 1.0), (0.6, 0.6)] {
            s.history_fade = from;
            s.sanitize();
            assert_eq!(s.history_fade, to);
        }
    }

    #[test]
    fn defaults_are_the_legacy_viewers() {
        let s = Settings::default();
        assert!(!s.auto_order && !s.new_first && !s.show_you && !s.system_brackets && !s.open_on_offline);
        assert!(s.highlight_moderators);
        assert_eq!((s.order, s.tab_names), (TabOrder::GroupsConferencesIms, TabNames::DisplayName));
        assert_eq!((s.moderator_name, s.moderator_text), (FontStyle::Bold, FontStyle::Bold));
        // an older file without the block reads as the defaults
        let read: Settings = serde_json::from_str("{}").expect("an empty block");
        assert_eq!(read, s);
    }
}
