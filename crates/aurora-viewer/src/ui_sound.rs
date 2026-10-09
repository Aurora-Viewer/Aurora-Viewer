//! Interface sounds, after Firestorm's UISnd* settings
//! (indra/newview/app_settings/settings.xml, defaults of `UISnd<name>` and
//! `PlayModeUISnd<name>`), find_ui_sound / make_ui_sound
//! (indra/llui/llui.cpp) and the UI sounds panel
//! (indra/newview/fspanelpreferenceuisounds.cpp); originally LGPL 2.1,
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project.
//!
//! Each sound has Firestorm's asset UUID and "play" default; both can be
//! changed per sound in Préférences › Son et voix. The sounds are SL assets
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
    PieMenuAppear,
    PieMenuHide,
    /// UISndPieMenuSliceHighlight0..7.
    PieMenuSlice(u8),
    Restart,
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

/// Catalog entry: Firestorm setting name (without `UISnd`), French label,
/// default UUID and default `PlayModeUISnd<name>`.
pub struct Entry {
    pub sound: UiSound,
    pub name: &'static str,
    pub label: &'static str,
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

macro_rules! e {
    ($s:expr, $n:literal, $l:literal, $u:expr, $p:literal) => {
        Entry {
            sound: $s,
            name: $n,
            label: $l,
            uuid: $u,
            play: $p,
        }
    };
}

use UiSound as S;

/// Every interface sound, in the order of the preferences list.
#[rustfmt::skip]
pub const CATALOG: [Entry; 40] = [
    e!(S::Click, "Click", "Clic (bouton enfoncé)", CLICK, true),
    e!(S::ClickRelease, "ClickRelease", "Clic (bouton relâché)", CLICK, true),
    e!(S::BadKeystroke, "BadKeystroke", "Touche refusée", u(0x2ca849ba_2885_4bc3_90ef_d4987a5b983a), true),
    e!(S::WindowOpen, "WindowOpen", "Fenêtre ouverte", WINDOW_OPEN, true),
    e!(S::WindowClose, "WindowClose", "Fenêtre fermée", WINDOW_CLOSE, true),
    e!(S::ScriptFloaterOpen, "ScriptFloaterOpen", "Dialogue de script ouvert", WINDOW_OPEN, true),
    e!(S::ScriptFloaterClose, "ScriptFloaterClose", "Dialogue de script fermé", WINDOW_CLOSE, true),
    e!(S::PieMenuAppear, "PieMenuAppear", "Menu clic droit ouvert", u(0x8eaed61f_92ff_6485_de83_4dcc938a478e), true),
    e!(S::PieMenuHide, "PieMenuHide", "Menu clic droit fermé", Uuid::nil(), true),
    e!(S::PieMenuSlice(0), "PieMenuSliceHighlight0", "Menu : choix 1 survolé", PIE_SLICE_0, true),
    e!(S::PieMenuSlice(1), "PieMenuSliceHighlight1", "Menu : choix 2 survolé", u(0xf6ba9816_dcaf_f755_7b67_51b31b6233e5), true),
    e!(S::PieMenuSlice(2), "PieMenuSliceHighlight2", "Menu : choix 3 survolé", u(0x7aff2265_d05b_8b72_63c7_dbf96dc2f21f), true),
    e!(S::PieMenuSlice(3), "PieMenuSliceHighlight3", "Menu : choix 4 survolé", u(0x09b2184e_8601_44e2_afbb_ce37434b8ba1), true),
    e!(S::PieMenuSlice(4), "PieMenuSliceHighlight4", "Menu : choix 5 survolé", u(0xbbe4c7fc_7044_b05e_7b89_36924a67593c), true),
    e!(S::PieMenuSlice(5), "PieMenuSliceHighlight5", "Menu : choix 6 survolé", u(0xd166039b_b4f5_c2ec_4911_c85c727b016c), true),
    e!(S::PieMenuSlice(6), "PieMenuSliceHighlight6", "Menu : choix 7 survolé", u(0x242af82b_43c2_9a3b_e108_3b0c7e384981), true),
    e!(S::PieMenuSlice(7), "PieMenuSliceHighlight7", "Menu : choix 8 survolé", u(0xc1f334fb_a5be_8fe7_22b3_29631c21cf0b), true),
    e!(S::Alert, "Alert", "Alerte", ALERT, true),
    e!(S::Restart, "Restart", "Redémarrage de la région", u(0xb92a0f64_7709_8811_40c5_16afd624a45f), true),
    e!(S::TeleportOut, "TeleportOut", "Téléportation", u(0xd7a9a565_a013_2a69_797d_5332baa1a947), true),
    e!(S::MoneyChangeUp, "MoneyChangeUp", "L$ reçus", u(0x77a018af_098e_c037_51a6_178f05877c6f), true),
    e!(S::MoneyChangeDown, "MoneyChangeDown", "L$ dépensés", u(0x104974e3_dfda_428b_99ee_b0d4e748d3a3), true),
    e!(S::StartIm, "StartIM", "Démarrer une conversation", START_IM, true),
    e!(S::NewIncomingImSession, "NewIncomingIMSession", "Message privé reçu", NEW_SESSION, true),
    e!(S::NewIncomingGroupImSession, "NewIncomingGroupIMSession", "Message de groupe reçu", NEW_SESSION, true),
    e!(S::NewIncomingConfImSession, "NewIncomingConfIMSession", "Message de conférence reçu", NEW_SESSION, true),
    e!(S::NearbyChat, "NearbyChat", "Chat local reçu", u(0xa3f48b85_c29f_1f97_ebb6_644b7c053512), false),
    e!(S::Typing, "Typing", "Avatars qui écrivent", u(0x5e191c7b_8996_9ced_a177_b2ac32bfea06), true),
    e!(S::FriendOnline, "FriendOnline", "Ami en ligne", ALERT, false),
    e!(S::FriendOffline, "FriendOffline", "Ami hors ligne", ALERT, false),
    e!(S::FriendshipOffer, "FriendshipOffer", "Offre d'amitié", NEW_SESSION, false),
    e!(S::TeleportOffer, "TeleportOffer", "Offre de téléportation", NEW_SESSION, false),
    e!(S::InventoryOffer, "InventoryOffer", "Objet offert", NEW_SESSION, false),
    e!(S::GroupInvitation, "GroupInvitation", "Invitation dans un groupe", START_IM, false),
    e!(S::GroupNotice, "GroupNotice", "Avis de groupe", START_IM, false),
    e!(S::ObjectCreate, "ObjectCreate", "Création d'un objet", u(0xf4a0660f_5446_dea2_80b7_6482a082803c), true),
    e!(S::ObjectDelete, "ObjectDelete", "Suppression d'un objet", u(0x0cb7b00a_4c10_6948_84de_a93c09af2ba9), false),
    e!(S::ObjectRezOut, "ObjectRezOut", "Objet pris", Uuid::nil(), true),
    e!(S::MicToggle, "MicToggle", "Micro activé / coupé", PIE_SLICE_0, false),
    e!(S::TrackerBeacon, "TrackerBeacon", "Balise de destination", ALERT, false),
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
            Self::Mute => "Jamais",
            Self::NewSession => "Nouvelle conversation",
            Self::EveryMessage => "Chaque message",
            Self::NotFocused => "Si la conversation n'est pas au premier plan",
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
}
