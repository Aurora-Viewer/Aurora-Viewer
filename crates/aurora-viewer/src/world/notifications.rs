//! Notification center: simulator alerts, offers (teleport, friendship,
//! inventory, group) and script requests (llDialog / llTextBox,
//! permissions, llLoadURL), with the answers SL expects
//! (LLOfferInfo, LLScriptFloater, LLPanelNotification in Firestorm).

use super::notification_history::{self as history, Entry, Follow, Look, Outcome};
use aurora_net::NetCommand;
use std::collections::HashMap;
use std::time::SystemTime;
use uuid::Uuid;

/// IM dialogs (llinstantmessage.h).
pub mod im {
    pub const MESSAGEBOX: u8 = 1;
    pub const GROUP_INVITATION: u8 = 3;
    pub const INVENTORY_OFFERED: u8 = 4;
    pub const INVENTORY_ACCEPTED: u8 = 5;
    pub const INVENTORY_DECLINED: u8 = 6;
    pub const TASK_INVENTORY_OFFERED: u8 = 9;
    pub const TASK_INVENTORY_ACCEPTED: u8 = 10;
    pub const TASK_INVENTORY_DECLINED: u8 = 11;
    pub const LURE_USER: u8 = 22;
    pub const LURE_ACCEPTED: u8 = 23;
    pub const LURE_DECLINED: u8 = 24;
    pub const GODLIKE_LURE_USER: u8 = 25;
    pub const TELEPORT_REQUEST: u8 = 26;
    pub const GOTO_URL: u8 = 28;
    pub const FROM_TASK_AS_ALERT: u8 = 31;
    pub const GROUP_NOTICE: u8 = 32;
    pub const GROUP_NOTICE_REQUESTED: u8 = 37;
    pub const GROUP_INVITATION_ACCEPT: u8 = 35;
    pub const GROUP_INVITATION_DECLINE: u8 = 36;
    pub const FRIENDSHIP_OFFERED: u8 = 38;
    pub const FRIENDSHIP_ACCEPTED: u8 = 39;
}

/// LSL permission bits (llRequestPermissions) with their wording
/// (SCRIPT_PERMISSIONS of llscriptruntimeperms.h).
fn permissions() -> &'static Vec<(i32, &'static str)> {
    static TABLE: aurora_i18n::Localized<Vec<(i32, &'static str)>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            (0x2, aurora_i18n::tr!("world-notifications-take-l-from-your-account")),
            (0x4, aurora_i18n::tr!("world-notifications-take-control-of-your-keys")),
            (0x8, aurora_i18n::tr!("world-notifications-remap-your-control-inputs")),
            (0x10, aurora_i18n::tr!("world-notifications-animate-your-avatar")),
            (0x20, aurora_i18n::tr!("world-notifications-attach-to-your-avatar")),
            (0x40, aurora_i18n::tr!("world-notifications-release-ownership")),
            (0x80, aurora_i18n::tr!("world-notifications-link-and-unlink-objects")),
            (0x100, aurora_i18n::tr!("world-notifications-add-and-remove-joints")),
            (0x200, aurora_i18n::tr!("world-notifications-change-its-permissions")),
            (0x400, aurora_i18n::tr!("world-notifications-follow-your-camera")),
            (0x800, aurora_i18n::tr!("world-notifications-control-your-camera")),
            (0x1000, aurora_i18n::tr!("world-notifications-teleport")),
            // 0x2000 (JoinAnExperience) has no line, like Firestorm's empty string
            (0x4000, aurora_i18n::tr!("world-notifications-manage-estate-silently")),
            (0x8000, aurora_i18n::tr!("world-notifications-override-your-animations")),
            (0x10000, aurora_i18n::tr!("world-notifications-return-objects")),
            (0x20000, aurora_i18n::tr!("world-notifications-force-your-avatar-to-sit")),
            (0x40000, aurora_i18n::tr!("world-notifications-change-your-environment-settings")),
            (0x80000, aurora_i18n::tr!("world-notifications-sell-land-on-your-behalf")),
        ]
    })
}

