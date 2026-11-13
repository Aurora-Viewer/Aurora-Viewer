//! A contact set's own online notices and automatic responses.
//! Port of LGGContactSets::notifyForFriend, getAutoresponseForSet,
//! setAutoresponseForSet and getAutoresponseForFriend
//! (indra/newview/lggcontactsets.cpp, originally LGPL 2.1), the
//! FSContactSetsNotificationToast / FSContactSetsNotificationNearbyChat
//! choices of FSFloaterContactSetConfiguration
//! (indra/newview/fsfloatercontactsetconfiguration.cpp), and their use in
//! LLIMProcessing::getAutoresponseTextForAvatar (llimprocessing.cpp).
use super::*;

/// The status modes a set can answer with its own text
/// (ContactSetAutoresponseMode). The away response has none, as in the
/// Legacy Viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseMode {
    /// Do not disturb (BUSY).
    Busy,
    /// The automatic response mode (AUTORESPONSE).
    Autorespond,
    /// The automatic response to non-friends (AUTORESPONSE_NONFRIENDS).
    NonFriends,
}

impl ResponseMode {
    pub const ALL: [ResponseMode; 3] = [ResponseMode::Busy, ResponseMode::Autorespond, ResponseMode::NonFriends];

    fn index(self) -> usize {
        self as usize
    }

    /// The keys of the file (enabled, text), the Legacy Viewer's.
    pub(super) fn keys(self) -> (&'static str, &'static str) {
        match self {
            ResponseMode::Busy => ("autoresponse_busy_enabled", "autoresponse_busy"),
            ResponseMode::Autorespond => ("autoresponse_mode_enabled", "autoresponse_mode"),
            ResponseMode::NonFriends => ("autoresponse_nonfriends_enabled", "autoresponse_nonfriends"),
        }
    }
}

/// One of a set's custom responses: kept, text included, while it is off.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SetResponse {
    pub enabled: bool,
    pub text: String,
}

/// The global choices of the sets' notices; `notices_chat` is on by default
/// (FSContactSetsNotificationNearbyChat), `notices_toast` off
/// (FSContactSetsNotificationToast).
pub(super) const NOTICES_TOAST: &str = "aurora_notices_toast";
pub(super) const NOTICES_CHAT: &str = "aurora_notices_chat";

impl ContactSet {
    /// getAutoresponseForSet.
    pub fn response(&self, mode: ResponseMode) -> &SetResponse {
        &self.responses[mode.index()]
    }

    pub(super) fn read_responses(&mut self, value: &Llsd) {
        for mode in ResponseMode::ALL {
            let (enabled, text) = mode.keys();
            self.responses[mode.index()] = SetResponse {
                enabled: value[enabled].as_bool(),
                text: value[text].as_str().to_owned(),
            };
        }
    }

    /// exportToLLSD writes all six keys, used or not.
    pub(super) fn write_responses(&self, out: &mut Llsd) {
        for mode in ResponseMode::ALL {
            let (enabled, text) = mode.keys();
            let r = self.response(mode);
            out.insert(enabled, r.enabled);
            out.insert(text, r.text.clone());
        }
    }
}

impl ContactSets {
    /// The "Show notifications" choice of a set (mNotify).
    pub fn set_notify(&mut self, name: &str, on: bool) {
        if let Some(s) = self.sets.get_mut(name)
            && s.notify != on
        {
            s.notify = on;
            self.changed();
        }
    }

    /// notifyForFriend: one of `id`'s sets asks for their online notices.
    pub fn notify_for(&self, id: &Uuid) -> bool {
        self.sets.values().any(|s| s.notify && s.friends.contains(id))
    }

    /// "Show Online Notices for certain Sets as toast".
    pub fn set_notices_toast(&mut self, on: bool) {
        if self.notices_toast != on {
            self.notices_toast = on;
            self.changed();
        }
    }

    /// "Show Online Notices for certain Sets in nearby chat".
    pub fn set_notices_chat(&mut self, on: bool) {
        if self.notices_chat != on {
            self.notices_chat = on;
            self.changed();
        }
    }

    /// setAutoresponseForSet.
    pub fn set_response(&mut self, name: &str, mode: ResponseMode, response: SetResponse) {
        if let Some(s) = self.sets.get_mut(name)
            && s.responses[mode.index()] != response
        {
            s.responses[mode.index()] = response;
            self.changed();
        }
    }

