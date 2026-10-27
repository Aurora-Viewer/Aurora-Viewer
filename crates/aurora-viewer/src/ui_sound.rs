//! Interface sounds, after Firestorm's UISnd* settings
//! (indra/newview/app_settings/settings.xml, defaults of `UISnd<name>` and
//! `PlayModeUISnd<name>`), find_ui_sound / make_ui_sound
//! (indra/llui/llui.cpp) and the UI sounds panel
//! (indra/newview/fspanelpreferenceuisounds.cpp); originally LGPL 2.1,
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project.
//!
//! Each sound has Firestorm's asset UUID and "play" default; both can be
//! changed per sound in Preferences › Sound and voice. The sounds are SL assets
//! (fetched from the ViewerAsset capability, cached like the other sounds).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// One interface sound (Firestorm `UISnd<name>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiSound {
    Alert,
    BadKeystroke,
    Click,
    ClickRelease,
    FriendOffline,
    FriendOnline,
    FriendshipOffer,
    GroupInvitation,
    GroupNotice,
    /// Someone calls (UISndIncomingVoiceCall, LLIncomingCallDialog::onOpen).
    IncomingVoiceCall,
    InventoryOffer,
    MicToggle,
    MoneyChangeDown,
    MoneyChangeUp,
    NearbyChat,
    NewIncomingImSession,
    NewIncomingGroupImSession,
    NewIncomingConfImSession,
    ObjectCreate,
    ObjectDelete,
    ObjectRezOut,
    ObjectRezIn,
    PieMenuAppear,
    PieMenuHide,
    /// UISndPieMenuSliceHighlight0..7.
    PieMenuSlice(u8),
    /// The radar's alerts (UISndRadarChatEnter… FIRE-6069, played by
    /// FSRadar::updateRadarList as each crossing is reported).
    RadarChatEnter,
    RadarChatLeave,
    RadarDrawEnter,
    RadarDrawLeave,
    RadarRegionEnter,
    RadarRegionLeave,
    RadarAgeAlert,
    Restart,
    /// The region restart notice of an OpenSim grid (UISndRestartOpenSim).
    RestartOpenSim,
    /// A photo taken (UISndSnapshot).
    Snapshot,
    ScriptFloaterOpen,
    ScriptFloaterClose,
    StartIm,
    TeleportOffer,
    TeleportOut,
    TrackerBeacon,
    Typing,
    WindowClose,
    WindowOpen,
}

/// The sounds of the radar's alerts.
pub const RADAR_SOUNDS: [UiSound; 7] = [
    UiSound::RadarChatEnter,
    UiSound::RadarChatLeave,
    UiSound::RadarDrawEnter,
    UiSound::RadarDrawLeave,
    UiSound::RadarRegionEnter,
    UiSound::RadarRegionLeave,
    UiSound::RadarAgeAlert,
];

/// Catalog entry: Firestorm setting name (without `UISnd`), localized label,
/// default UUID and default `PlayModeUISnd<name>`.
pub struct Entry {
    pub sound: UiSound,
    pub name: &'static str,
    pub label: fn() -> &'static str,
    pub uuid: Uuid,
    pub play: bool,
}

const fn u(v: u128) -> Uuid {
    Uuid::from_u128(v)
}

const ALERT: Uuid = u(0xed124764_705d_d497_167a_182cd9fa2e6c);
const CLICK: Uuid = u(0x4c8c3c77_de8d_bde2_b9b8_32635e0fd4a6);
const NEW_SESSION: Uuid = u(0x67cc2844_00f3_2b3c_b991_6418d01e1bb7);
const START_IM: Uuid = u(0xc825dfbc_9827_7e02_6507_3713d18916c1);
const WINDOW_OPEN: Uuid = u(0xc80260ba_41fd_8a46_768a_6bf236360e3a);
const WINDOW_CLOSE: Uuid = u(0x2c346eda_b60c_ab33_1119_b8941916a499);
const PIE_SLICE_0: Uuid = u(0xd9f73cf8_17b4_6f7a_1565_7951226c305d);
const NEARBY_CHAT: Uuid = u(0xa3f48b85_c29f_1f97_ebb6_644b7c053512);

macro_rules! e {
    ($s:expr, $n:literal, $l:expr, $u:expr, $p:literal) => {
        Entry {
            sound: $s,
            name: $n,
            label: || $l,
            uuid: $u,
            play: $p,
        }
    };
}