pub fn permission_lines(questions: i32) -> Vec<&'static str> {
    permissions().iter().filter(|(b, _)| questions & b != 0).map(|(_, t)| *t).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Alert,
    Teleport,
    Friendship,
    Inventory,
    Group,
    Script,
    Permissions,
    Url,
    /// A friend came online or went offline (FriendOnlineOffline).
    Presence,
    /// A message in a conversation that is not in sight (IMToast).
    Message,
    /// Someone calls (LLIncomingCallDialog).
    Call,
}

/// What an answer needs (a friendship offer's `from` becomes a friend when
/// accepted, formFriendship).
#[derive(Debug, Clone)]
pub enum Data {
    None,
    /// A teleport offer (IM_LURE_USER), answered by `World::answer_teleport_invite`.
    Lure {
        from: Uuid,
        invite: u64,
        /// The region is above the content rating: accepting changes it
        /// first ("Change and Continue").
        change_rating: bool,
    },
    /// A teleport request (IM_TELEPORT_REQUEST): Yes offers a teleport.
    TeleportRequest {
        from: Uuid,
        invite: u64,
    },
    Friend {
        from: Uuid,
        tx: Uuid,
        /// Offered while we were offline: answered by capability.
        offline: bool,
    },
    /// A resident offers their calling card (OfferCallingCard): the answer
    /// goes to the simulator of `region`.
    CallingCard {
        from: Uuid,
        tx: Uuid,
        region: aurora_net::RegionHandle,
    },
    Inventory {
        from: Uuid,
        tx: Uuid,
        asset_type: i8,
        /// Item or folder a resident's offer already put in our inventory
        /// (nil for objects' offers, delivered once accepted).
        object: Uuid,
        task: bool,
    },
    Group {
        group: Uuid,
        tx: Uuid,
        /// Received while we were offline (answered by capability when it
        /// came without a session id).
        offline: bool,
    },
    Dialog {
        object: Uuid,
        object_name: String,
        own_object: bool,
        channel: i32,
        buttons: Vec<String>,
    },
    TextBox {
        object: Uuid,
        channel: i32,
    },
    Permissions {
        task: Uuid,
        item: Uuid,
        questions: i32,
        /// The simulator that asked, which gets the answer.
        region: aurora_net::RegionHandle,
        /// The question of an experience (ScriptQuestionExperience).
        experience: Option<ExperiencePrompt>,
    },
    Url(String),
    /// A friend's online / offline notice: a toast only, never kept in the
    /// list (a notifytip with can_be_stored = false, llnotificationtiphandler.cpp).
    Presence {
        who: Uuid,
        online: bool,
    },
    /// The messages of a conversation that arrived while it was not in
    /// sight: one entry per conversation, updated by each new message.
    Message {
        session: Uuid,
        /// The sender of the last message (the picture of the card).
        from: Uuid,
        count: usize,
    },
    /// A voice call invitation (VoiceInviteP2P / VoiceInviteGroup /
    /// VoiceInviteAdHoc), answered by `World::respond_notification`.
    Call {
        invite: Box<aurora_net::VoiceInvite>,
    },
    /// The restart of the agent's region (RegionRestartMinutes /
    /// RegionRestartSeconds, show_toast="false"): pinned at the top of "To
    /// do" with the live count while the alert counts (`region_restart.rs`).
    RegionRestart,
}

/// What the question of an experience shows besides the permissions.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperiencePrompt {
    pub id: Uuid,
    /// The experience's name (its id when its details are unknown).
    pub name: String,
    /// The source object, blocked by "Block object".
    pub object_name: String,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u64,
    pub time: SystemTime,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub read: bool,
    pub data: Data,
    /// llTextBox reply being typed.
    pub text: String,
    /// Shown as a toast only: never listed under the bell nor counted, and
    /// dropped when its toast ends.
    pub transient: bool,
    /// Listed without a toast (a notification of Firestorm with
    /// show_toast="false"): it waits under the bell.
    pub quiet: bool,
    /// A region's rating (SIM_ACCESS_*) the entry is about, shown as the
    /// navigation bar's badge (0: none).
    pub maturity: u8,
}

impl Kind {
    pub fn look(self) -> Look {
        match self {
            Kind::Info | Kind::Presence => Look::Info,
            Kind::Alert => Look::Alert,
            Kind::Teleport => Look::Teleport,
            Kind::Friendship => Look::Friendship,
            Kind::Inventory => Look::Inventory,
            Kind::Group => Look::Group,
            Kind::Script => Look::Script,
            Kind::Permissions => Look::Permissions,
            Kind::Url => Look::Url,
            Kind::Message => Look::Message,
            Kind::Call => Look::Call,
        }
    }