    /// getAutoresponseForFriend: among `id`'s sets whose response for
    /// `mode` is on and not empty, the one of the set with the fewest
    /// members (the first by name on a tie, the order of the file's map).
    pub fn response_for(&self, id: &Uuid, mode: ResponseMode) -> Option<&str> {
        let mut best: Option<(usize, &str)> = None;
        for set in self.sets.values().filter(|s| s.friends.contains(id)) {
            let r = set.response(mode);
            if !r.enabled || r.text.is_empty() {
                continue;
            }
            if best.is_none_or(|(size, _)| set.friends.len() < size) {
                best = Some((set.friends.len(), &r.text));
            }
        }
        best.map(|(_, text)| text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn on(text: &str) -> SetResponse {
        SetResponse {
            enabled: true,
            text: text.into(),
        }
    }

    #[test]
    fn notices_follow_the_sets_of_the_friend() {
        let is_friend = |_: &Uuid| true;
        let mut cs = ContactSets::default();
        cs.add_set("Close");
        cs.add_set("Work");
        cs.add_to_set(&[id(1), id(2)], "Close", is_friend);
        cs.add_to_set(&[id(2), id(3)], "Work", is_friend);
        assert!(!cs.notify_for(&id(1)));
        cs.set_notify("Close", true);
        assert!(cs.notify_for(&id(1)) && cs.notify_for(&id(2)));
        assert!(!cs.notify_for(&id(3)), "Work does not ask for notices");
        assert!(!cs.notify_for(&id(4)), "in no set");
    }

    #[test]
    fn the_smallest_set_with_a_response_answers() {
        let is_friend = |_: &Uuid| true;
        let mut cs = ContactSets::default();
        for name in ["Big", "Small", "Empty text"] {
            cs.add_set(name);
        }
        cs.add_to_set(&[id(1), id(2), id(3)], "Big", is_friend);
        cs.add_to_set(&[id(1)], "Small", is_friend);
        cs.add_to_set(&[id(1)], "Empty text", is_friend);
        cs.set_response("Big", ResponseMode::Busy, on("Busy, big"));
        cs.set_response("Empty text", ResponseMode::Busy, on(""));
        assert_eq!(cs.response_for(&id(1), ResponseMode::Busy), Some("Busy, big"));
        // a response kept while it is off answers nothing
        cs.set_response(
            "Small",
            ResponseMode::Busy,
            SetResponse {
                enabled: false,
                text: "Busy, small".into(),
            },
        );
        assert_eq!(cs.response_for(&id(1), ResponseMode::Busy), Some("Busy, big"));
        cs.set_response("Small", ResponseMode::Busy, on("Busy, small"));
        assert_eq!(cs.response_for(&id(1), ResponseMode::Busy), Some("Busy, small"));
        assert_eq!(cs.response_for(&id(2), ResponseMode::Busy), Some("Busy, big"));
        assert_eq!(
            cs.response_for(&id(1), ResponseMode::Autorespond),
            None,
            "each mode has its own text"
        );
        assert_eq!(cs.response_for(&id(9), ResponseMode::Busy), None);
    }

    #[test]
    fn responses_and_notices_round_trip_the_file() {
        let is_friend = |_: &Uuid| true;
        let mut cs = ContactSets::default();
        cs.add_set("Close");
        cs.add_to_set(&[id(1)], "Close", is_friend);
        cs.set_notify("Close", true);
        cs.set_response("Close", ResponseMode::NonFriends, on("Hello stranger"));
        cs.set_notices_toast(true);
        cs.set_notices_chat(false);
        let xml = aurora_llsd::to_xml(&cs.export());
        let mut again = ContactSets::default();
        again.import(&aurora_llsd::from_xml(&xml).expect("valid xml"));
        assert_eq!(again.set("Close"), cs.set("Close"));
        assert!(again.notices_toast && !again.notices_chat);
        // the Legacy Viewer's keys, all six
        let text = String::from_utf8_lossy(&xml);
        for mode in ResponseMode::ALL {
            let (enabled, value) = mode.keys();
            assert!(text.contains(enabled) && text.contains(value));
        }
    }

    #[test]
    fn a_file_without_the_choices_keeps_their_defaults() {
        let mut cs = ContactSets::default();
        let xml = br#"<?xml version="1.0" ?><llsd><map>
            <key>globalSettings</key><map><key>defaultColor</key><array><real>0.5</real><real>0.5</real><real>0.5</real><real>1</real></array></map>
        </map></llsd>"#;
        cs.import(&aurora_llsd::from_xml(xml).expect("valid xml"));
        assert!(cs.notices_chat && !cs.notices_toast);
    }
}