use UiSound as S;

/// Every interface sound, in the order of the preferences list.
#[rustfmt::skip]
pub const CATALOG: [Entry; 51] = [
    e!(S::Click, "Click", aurora_i18n::tr!("ui-sound-click-button-pressed"), CLICK, true),
    e!(S::ClickRelease, "ClickRelease", aurora_i18n::tr!("ui-sound-click-button-released"), CLICK, true),
    e!(S::BadKeystroke, "BadKeystroke", aurora_i18n::tr!("ui-sound-key-rejected"), u(0x2ca849ba_2885_4bc3_90ef_d4987a5b983a), true),
    e!(S::WindowOpen, "WindowOpen", aurora_i18n::tr!("ui-sound-window-opened"), WINDOW_OPEN, true),
    e!(S::WindowClose, "WindowClose", aurora_i18n::tr!("ui-sound-window-closed"), WINDOW_CLOSE, true),
    e!(S::ScriptFloaterOpen, "ScriptFloaterOpen", aurora_i18n::tr!("ui-sound-script-dialog-opened"), WINDOW_OPEN, true),
    e!(S::ScriptFloaterClose, "ScriptFloaterClose", aurora_i18n::tr!("ui-sound-script-dialog-closed"), WINDOW_CLOSE, true),
    e!(S::PieMenuAppear, "PieMenuAppear", aurora_i18n::tr!("ui-sound-context-menu-opened"), u(0x8eaed61f_92ff_6485_de83_4dcc938a478e), true),
    e!(S::PieMenuHide, "PieMenuHide", aurora_i18n::tr!("ui-sound-context-menu-closed"), Uuid::nil(), true),
    e!(S::PieMenuSlice(0), "PieMenuSliceHighlight0", aurora_i18n::tr!("ui-sound-menu-option-1-hovered"), PIE_SLICE_0, true),
    e!(S::PieMenuSlice(1), "PieMenuSliceHighlight1", aurora_i18n::tr!("ui-sound-menu-option-2-hovered"), u(0xf6ba9816_dcaf_f755_7b67_51b31b6233e5), true),
    e!(S::PieMenuSlice(2), "PieMenuSliceHighlight2", aurora_i18n::tr!("ui-sound-menu-option-3-hovered"), u(0x7aff2265_d05b_8b72_63c7_dbf96dc2f21f), true),
    e!(S::PieMenuSlice(3), "PieMenuSliceHighlight3", aurora_i18n::tr!("ui-sound-menu-option-4-hovered"), u(0x09b2184e_8601_44e2_afbb_ce37434b8ba1), true),
    e!(S::PieMenuSlice(4), "PieMenuSliceHighlight4", aurora_i18n::tr!("ui-sound-menu-option-5-hovered"), u(0xbbe4c7fc_7044_b05e_7b89_36924a67593c), true),
    e!(S::PieMenuSlice(5), "PieMenuSliceHighlight5", aurora_i18n::tr!("ui-sound-menu-option-6-hovered"), u(0xd166039b_b4f5_c2ec_4911_c85c727b016c), true),
    e!(S::PieMenuSlice(6), "PieMenuSliceHighlight6", aurora_i18n::tr!("ui-sound-menu-option-7-hovered"), u(0x242af82b_43c2_9a3b_e108_3b0c7e384981), true),
    e!(S::PieMenuSlice(7), "PieMenuSliceHighlight7", aurora_i18n::tr!("ui-sound-menu-option-8-hovered"), u(0xc1f334fb_a5be_8fe7_22b3_29631c21cf0b), true),
    e!(S::Alert, "Alert", aurora_i18n::tr!("ui-sound-alert"), ALERT, true),
    e!(S::Restart, "Restart", aurora_i18n::tr!("ui-sound-region-restart"), u(0xb92a0f64_7709_8811_40c5_16afd624a45f), true),
    e!(S::RestartOpenSim, "RestartOpenSim", aurora_i18n::tr!("ui-sound-region-restart-opensim"), u(0x4174f859_0d3d_c517_c424_72923dc21f65), true),
    e!(S::TeleportOut, "TeleportOut", aurora_i18n::tr!("common-teleport-2"), u(0xd7a9a565_a013_2a69_797d_5332baa1a947), true),
    e!(S::MoneyChangeUp, "MoneyChangeUp", aurora_i18n::tr!("ui-sound-l-received"), u(0x77a018af_098e_c037_51a6_178f05877c6f), true),
    e!(S::MoneyChangeDown, "MoneyChangeDown", aurora_i18n::tr!("ui-sound-l-spent"), u(0x104974e3_dfda_428b_99ee_b0d4e748d3a3), true),
    e!(S::StartIm, "StartIM", aurora_i18n::tr!("ui-sound-start-conversation"), START_IM, true),
    e!(S::NewIncomingImSession, "NewIncomingIMSession", aurora_i18n::tr!("ui-sound-private-message-received"), NEW_SESSION, true),
    e!(S::NewIncomingGroupImSession, "NewIncomingGroupIMSession", aurora_i18n::tr!("ui-sound-group-message-received"), NEW_SESSION, true),
    e!(S::NewIncomingConfImSession, "NewIncomingConfIMSession", aurora_i18n::tr!("ui-sound-conference-message-received"), NEW_SESSION, true),
    e!(S::NearbyChat, "NearbyChat", aurora_i18n::tr!("ui-sound-nearby-chat-received"), NEARBY_CHAT, false),
    e!(S::Typing, "Typing", aurora_i18n::tr!("ui-sound-typing-avatars"), u(0x5e191c7b_8996_9ced_a177_b2ac32bfea06), true),
    e!(S::FriendOnline, "FriendOnline", aurora_i18n::tr!("ui-sound-friend-online"), ALERT, false),
    e!(S::FriendOffline, "FriendOffline", aurora_i18n::tr!("ui-sound-friend-offline"), ALERT, false),
    e!(S::FriendshipOffer, "FriendshipOffer", aurora_i18n::tr!("ui-sound-friendship-offer"), NEW_SESSION, false),
    e!(S::TeleportOffer, "TeleportOffer", aurora_i18n::tr!("ui-sound-teleport-offer"), NEW_SESSION, false),
    e!(S::InventoryOffer, "InventoryOffer", aurora_i18n::tr!("ui-sound-object-offered"), NEW_SESSION, false),
    e!(S::GroupInvitation, "GroupInvitation", aurora_i18n::tr!("ui-sound-group-invitation"), START_IM, false),
    e!(S::GroupNotice, "GroupNotice", aurora_i18n::tr!("common-group-notice"), START_IM, false),
    e!(S::IncomingVoiceCall, "IncomingVoiceCall", aurora_i18n::tr!("ui-sound-incoming-voice-call"), WINDOW_OPEN, true),
    e!(S::ObjectCreate, "ObjectCreate", aurora_i18n::tr!("ui-sound-object-creation"), u(0xf4a0660f_5446_dea2_80b7_6482a082803c), true),
    e!(S::ObjectDelete, "ObjectDelete", aurora_i18n::tr!("ui-sound-object-deletion"), u(0x0cb7b00a_4c10_6948_84de_a93c09af2ba9), false),
    e!(S::ObjectRezOut, "ObjectRezOut", aurora_i18n::tr!("ui-sound-object-taken"), Uuid::nil(), true),
    e!(S::ObjectRezIn, "ObjectRezIn", aurora_i18n::tr!("ui-sound-object-rezzed"), u(0x3c8fc726_1fd6_862d_fa01_16c5b2568db6), true),
    e!(S::MicToggle, "MicToggle", aurora_i18n::tr!("ui-sound-microphone-on-off"), PIE_SLICE_0, false),
    e!(S::TrackerBeacon, "TrackerBeacon", aurora_i18n::tr!("ui-sound-destination-beacon"), ALERT, false),
    e!(S::Snapshot, "Snapshot", aurora_i18n::tr!("ui-sound-photo-taken"), u(0x3d09f582_3851_c0e0_f5ba_277ac5c73fb4), true),
    e!(S::RadarChatEnter, "RadarChatEnter", aurora_i18n::tr!("ui-sound-radar-chat-enter"), NEARBY_CHAT, false),
    e!(S::RadarChatLeave, "RadarChatLeave", aurora_i18n::tr!("ui-sound-radar-chat-leave"), NEARBY_CHAT, false),
    e!(S::RadarDrawEnter, "RadarDrawEnter", aurora_i18n::tr!("ui-sound-radar-draw-enter"), NEARBY_CHAT, false),
    e!(S::RadarDrawLeave, "RadarDrawLeave", aurora_i18n::tr!("ui-sound-radar-draw-leave"), NEARBY_CHAT, false),
    e!(S::RadarRegionEnter, "RadarSimEnter", aurora_i18n::tr!("ui-sound-radar-region-enter"), NEARBY_CHAT, false),
    e!(S::RadarRegionLeave, "RadarSimLeave", aurora_i18n::tr!("ui-sound-radar-region-leave"), NEARBY_CHAT, false),
    e!(S::RadarAgeAlert, "RadarAgeAlert", aurora_i18n::tr!("ui-sound-radar-age-alert"), NEARBY_CHAT, false),
];

