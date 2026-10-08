//! Notification center: simulator alerts, offers (teleport, friendship,
//! inventory, group) and script requests (llDialog / llTextBox,
//! permissions, llLoadURL), with the answers SL expects
//! (LLOfferInfo, LLScriptFloater, LLPanelNotification in Firestorm).

use aurora_net::NetCommand;
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
    pub const GOTO_URL: u8 = 28;
    pub const FROM_TASK_AS_ALERT: u8 = 31;
    pub const GROUP_NOTICE: u8 = 32;
    pub const GROUP_INVITATION_ACCEPT: u8 = 35;
    pub const GROUP_INVITATION_DECLINE: u8 = 36;
    pub const FRIENDSHIP_OFFERED: u8 = 38;
    pub const FRIENDSHIP_ACCEPTED: u8 = 39;
}

/// LSL permission bits (llRequestPermissions) with their French wording.
const PERMISSIONS: &[(i32, &str)] = &[
    (0x2, "Prendre des L$ de votre compte"),
    (0x4, "Prendre le contrôle de vos touches"),
    (0x10, "Animer votre avatar"),
    (0x20, "S'attacher à votre avatar"),
    (0x80, "Lier et délier des objets"),
    (0x400, "Suivre votre caméra"),
    (0x800, "Contrôler votre caméra"),
    (0x1000, "Vous téléporter"),
    (0x4000, "Gérer le domaine en silence"),
    (0x8000, "Remplacer vos animations"),
    (0x10000, "Renvoyer des objets"),
];

pub fn permission_lines(questions: i32) -> Vec<&'static str> {
    PERMISSIONS.iter().filter(|(b, _)| questions & b != 0).map(|(_, t)| *t).collect()
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
}

#[derive(Debug, Clone)]
pub enum Data {
    None,
    Lure { from: Uuid, lure: Uuid },
    Friend { tx: Uuid },
    Inventory { from: Uuid, tx: Uuid, asset_type: i8, task: bool },
    Group { group: Uuid, tx: Uuid },
    Dialog { object: Uuid, channel: i32, buttons: Vec<String> },
    TextBox { object: Uuid, channel: i32 },
    Permissions { task: Uuid, item: Uuid, questions: i32 },
    Url(String),
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
}

impl Notification {
    /// Needs an answer (stays until answered).
    pub fn interactive(&self) -> bool {
        !matches!(self.data, Data::None)
    }
}

#[derive(Debug, Clone)]
pub enum Response {
    Dismiss,
    Accept,
    Decline,
    /// Script dialog button (index).
    Button(usize),
    /// llTextBox: send the typed text.
    SendText,
    OpenUrl,
}

#[derive(Default)]
pub struct Notifications {
    pub list: Vec<Notification>,
    next: u64,
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

    pub fn unread(&self) -> usize {
        self.list.iter().filter(|n| !n.read).count()
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
        let mut out = Vec::new();
        let mut url = None;
        let accept = matches!(r, Response::Accept);
        match (&n.data, &r) {
            (_, Response::Dismiss) => {
                // closing an offer declines it, like Firestorm's toasts
                match &n.data {
                    Data::Lure { from, lure } => out.push(decline_lure(*from, *lure)),
                    Data::Friend { tx } => out.push(NetCommand::DeclineFriendship { transaction: *tx }),
                    Data::Inventory { from, tx, task, .. } => out.push(inventory_answer(*from, *tx, *task, false, Uuid::nil())),
                    Data::Group { group, tx } => out.push(group_answer(*group, *tx, false)),
                    Data::Dialog { .. } | Data::TextBox { .. } => {} // ignore = no reply
                    _ => {}
                }
            }
            (Data::Lure { from, lure }, _) => {
                if accept {
                    out.push(NetCommand::AcceptLure { lure_id: *lure });
                } else {
                    out.push(decline_lure(*from, *lure));
                }
            }
            (Data::Friend { tx }, _) => {
                out.push(if accept {
                    NetCommand::AcceptFriendship {
                        transaction: *tx,
                        folder: calling_cards,
                    }
                } else {
                    NetCommand::DeclineFriendship { transaction: *tx }
                });
            }
            (
                Data::Inventory {
                    from,
                    tx,
                    asset_type,
                    task,
                },
                _,
            ) => {
                let folder = if accept { folder_for(*asset_type) } else { Uuid::nil() };
                out.push(inventory_answer(*from, *tx, *task, accept, folder));
            }
            (Data::Group { group, tx }, _) => out.push(group_answer(*group, *tx, accept)),
            (Data::Dialog { object, channel, buttons }, Response::Button(i)) => {
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
            (Data::Permissions { task, item, questions }, _) => {
                // ScriptAnswerYes with the granted bits (0 = refuse)
                out.push(NetCommand::ScriptAnswer {
                    task_id: *task,
                    item_id: *item,
                    questions: if accept { *questions } else { 0 },
                });
            }
            (Data::Url(u), Response::OpenUrl) => url = Some(u.clone()),
            _ => {}
        }
        (out, url)
    }
}

fn decline_lure(from: Uuid, lure: Uuid) -> NetCommand {
    NetCommand::SendImDialog {
        to: from,
        dialog: im::LURE_DECLINED,
        id: lure,
        message: String::new(),
        bucket: Vec::new(),
    }
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

fn group_answer(group: Uuid, tx: Uuid, accept: bool) -> NetCommand {
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
                channel: -42,
                buttons: vec!["Oui".into(), "Non".into()],
            },
        );
        assert_eq!(n.unread(), 1);
        let (cmds, _) = n.respond(id, Response::Button(1), |_| Uuid::nil(), Uuid::nil());
        assert!(matches!(&cmds[..], [NetCommand::ScriptDialogReply { channel: -42, index: 1, label, .. }] if label == "Non"));
        assert!(n.list.is_empty());
    }

    #[test]
    fn inventory_accept_puts_folder_in_bucket() {
        let mut n = Notifications::default();
        let (from, tx, folder) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let id = n.push(
            Kind::Inventory,
            "Objet",
            "",
            Data::Inventory {
                from,
                tx,
                asset_type: 6,
                task: false,
            },
        );
        let (cmds, _) = n.respond(id, Response::Accept, |t| if t == 6 { folder } else { Uuid::nil() }, Uuid::nil());
        assert!(
            matches!(&cmds[..], [NetCommand::SendImDialog { dialog: im::INVENTORY_ACCEPTED, bucket, .. }] if bucket[..] == folder.as_bytes()[..])
        );
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
            },
        );
        let (cmds, _) = n.respond(id, Response::Decline, |_| Uuid::nil(), Uuid::nil());
        assert!(matches!(&cmds[..], [NetCommand::ScriptAnswer { questions: 0, .. }]));
        assert_eq!(
            permission_lines(0x10 | 0x20),
            vec!["Animer votre avatar", "S'attacher à votre avatar"]
        );
    }
}