    pub fn of_look(look: Look) -> Kind {
        match look {
            Look::Info => Kind::Info,
            Look::Alert => Kind::Alert,
            Look::Teleport => Kind::Teleport,
            Look::Friendship => Kind::Friendship,
            Look::Inventory => Kind::Inventory,
            Look::Group => Kind::Group,
            Look::Script => Kind::Script,
            Look::Permissions => Kind::Permissions,
            Look::Url => Kind::Url,
            Look::Message => Kind::Message,
            Look::Call => Kind::Call,
        }
    }
}

/// A place link in a text (a teleport offer's SLURL), for "Show the place".
fn place_link(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.starts_with("secondlife://") || w.contains("maps.secondlife.com/secondlife/"))
        .map(|w| w.trim_end_matches([')', ',', '.', ']']).to_owned())
}

impl Notification {
    /// Its history entry, once it left the list with `outcome`: what it was,
    /// and the action still useful (an offer gone cannot be accepted, what
    /// it was about can be opened).
    pub fn history_entry(&self, outcome: Outcome) -> Entry {
        let follow = match &self.data {
            Data::None => Follow::Restore,
            Data::Message { session, .. } => Follow::Conversation(*session),
            Data::Lure { from, .. } => place_link(&self.body).map_or(Follow::Conversation(*from), Follow::Place),
            Data::TeleportRequest { from, .. } | Data::Friend { from, .. } | Data::CallingCard { from, .. } => Follow::Conversation(*from),
            Data::Call { invite } if invite.p2p => Follow::Conversation(invite.from),
            Data::Call { invite } => Follow::Conversation(invite.session),
            Data::Inventory { object, .. } if !object.is_nil() && outcome == Outcome::Kept => Follow::Inventory(*object),
            _ => Follow::None,
        };
        Entry {
            time: history::secs(self.time),
            look: self.kind.look(),
            title: self.title.clone(),
            body: self.body.clone(),
            outcome,
            follow,
        }
    }

    /// Needs an answer (stays until answered).
    pub fn interactive(&self) -> bool {
        !matches!(self.data, Data::None | Data::Presence { .. } | Data::Message { .. })
    }

    /// Clicking the card itself acts (opens the conversation): a friend
    /// online opens an IM (LLAvatarActions::startIM, respond_on_mousedown),
    /// a message its conversation.
    pub fn opens(&self) -> bool {
        matches!(self.data, Data::Presence { online: true, .. } | Data::Message { .. })
    }