impl UiSound {
    pub fn entry(self) -> &'static Entry {
        CATALOG
            .iter()
            .find(|e| e.sound == self)
            .expect("every UiSound is in CATALOG (tested)")
    }

    /// The IM session sounds are gated by their caller (the IM modes), not
    /// by the play flag, as in find_ui_sound.
    pub fn caller_gated(self) -> bool {
        matches!(
            self,
            S::NewIncomingImSession | S::NewIncomingGroupImSession | S::NewIncomingConfImSession | S::TrackerBeacon
        )
    }
}

/// When a conversation plays its sound (Firestorm's U32
/// `PlayModeUISndNewIncoming*Session`, shared with Chat › Notifications).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImSoundMode {
    Mute,
    /// Only for a new conversation (Firestorm default).
    #[default]
    NewSession,
    EveryMessage,
    /// Every message when the conversation is not in front.
    NotFocused,
}

impl ImSoundMode {
    pub const ALL: [ImSoundMode; 4] = [Self::Mute, Self::NewSession, Self::EveryMessage, Self::NotFocused];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mute => aurora_i18n::tr!("ui-sound-never"),
            Self::NewSession => aurora_i18n::tr!("ui-sound-new-conversation"),
            Self::EveryMessage => aurora_i18n::tr!("ui-sound-every-message"),
            Self::NotFocused => aurora_i18n::tr!("ui-sound-when-the-conversation-is-not-in-the-foreground"),
        }
    }

    /// Whether a message plays the sound (LLIMMgr::addMessage).
    pub fn plays(self, new_session: bool, focused: bool) -> bool {
        match self {
            Self::Mute => false,
            Self::NewSession => new_session,
            Self::EveryMessage => true,
            Self::NotFocused => new_session || !focused,
        }
    }
}

/// Kind of conversation, for its sound mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImKind {
    Private,
    Group,
    Conference,
}

/// A message received in a conversation (its sound is decided by the app,
/// which knows the IM modes and whether the conversation is in front).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImMessage {
    pub kind: ImKind,
    pub session: Uuid,
    pub new_session: bool,
}

/// Per-sound choices (only the ones changed from Firestorm's defaults are
/// saved, keyed by the Firestorm name).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSoundSettings {
    /// `PlayModeUISnd<name>`.
    pub play: BTreeMap<String, bool>,
    /// `UISnd<name>` (nil = silent).
    pub uuid: BTreeMap<String, Uuid>,
    pub im_mode: ImSoundMode,
    pub group_mode: ImSoundMode,
    pub conf_mode: ImSoundMode,
    /// UISndMoneyChangeThreshold (L$).
    pub money_threshold: f32,
}

impl Default for UiSoundSettings {
    fn default() -> Self {
        UiSoundSettings {
            play: BTreeMap::new(),
            uuid: BTreeMap::new(),
            im_mode: ImSoundMode::NewSession,
            group_mode: ImSoundMode::NewSession,
            conf_mode: ImSoundMode::NewSession,
            money_threshold: 50.0,
        }
    }
}

impl UiSoundSettings {
    pub fn plays(&self, s: UiSound) -> bool {
        let e = s.entry();
        self.play.get(e.name).copied().unwrap_or(e.play)
    }

    pub fn set_plays(&mut self, s: UiSound, on: bool) {
        let e = s.entry();
        if on == e.play {
            self.play.remove(e.name);
        } else {
            self.play.insert(e.name.to_owned(), on);
        }
    }

    pub fn uuid(&self, s: UiSound) -> Uuid {
        let e = s.entry();
        self.uuid.get(e.name).copied().unwrap_or(e.uuid)
    }