    /// Closing it answers "no"; a link has no answer, a resident's
    /// inventory offer is kept and a calling card offer gets no answer
    /// (callingcard_offer_callback: "close button probably").
    pub fn close_declines(&self) -> bool {
        match &self.data {
            Data::None | Data::Url(_) | Data::Presence { .. } | Data::Message { .. } | Data::RegionRestart | Data::CallingCard { .. } => {
                false
            }
            Data::Inventory { object, .. } => object.is_nil(),
            _ => true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Response {
    Dismiss,
    /// Block the source object and discard its script menus, without replying
    /// (a permission question is refused first, script_question_mute).
    Block,
    /// Refuse an experience's question and block the experience
    /// (script_question_cb, "BlockExperience").
    BlockExperience,
    /// Open the profile of the experience; the question stays.
    OpenExperience(Uuid),
    Accept,
    Decline,
    /// Script dialog button (index).
    Button(usize),
    /// llTextBox: send the typed text.
    SendText,
    OpenUrl,
    /// The card itself was clicked: open what it is about (`Notification::opens`).
    Open,
}

/// Which toasts the preferences allow (Preferences › Chat › Toasts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastPrefs {
    /// Friends online / offline (ChatOnlineNotification).
    pub presence: bool,
    /// Messages of IMs and conferences (EnableIMChatPopups).
    pub im: bool,
    /// Messages of group chats (EnableGroupChatPopups).
    pub group_im: bool,
}

impl Default for ToastPrefs {
    fn default() -> Self {
        ToastPrefs {
            presence: true,
            im: true,
            group_im: false,
        }
    }
}

#[derive(Default)]
pub struct Notifications {
    pub list: Vec<Notification>,
    next: u64,
    /// Our legacy full name ("First Last"), for the answers that carry it.
    pub own_name: String,
    pub toasts: ToastPrefs,
    /// The world is shown (the loading screen gone). Before that, the
    /// friends already online that the server announces at login get no
    /// toast: Firestorm's tip channel drops them until handleLoginComplete
    /// (LLScreenChannel::addToast, mWasStartUpToastShown).
    pub started: bool,
    /// Aurora's notification center (Preferences › Aurora): in do not
    /// disturb its bell is paused but everything still arrives in "To do";
    /// Firestorm's style drops the messages' entries then.
    pub aurora: bool,
    /// What became of the notifications that left the list (answered,
    /// declined…), for their history entry.
    outcomes: HashMap<u64, Outcome>,
    /// The listed notifications last time `take_departures` looked.
    seen: Vec<(u64, Entry)>,
}

const MAX: usize = 100;

impl Notifications {
    pub fn push(&mut self, kind: Kind, title: impl Into<String>, body: impl Into<String>, data: Data) -> u64 {
        self.next += 1;
        self.list.push(Notification {
            id: self.next,
            time: SystemTime::now(),
            kind,
            title: title.into(),
            body: body.into(),
            read: false,
            data,
            text: String::new(),
            transient: false,
            quiet: false,
            maturity: 0,
        });
        if self.list.len() > MAX {
            // drop the oldest that needs no answer
            if let Some(i) = self.list.iter().position(|n| !n.interactive()) {
                self.list.remove(i);
            } else {
                self.list.remove(0);
            }
        }
        self.next
    }

    /// A friend's online / offline toast: never kept in the list, so that a
    /// login among many friends does not fill the bell.
    pub fn push_presence(&mut self, who: Uuid, text: impl Into<String>, online: bool) -> u64 {
        let id = self.push(Kind::Presence, text, "", Data::Presence { who, online });
        if let Some(n) = self.list.iter_mut().find(|n| n.id == id) {
            n.transient = true;
        }
        id
    }

    /// A message of a conversation out of sight: it replaces the entry of
    /// that conversation still waiting (a new toast, the count of messages
    /// since it was last read), so that a chatty friend fills one card.
    pub fn push_message(&mut self, session: Uuid, from: Uuid, title: impl Into<String>, text: impl Into<String>) -> u64 {
        let mut count = 1;
        let mut before = None;
        if let Some(i) = self
            .list
            .iter()
            .position(|n| matches!(n.data, Data::Message { session: s, .. } if s == session))
        {
            let old = self.list.remove(i);
            before = Some(old.id);
            if let Data::Message { count: c, .. } = old.data {
                count += c;
            }
        }
        let id = self.push(Kind::Message, title, text, Data::Message { session, from, count });
        // the same entry, not one gone to the history
        if let Some(seen) = self.seen.iter_mut().find(|(i, _)| Some(*i) == before) {
            seen.0 = id;
        }
        id
    }

    /// Forget the message entries of the conversations now read (opened in
    /// the Conversations window): the bell does not keep what was seen.
    pub fn acknowledge_messages(&mut self, read: impl Fn(&Uuid) -> bool) {
        self.list
            .retain(|n| !matches!(n.data, Data::Message { session, .. } if read(&session)));
    }

    /// The listed entries (the toast-only ones left out).
    pub fn listed(&self) -> impl Iterator<Item = &Notification> {
        self.list.iter().filter(|n| !n.transient)
    }

    pub fn unread(&self) -> usize {
        self.listed().filter(|n| !n.read).count()
    }

    pub fn mark_all_read(&mut self) {
        for n in self.list.iter_mut() {
            n.read = true;
        }
    }

    /// Remove everything that does not wait for an answer.
    pub fn clear_info(&mut self) {
        self.list.retain(|n| n.interactive());
    }

    /// Record what became of `id` before it leaves the list some other way
    /// than `respond` (a card answered in a conversation, a block).
    pub fn note(&mut self, id: u64, outcome: Outcome) {
        self.outcomes.insert(id, outcome);
    }

    /// Put an entry of the history back in the list ("Put back in To do").
    pub fn restore(&mut self, e: &Entry) -> u64 {
        let id = self.push(Kind::of_look(e.look), e.title.clone(), e.body.clone(), Data::None);
        if let Some(n) = self.list.iter_mut().find(|n| n.id == id) {
            n.time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(e.time);
        }
        id
    }

    /// The listed notifications that left the list since the last call,
    /// as history entries (the toast-only ones are never kept).
    pub fn take_departures(&mut self) -> Vec<Entry> {
        let now: Vec<u64> = self.listed().map(|n| n.id).collect();
        if now.len() == self.seen.len() && now.iter().zip(&self.seen).all(|(a, (b, _))| a == b) {
            return Vec::new();
        }
        let mut gone = Vec::new();
        for (id, mut entry) in std::mem::take(&mut self.seen) {
            if now.contains(&id) {
                continue;
            }
            if let Some(o) = self.outcomes.remove(&id) {
                entry.outcome = o;
            }
            if entry.outcome != Outcome::Kept && matches!(entry.follow, Follow::Inventory(_)) {
                entry.follow = Follow::None;
            }
            gone.push(entry);
        }
        // what each entry becomes when it leaves without a recorded answer:
        // information read, an offer left unanswered ignored
        self.seen = self
            .listed()
            .map(|n| {
                let mut e = n.history_entry(if n.interactive() { Outcome::Ignored } else { Outcome::Read });
                if let Data::Inventory { object, .. } = n.data
                    && !object.is_nil()
                {
                    e.follow = Follow::Inventory(object);
                }
                (n.id, e)
            })
            .collect();
        gone
    }

    /// Apply an answer: returns the messages to send and an URL to open.
    /// `folder_for` gives the inventory folder for an asset type (offers)
    /// and `calling_cards` the calling card folder (friendship).
    pub fn respond(
        &mut self,
        id: u64,
        r: Response,
        folder_for: impl Fn(i8) -> Uuid,
        calling_cards: Uuid,
    ) -> (Vec<NetCommand>, Option<String>) {
        let Some(pos) = self.list.iter().position(|n| n.id == id) else {
            return (Vec::new(), None);
        };
        let n = self.list.remove(pos);
        self.outcomes.insert(id, outcome_of(&n, &r));
        let mut out = Vec::new();
        let mut url = None;
        let accept = matches!(r, Response::Accept);
        match (&n.data, &r) {
            (_, Response::Dismiss) => {
                // closing an offer declines it, like Firestorm's toasts
                match &n.data {
                    Data::Friend { from, tx, offline } => out.push(self.friend_answer(*from, *tx, *offline, false, calling_cards)),
                    // Closing a resident's offer refuses nothing: what it
                    // delivered stays (inventory_offer_callback, "close
                    // button probably"), and the giver reads "accepted" as
                    // with Firestorm's answer on arrival. An object's
                    // offer is declined.
                    Data::Inventory {
                        from,
                        tx,
                        asset_type,
                        object,
                        task,
                    } => {
                        let keep = !object.is_nil();
                        let folder = if keep { folder_for(*asset_type) } else { Uuid::nil() };
                        out.push(inventory_answer(*from, *tx, *task, keep, folder));
                    }
                    Data::Group { group, tx, offline } => out.push(group_answer(*group, *tx, *offline, false)),
                    Data::Dialog { .. } | Data::TextBox { .. } => {} // ignore = no reply
                    _ => {}
                }
            }
            (Data::Friend { from, tx, offline }, _) => out.push(self.friend_answer(*from, *tx, *offline, accept, calling_cards)),
            // callingcard_offer_callback: Accept into the Calling Cards
            // folder, Decline
            (Data::CallingCard { tx, region, .. }, _) => out.push(NetCommand::AnswerCallingCard {
                handle: *region,
                transaction: *tx,
                folder: accept.then_some(calling_cards),
            }),
            (
                Data::Inventory {
                    from,
                    tx,
                    asset_type,
                    task,
                    ..
                },
                _,
            ) => {
                let folder = if accept { folder_for(*asset_type) } else { Uuid::nil() };
                out.push(inventory_answer(*from, *tx, *task, accept, folder));
            }
            (Data::Group { group, tx, offline }, _) => out.push(group_answer(*group, *tx, *offline, accept)),
            (
                Data::Dialog {
                    object, channel, buttons, ..
                },
                Response::Button(i),
            ) => {
                if let Some(label) = buttons.get(*i) {
                    out.push(NetCommand::ScriptDialogReply {
                        object_id: *object,
                        channel: *channel,
                        index: *i as i32,
                        label: label.clone(),
                    });
                }
            }
            (Data::TextBox { object, channel }, Response::SendText) => {
                out.push(NetCommand::ScriptDialogReply {
                    object_id: *object,
                    channel: *channel,
                    index: 0,
                    label: n.text.clone(),
                });
            }
            (
                Data::Permissions {
                    task,
                    item,
                    questions,
                    region,
                    ..
                },
                _,
            ) => {
                // ScriptAnswerYes with the granted bits (0 = refuse), to the
                // simulator that asked
                out.push(NetCommand::ScriptAnswer {
                    task_id: *task,
                    item_id: *item,
                    questions: if accept { *questions } else { 0 },
                    region: *region,
                });
            }
            (Data::Url(u), Response::OpenUrl) => url = Some(u.clone()),
            _ => {}
        }
        (out, url)
    }

    /// Answer to a friendship offer (friendship_offer_callback): one made
    /// while we were offline goes through the region's capability, which
    /// needs who offered and our name instead of a transaction id.
    fn friend_answer(&self, from: Uuid, tx: Uuid, offline: bool, accept: bool, calling_cards: Uuid) -> NetCommand {
        if offline {
            NetCommand::AnswerOfflineFriendship {
                from,
                transaction: tx,
                folder: calling_cards,
                accept,
                agent_name: self.own_name.clone(),
            }
        } else if accept {
            NetCommand::AcceptFriendship {
                transaction: tx,
                folder: calling_cards,
            }
        } else {
            NetCommand::DeclineFriendship { transaction: tx }
        }
    }
}

/// What an answer did, for the history.
fn outcome_of(n: &Notification, r: &Response) -> Outcome {
    match r {
        Response::Accept if matches!(n.data, Data::Inventory { .. }) => Outcome::Kept,
        Response::Accept | Response::Button(_) | Response::SendText | Response::OpenUrl | Response::Open => Outcome::Answered,
        Response::Decline | Response::Block | Response::BlockExperience => Outcome::Declined,
        Response::Dismiss => match &n.data {
            Data::Dialog { .. } | Data::TextBox { .. } => Outcome::Ignored,
            Data::Inventory { object, .. } if !object.is_nil() => Outcome::Kept,
            _ if n.close_declines() => Outcome::Declined,
            _ => Outcome::Read,
        },
        Response::OpenExperience(_) => Outcome::Read,
    }
}

/// LLAssetType::AT_CATEGORY: the offer is a folder.
pub const AT_CATEGORY: i8 = 8;

/// Binary bucket of a resident's inventory offer (offer_agent_bucket_t,
/// process_improved_im in indra/newview/llimprocessing.cpp, originally
/// LGPL 2.1): the asset type, then the id of the item or folder. The server
/// has already copied it into our inventory when the offer arrives, so the
/// id is the one to fetch (inventory_offer_handler) or to discard.
pub fn offered_object(bucket: &[u8]) -> Option<(i8, Uuid)> {
    let (asset_type, id) = bucket.split_first()?;
    let id = Uuid::from_slice(id).ok().filter(|id| !id.is_nil())?;
    Some((*asset_type as i8, id))
}

pub(crate) fn inventory_answer(from: Uuid, tx: Uuid, task: bool, accept: bool, folder: Uuid) -> NetCommand {
    let dialog = match (task, accept) {
        (false, true) => im::INVENTORY_ACCEPTED,
        (false, false) => im::INVENTORY_DECLINED,
        (true, true) => im::TASK_INVENTORY_ACCEPTED,
        (true, false) => im::TASK_INVENTORY_DECLINED,
    };
    NetCommand::SendImDialog {
        to: from,
        dialog,
        id: tx,
        message: String::new(),
        // the destination folder travels in the binary bucket
        bucket: if accept { folder.as_bytes().to_vec() } else { Vec::new() },
    }
}

/// send_join_group_response: an invitation received while offline without
/// a session id (the offline messages capability gives none) is answered by
/// capability, the others by instant message.
fn group_answer(group: Uuid, tx: Uuid, offline: bool, accept: bool) -> NetCommand {
    if offline && tx.is_nil() {
        return NetCommand::AnswerOfflineGroupInvite { group, accept };
    }
    NetCommand::SendImDialog {
        to: group,
        dialog: if accept {
            im::GROUP_INVITATION_ACCEPT
        } else {
            im::GROUP_INVITATION_DECLINE
        },
        id: tx,
        message: String::new(),
        bucket: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_button_reply_and_removal() {
        let mut n = Notifications::default();
        let obj = Uuid::new_v4();
        let id = n.push(
            Kind::Script,
            "Menu",
            "Choisis",
            Data::Dialog {
                object: obj,
                object_name: "Menu".into(),
                own_object: false,
                channel: -42,
                buttons: vec![aurora_i18n::tr!("common-yes").into(), aurora_i18n::tr!("common-no").into()],
            },
        );
        assert_eq!(n.unread(), 1);
        let (cmds, _) = n.respond(id, Response::Button(1), |_| Uuid::nil(), Uuid::nil());
        assert!(
            matches!(&cmds[..], [NetCommand::ScriptDialogReply { channel: -42, index: 1, label, .. }] if label == aurora_i18n::tr!("common-no"))
        );
        assert!(n.list.is_empty());
    }

    #[test]
    fn inventory_accept_puts_folder_in_bucket() {
        let mut n = Notifications::default();
        let (from, tx, folder) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let id = n.push(
            Kind::Inventory,
            aurora_i18n::tr!("common-object"),
            "",
            Data::Inventory {
                from,
                tx,
                asset_type: 6,
                object: Uuid::nil(),
                task: false,
            },
        );
        let (cmds, _) = n.respond(id, Response::Accept, |t| if t == 6 { folder } else { Uuid::nil() }, Uuid::nil());
        assert!(
            matches!(&cmds[..], [NetCommand::SendImDialog { dialog: im::INVENTORY_ACCEPTED, bucket, .. }] if bucket[..] == folder.as_bytes()[..])
        );
    }

    #[test]
    fn offline_offers_are_answered_by_capability() {
        let mut n = Notifications {
            own_name: "Aurora Demo".into(),
            ..Default::default()
        };
        let (from, cards) = (Uuid::from_u128(7), Uuid::from_u128(8));
        // a friendship offer made while we were away has no transaction id
        let offer = |n: &mut Notifications, tx: Uuid, offline: bool| n.push(Kind::Friendship, "", "", Data::Friend { from, tx, offline });
        let id = offer(&mut n, Uuid::nil(), true);
        let (cmds, _) = n.respond(id, Response::Accept, |_| Uuid::nil(), cards);
        assert!(matches!(
            &cmds[..],
            [NetCommand::AnswerOfflineFriendship { from: f, transaction, folder, accept: true, agent_name }]
                if *f == from && transaction.is_nil() && *folder == cards && agent_name == "Aurora Demo"
        ));
        // closing it declines it, the same way
        let id = offer(&mut n, Uuid::nil(), true);
        let (cmds, _) = n.respond(id, Response::Dismiss, |_| Uuid::nil(), cards);
        assert!(matches!(&cmds[..], [NetCommand::AnswerOfflineFriendship { accept: false, .. }]));
        // an offer received while online keeps the message
        let tx = Uuid::from_u128(9);
        let id = offer(&mut n, tx, false);
        let (cmds, _) = n.respond(id, Response::Accept, |_| Uuid::nil(), cards);
        assert!(matches!(&cmds[..], [NetCommand::AcceptFriendship { transaction, folder }] if *transaction == tx && *folder == cards));

        // group invitations: by capability only when offline and without a
        // session id (what the offline messages capability gives)
        let group = Uuid::from_u128(0x6200);
        let invite = |n: &mut Notifications, tx: Uuid, offline: bool| n.push(Kind::Group, "", "", Data::Group { group, tx, offline });
        let id = invite(&mut n, Uuid::nil(), true);
        let (cmds, _) = n.respond(id, Response::Accept, |_| Uuid::nil(), cards);
        assert!(matches!(&cmds[..], [NetCommand::AnswerOfflineGroupInvite { group: g, accept: true }] if *g == group));
        let id = invite(&mut n, Uuid::nil(), true);
        let (cmds, _) = n.respond(id, Response::Decline, |_| Uuid::nil(), cards);
        assert!(matches!(&cmds[..], [NetCommand::AnswerOfflineGroupInvite { accept: false, .. }]));
        // offline but with a session id (the legacy request): by message
        for (tx, offline) in [(tx, true), (tx, false)] {
            let id = invite(&mut n, tx, offline);
            let (cmds, _) = n.respond(id, Response::Accept, |_| Uuid::nil(), cards);
            assert!(matches!(&cmds[..], [NetCommand::SendImDialog { dialog: im::GROUP_INVITATION_ACCEPT, id, .. }] if *id == tx));
        }
    }

    /// What leaves the list goes to the history with its outcome; a
    /// friend's toast never does; information can be put back.
    #[test]
    fn departures_reach_the_history_with_their_outcome() {
        let mut n = Notifications::default();
        let (from, tx, object) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let info = n.push(Kind::Info, "Lune paid you L$50", "", Data::None);
        let offer = n.push(
            Kind::Inventory,
            "Orion gives you a lantern",
            "",
            Data::Inventory {
                from,
                tx,
                asset_type: 6,
                object,
                task: false,
            },
        );
        n.push_presence(from, "Orion is online.", true);
        assert!(n.take_departures().is_empty());
        n.respond(offer, Response::Accept, |_| Uuid::nil(), Uuid::nil());
        n.clear_info();
        let gone = n.take_departures();
        assert_eq!(gone.len(), 2);
        let kept = gone.iter().find(|e| e.title.starts_with("Orion")).expect("offer");
        assert_eq!((kept.outcome, &kept.follow), (Outcome::Kept, &Follow::Inventory(object)));
        let read = gone.iter().find(|e| e.title.starts_with("Lune")).expect("info");
        assert_eq!((read.outcome, &read.follow), (Outcome::Read, &Follow::Restore));
        assert!(n.list.iter().all(|x| x.id != info));
        // put back in "To do"
        let back = n.restore(read);
        assert!(n.listed().any(|x| x.id == back && x.title == read.title));
        // a refused teleport offer keeps a way to its place
        let lure = n.push(
            Kind::Teleport,
            "Tess offers a teleport",
            "Lagoon secondlife:///app/teleport/Aurora/128/130/25",
            Data::Lure {
                from,
                invite: 9,
                change_rating: false,
            },
        );
        n.take_departures();
        n.note(lure, Outcome::Declined);
        n.list.retain(|x| x.id != lure);
        let gone = n.take_departures();
        assert_eq!(gone[0].outcome, Outcome::Declined);
        assert_eq!(gone[0].follow, Follow::Place("secondlife:///app/teleport/Aurora/128/130/25".into()));
    }

    #[test]
    fn offered_object_needs_the_type_and_a_full_id() {
        let id = Uuid::from_u128(0x1234_5678);
        let mut bucket = vec![5u8];
        bucket.extend_from_slice(id.as_bytes());
        assert_eq!(offered_object(&bucket), Some((5, id)));
        // objects' offers only carry the type; a nil id is nothing to fetch
        assert_eq!(offered_object(&[6]), None);
        assert_eq!(offered_object(&[]), None);
        assert_eq!(offered_object(&[0; 17]), None);
        bucket.push(0);
        assert_eq!(offered_object(&bucket), None);
    }

    #[test]
    fn permissions_refused_sends_zero() {
        let mut n = Notifications::default();
        let id = n.push(
            Kind::Permissions,
            "",
            "",
            Data::Permissions {
                task: Uuid::nil(),
                item: Uuid::nil(),
                questions: 0x10,
                region: 42,
                experience: None,
            },
        );
        let (cmds, _) = n.respond(id, Response::Decline, |_| Uuid::nil(), Uuid::nil());
        assert!(matches!(
            &cmds[..],
            [NetCommand::ScriptAnswer {
                questions: 0,
                region: 42,
                ..
            }]
        ));
        assert_eq!(
            permission_lines(0x10 | 0x20),
            vec![
                aurora_i18n::tr!("world-notifications-animate-your-avatar"),
                aurora_i18n::tr!("world-notifications-attach-to-your-avatar")
            ]
        );
    }
}