    pub fn set_uuid(&mut self, s: UiSound, id: Uuid) {
        let e = s.entry();
        if id == e.uuid {
            self.uuid.remove(e.name);
        } else {
            self.uuid.insert(e.name.to_owned(), id);
        }
    }

    /// The asset to play, or None when silent (find_ui_sound: play flag off
    /// unless `force`, nil UUID).
    pub fn resolve(&self, s: UiSound, force: bool) -> Option<Uuid> {
        if !force && !s.caller_gated() && !self.plays(s) {
            return None;
        }
        Some(self.uuid(s)).filter(|id| !id.is_nil())
    }

    /// The mode of a conversation sound (None for the other sounds).
    fn im_mode_of(&self, s: UiSound) -> Option<ImSoundMode> {
        match s {
            S::NewIncomingImSession => Some(self.im_mode),
            S::NewIncomingGroupImSession => Some(self.group_mode),
            S::NewIncomingConfImSession => Some(self.conf_mode),
            _ => None,
        }
    }

    /// The switch of a conversation sound in the per-sound list: on unless
    /// its mode is Mute (None for the other sounds).
    pub fn im_plays(&self, s: UiSound) -> Option<bool> {
        self.im_mode_of(s).map(|m| m != ImSoundMode::Mute)
    }

    /// Turned off, the mode becomes Mute; turned back on, the default mode
    /// (the Legacy Viewer's row is the mode combo, the rows above here). A
    /// switch already in that state keeps the chosen mode.
    pub fn set_im_plays(&mut self, s: UiSound, on: bool) {
        let mode = match s {
            S::NewIncomingImSession => &mut self.im_mode,
            S::NewIncomingGroupImSession => &mut self.group_mode,
            S::NewIncomingConfImSession => &mut self.conf_mode,
            _ => return,
        };
        if (*mode != ImSoundMode::Mute) != on {
            *mode = if on { ImSoundMode::default() } else { ImSoundMode::Mute };
        }
    }

    /// The sound of a conversation message, if its mode plays it
    /// (LLIMMgr::addMessage).
    pub fn im_sound(&self, m: ImMessage, focused: bool) -> Option<UiSound> {
        let (mode, sound) = match m.kind {
            ImKind::Private => (self.im_mode, S::NewIncomingImSession),
            ImKind::Group => (self.group_mode, S::NewIncomingGroupImSession),
            ImKind::Conference => (self.conf_mode, S::NewIncomingConfImSession),
        };
        mode.plays(m.new_session, focused).then_some(sound)
    }

    /// Every alert of the radar plays its sound.
    pub fn radar_sounds(&self) -> bool {
        RADAR_SOUNDS.into_iter().all(|s| self.plays(s))
    }

    /// Turn the seven alert sounds of the radar on or off at once (the entry
    /// of the radar's options menu; one by one in the preferences).
    pub fn set_radar_sounds(&mut self, on: bool) {
        for s in RADAR_SOUNDS {
            self.set_plays(s, on);
        }
    }

    /// The sounds loaded at login (init_audio preloads the UI sounds).
    pub fn preload(&self) -> Vec<Uuid> {
        let mut v: Vec<Uuid> = CATALOG.iter().map(|e| self.uuid(e.sound)).filter(|id| !id.is_nil()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// UISndMoneyChangeUp / Down: a balance change past the threshold
    /// (LLStatusBar::setBalance, the first balance is silent).
    pub fn money_sound(&self, old: Option<i32>, new: i32) -> Option<UiSound> {
        let old = old.filter(|&o| o != 0)?;
        if ((new - old) as f32).abs() <= self.money_threshold {
            return None;
        }
        Some(if new > old { S::MoneyChangeUp } else { S::MoneyChangeDown })
    }
}

/// Seconds between two tracker beacon sounds at `distance` metres
/// (FIRE-16969: closer = more frequent).
pub fn beacon_interval(distance: f32) -> f32 {
    distance.max(0.0) / 30.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_complete_and_unique() {
        let mut names: Vec<&str> = CATALOG.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CATALOG.len());
        for e in &CATALOG {
            assert_eq!(e.sound.entry().name, e.name);
        }
        for i in 0..8 {
            assert_eq!(UiSound::PieMenuSlice(i).entry().name, format!("PieMenuSliceHighlight{i}"));
        }
    }

    #[test]
    fn firestorm_defaults() {
        let s = UiSoundSettings::default();
        assert_eq!(s.resolve(S::ClickRelease, false), Some(CLICK));
        assert_eq!(s.resolve(S::NearbyChat, false), None);
        assert!(s.resolve(S::NearbyChat, true).is_some());
        assert_eq!(s.resolve(S::PieMenuHide, true), None);
        assert_eq!(s.resolve(S::NewIncomingGroupImSession, false), Some(NEW_SESSION));
        assert!(!s.preload().contains(&Uuid::nil()));
        assert_eq!(s.resolve(S::ObjectRezIn, false), Some(u(0x3c8fc726_1fd6_862d_fa01_16c5b2568db6)));
        // the radar's sounds: off, the nearby chat sound once turned on
        assert_eq!(s.resolve(S::RadarRegionEnter, false), None);
        assert_eq!(s.resolve(S::RadarAgeAlert, true), Some(NEARBY_CHAT));
        assert_eq!(S::RadarRegionLeave.entry().name, "RadarSimLeave");
    }

    #[test]
    fn the_radar_sounds_at_once() {
        let mut s = UiSoundSettings::default();
        assert!(!s.radar_sounds());
        s.set_plays(S::RadarChatEnter, true);
        // one of seven is not all of them
        assert!(!s.radar_sounds());
        s.set_radar_sounds(true);
        assert!(s.radar_sounds());
        assert!(RADAR_SOUNDS.iter().all(|r| s.resolve(*r, false) == Some(NEARBY_CHAT)));
        s.set_radar_sounds(false);
        assert!(s.play.is_empty());
    }

    #[test]
    fn overrides_only_store_changes() {
        let mut s = UiSoundSettings::default();
        s.set_plays(S::NearbyChat, true);
        s.set_plays(S::Click, true);
        assert_eq!(s.play.len(), 1);
        s.set_plays(S::NearbyChat, false);
        assert!(s.play.is_empty());
        let id = Uuid::from_u128(7);
        s.set_uuid(S::Alert, id);
        assert_eq!(s.resolve(S::Alert, false), Some(id));
        s.set_uuid(S::Alert, ALERT);
        assert!(s.uuid.is_empty());
        s.set_uuid(S::Alert, Uuid::nil());
        assert_eq!(s.resolve(S::Alert, false), None);
    }

    #[test]
    fn money_threshold() {
        let s = UiSoundSettings::default();
        assert_eq!(s.money_sound(None, 500), None);
        assert_eq!(s.money_sound(Some(0), 500), None);
        assert_eq!(s.money_sound(Some(100), 150), None);
        assert_eq!(s.money_sound(Some(100), 151), Some(S::MoneyChangeUp));
        assert_eq!(s.money_sound(Some(100), 10), Some(S::MoneyChangeDown));
    }

    #[test]
    fn im_modes() {
        use ImSoundMode as M;
        assert!(M::NewSession.plays(true, true));
        assert!(!M::NewSession.plays(false, false));
        assert!(M::EveryMessage.plays(false, true));
        assert!(M::NotFocused.plays(false, false));
        assert!(!M::NotFocused.plays(false, true));
        assert!(!M::Mute.plays(true, false));
    }

    #[test]
    fn im_switch_is_the_mode() {
        let mut s = UiSoundSettings::default();
        assert_eq!(s.im_plays(S::NewIncomingImSession), Some(true));
        assert_eq!(s.im_plays(S::Alert), None);
        s.group_mode = ImSoundMode::EveryMessage;
        // on stays on: the chosen mode is kept
        s.set_im_plays(S::NewIncomingGroupImSession, true);
        assert_eq!(s.group_mode, ImSoundMode::EveryMessage);
        s.set_im_plays(S::NewIncomingImSession, false);
        assert_eq!(s.im_mode, ImSoundMode::Mute);
        let m = ImMessage {
            kind: ImKind::Private,
            session: Uuid::nil(),
            new_session: true,
        };
        assert_eq!(s.im_sound(m, false), None);
        s.set_im_plays(S::NewIncomingImSession, true);
        assert_eq!(s.im_mode, ImSoundMode::NewSession);
        assert_eq!(s.im_sound(m, false), Some(S::NewIncomingImSession));
        // the other sounds have no mode
        s.set_im_plays(S::Alert, false);
        assert!(s.play.is_empty());
    }
}
