//! Which scripts an editor may follow: the ones the user sent to the
//! external editor, each followed by one connection at a time.
//!
//! Port of LLScriptEditorWSServer::subscribeScriptEditor / unsubscribeEditor
//! / unsubscribeConnection / updateScriptSubscription / handleScriptSubscribe
//! / handleScriptUnsubscribe / handleFileWatcherFileListRequest and
//! buildScriptSubscriptionId (indra/newview/llscripteditorws.cpp, originally
//! LGPL 2.1). A client never adds a script: it can only follow one the
//! viewer offered, by the id it read from the file's name.

use crate::ConnId;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// The id of a script for the protocol: the MD5 of
/// `<object id>_<item id>` (LLUUID::asString, lowercase), as hex. The
/// object id is nil for a script of the agent's inventory. The extension
/// reads it back from the exported file's name.
pub fn script_id(object: Uuid, item: Uuid) -> String {
    format!("{:x}", md5::compute(format!("{}_{}", object.hyphenated(), item.hyphenated())))
}

/// The exported file's name the extension recognises
/// (LLScriptEdContainer::getTmpFileName; the extension's
/// SCRIPT_FILE_PATTERN): `sl_script_<name>_<script id>.<lsl|luau>`, or
/// without the name when it is empty. `name` must already be safe for a
/// file name.
pub fn file_name(name: &str, script_id: &str, lua: bool) -> String {
    let extension = if lua { "luau" } else { "lsl" };
    if name.is_empty() {
        format!("sl_script_{script_id}.{extension}")
    } else {
        format!("sl_script_{name}_{script_id}.{extension}")
    }
}

/// SubscriptionError, sent as `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionError {
    InvalidEditor = 1,
    InvalidSubscription = 2,
    AlreadySubscribed = 3,
    Internal = 4,
}

impl SubscriptionError {
    fn message(self) -> &'static str {
        match self {
            SubscriptionError::InvalidEditor => "Invalid editor handle",
            SubscriptionError::InvalidSubscription => "No subscription found for script",
            SubscriptionError::AlreadySubscribed => "Script already subscribed",
            SubscriptionError::Internal => "Internal server error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry<K> {
    /// The prim that holds it; nil in the agent's inventory.
    pub object: Uuid,
    pub item: Uuid,
    pub name: String,
    /// The script's language is Luau (SLua): its runtime errors read
    /// differently.
    pub lua: bool,
    /// The viewer's editor window.
    pub editor: K,
    /// The connection that follows it.
    pub conn: Option<ConnId>,
}

#[derive(Debug)]
pub struct Subscriptions<K> {
    entries: HashMap<String, Entry<K>>,
}

impl<K> Default for Subscriptions<K> {
    fn default() -> Self {
        Subscriptions { entries: HashMap::new() }
    }
}

impl<K: Clone + PartialEq> Subscriptions<K> {
    /// The user sent a script to the external editor
    /// (subscribeScriptEditor): it can now be followed. Offered again, it
    /// keeps its connection and takes the new editor.
    pub fn offer(&mut self, object: Uuid, item: Uuid, name: &str, lua: bool, editor: K) -> String {
        let id = script_id(object, item);
        match self.entries.get_mut(&id) {
            Some(entry) => {
                entry.editor = editor;
                entry.name = name.to_owned();
                entry.lua = lua;
            }
            None => {
                self.entries.insert(
                    id.clone(),
                    Entry {
                        object,
                        item,
                        name: name.to_owned(),
                        lua,
                        editor,
                        conn: None,
                    },
                );
            }
        }
        id
    }

    /// The script's editor closed (unsubscribeEditor): the connection that
    /// followed it, to tell it with `script.unsubscribe`.
    pub fn remove(&mut self, script_id: &str) -> Option<ConnId> {
        self.entries.remove(script_id).and_then(|e| e.conn)
    }

    /// Every script of an editor window that closed.
    pub fn remove_editor(&mut self, editor: &K) -> Vec<(String, Option<ConnId>)> {
        let ids: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| &e.editor == editor)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter().map(|id| (id.clone(), self.remove(&id))).collect()
    }

    pub fn get(&self, script_id: &str) -> Option<&Entry<K>> {
        self.entries.get(script_id)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Entry<K>)> {
        self.entries.iter()
    }

    /// The entry an object's chat about `script_name` is about.
    pub fn find_by_name(&self, object: Uuid, script_name: &str) -> Option<(&String, &Entry<K>)> {
        self.entries
            .iter()
            .find(|(_, e)| !object.is_nil() && e.object == object && e.name == script_name)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `script.subscribe` (updateScriptSubscription). `editor_alive` says
    /// whether an entry's window is still open.
    pub fn subscribe(&mut self, conn: ConnId, script_id: &str, editor_alive: impl Fn(&K) -> bool) -> Result<&Entry<K>, SubscriptionError> {
        let Some(entry) = self.entries.get(script_id) else {
            return Err(SubscriptionError::InvalidSubscription);
        };
        if !editor_alive(&entry.editor) {
            self.entries.remove(script_id);
            return Err(SubscriptionError::InvalidEditor);
        }
        let entry = self.entries.get_mut(script_id).ok_or(SubscriptionError::Internal)?;
        // one editor at a time, the same connection included, as the Legacy
        // Viewer answers (a closed connection has already let go of it)
        if entry.conn.is_some() {
            return Err(SubscriptionError::AlreadySubscribed);
        }
        entry.conn = Some(conn);
        Ok(entry)
    }

    /// `script.unsubscribe`: only by the connection that follows it.
    pub fn unsubscribe(&mut self, conn: ConnId, script_id: &str) {
        if self.entries.get(script_id).is_some_and(|e| e.conn == Some(conn)) {
            self.entries.remove(script_id);
        }
    }

    /// A connection closed (unsubscribeConnection): its scripts can be
    /// followed again.
    pub fn drop_connection(&mut self, conn: ConnId) {
        for entry in self.entries.values_mut() {
            if entry.conn == Some(conn) {
                entry.conn = None;
            }
        }
    }

    /// Lets go of the scripts of connections that are gone, whether or not
    /// their `Event::Closed` arrived (the Legacy Viewer's
    /// updateScriptSubscription asks the connection isConnected()).
    pub fn release_closed(&mut self, open: impl Fn(ConnId) -> bool) {
        for entry in self.entries.values_mut() {
            if entry.conn.is_some_and(|c| !open(c)) {
                entry.conn = None;
            }
        }
    }

    /// The connection a script's messages go to.
    pub fn connection(&self, script_id: &str) -> Option<ConnId> {
        self.entries.get(script_id).and_then(|e| e.conn)
    }

    /// `script.list`'s `script_ids`.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.entries.keys().cloned().collect();
        ids.sort();
        ids
    }
}

/// The answer to `script.subscribe` (handleScriptSubscribe). `root` is the
/// root prim of the linkset that holds the script (the prim itself if not
/// known).
pub fn subscribe_reply<K>(script_id: &str, result: Result<&Entry<K>, SubscriptionError>, root: impl Fn(Uuid) -> Uuid) -> Value {
    let status = match &result {
        Ok(_) => 0,
        Err(e) => *e as i64,
    };
    let message = match &result {
        Ok(_) => "OK",
        Err(e) => e.message(),
    };
    let mut reply = json!({
        "script_id": script_id,
        "success": result.is_ok(),
        "status": status,
        "message": message,
    });
    if let Ok(entry) = result {
        reply["object_id"] = json!(entry.object.hyphenated().to_string());
        reply["root_id"] = json!(root(entry.object).hyphenated().to_string());
        reply["item_id"] = json!(entry.item.hyphenated().to_string());
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn script_ids_are_the_legacy_md5() {
        // LLMD5 of "00000000-0000-0000-0000-000000000001_00000000-0000-0000-0000-000000000002"
        let expected = format!(
            "{:x}",
            md5::compute("00000000-0000-0000-0000-000000000001_00000000-0000-0000-0000-000000000002")
        );
        assert_eq!(script_id(id(1), id(2)), expected);
        assert_eq!(script_id(id(1), id(2)).len(), 32);
        assert_ne!(script_id(id(1), id(2)), script_id(id(2), id(1)));
        assert_eq!(file_name("door", &expected, false), format!("sl_script_door_{expected}.lsl"));
        assert_eq!(file_name("", &expected, true), format!("sl_script_{expected}.luau"));
    }

    #[test]
    fn only_offered_scripts_can_be_followed_by_one_connection_at_a_time() {
        let mut subs = Subscriptions::default();
        assert_eq!(
            subs.subscribe(1, "unknown", |_| true).err(),
            Some(SubscriptionError::InvalidSubscription)
        );
        let sid = subs.offer(id(1), id(2), "door", false, 10u32);
        assert_eq!(subs.subscribe(1, &sid, |_| true).map(|e| e.item).ok(), Some(id(2)));
        // followed already: refused, to the same connection too
        assert_eq!(subs.subscribe(1, &sid, |_| true).err(), Some(SubscriptionError::AlreadySubscribed));
        assert_eq!(subs.subscribe(2, &sid, |_| true).err(), Some(SubscriptionError::AlreadySubscribed));
        assert_eq!(subs.connection(&sid), Some(1));
        // another connection cannot unsubscribe it
        subs.unsubscribe(2, &sid);
        assert_eq!(subs.connection(&sid), Some(1));
        // once the first one is gone, the second may follow it
        subs.drop_connection(1);
        assert_eq!(subs.connection(&sid), None);
        assert!(subs.subscribe(2, &sid, |_| true).is_ok());
        subs.unsubscribe(2, &sid);
        assert!(subs.get(&sid).is_none());
    }

    #[test]
    fn a_connection_gone_without_its_close_lets_go_of_its_scripts() {
        let mut subs = Subscriptions::default();
        let a = subs.offer(id(1), id(2), "door", false, 10u32);
        let b = subs.offer(id(1), id(3), "lamp", false, 10u32);
        assert!(subs.subscribe(1, &a, |_| true).is_ok());
        assert!(subs.subscribe(2, &b, |_| true).is_ok());
        // connection 1 is gone, its Event::Closed lost; 2 is still there
        subs.release_closed(|c| c == 2);
        assert_eq!(subs.connection(&a), None);
        assert_eq!(subs.connection(&b), Some(2));
        assert!(subs.subscribe(3, &a, |_| true).is_ok());
    }

    #[test]
    fn a_closed_editor_ends_its_subscriptions() {
        let mut subs = Subscriptions::default();
        let a = subs.offer(id(1), id(2), "a", false, 10u32);
        let b = subs.offer(id(1), id(3), "b", true, 10u32);
        let c = subs.offer(Uuid::nil(), id(4), "c", false, 11u32);
        assert!(subs.subscribe(5, &a, |_| true).is_ok());
        assert_eq!(subs.subscribe(5, &c, |e| *e != 11).err(), Some(SubscriptionError::InvalidEditor));
        assert!(subs.get(&c).is_none());
        let mut gone = subs.remove_editor(&10);
        gone.sort();
        let mut expected = vec![(a.clone(), Some(5)), (b.clone(), None)];
        expected.sort();
        assert_eq!(gone, expected);
        assert!(subs.is_empty());
    }

    #[test]
    fn runtime_chat_finds_its_script_by_object_and_name() {
        let mut subs = Subscriptions::default();
        let sid = subs.offer(id(1), id(2), "door", false, 10u32);
        subs.offer(Uuid::nil(), id(3), "door", false, 10u32);
        assert_eq!(subs.find_by_name(id(1), "door").map(|(s, _)| s.clone()), Some(sid));
        assert!(subs.find_by_name(id(9), "door").is_none());
        assert!(subs.find_by_name(id(1), "Door").is_none());
        // an inventory script has no object to speak for it
        assert!(subs.find_by_name(Uuid::nil(), "door").is_none());
    }

    #[test]
    fn subscribe_replies_match_the_legacy_viewer() {
        let mut subs = Subscriptions::default();
        let sid = subs.offer(id(1), id(2), "door", false, 10u32);
        let reply = subscribe_reply(&sid, subs.subscribe(1, &sid, |_| true), |_| id(7));
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["status"], json!(0));
        assert_eq!(reply["message"], json!("OK"));
        assert_eq!(reply["object_id"], json!(id(1).to_string()));
        assert_eq!(reply["root_id"], json!(id(7).to_string()));
        assert_eq!(reply["item_id"], json!(id(2).to_string()));
        let refused = subscribe_reply::<u32>("nope", Err(SubscriptionError::InvalidSubscription), |o| o);
        assert_eq!(
            refused,
            json!({"script_id": "nope", "success": false, "status": 2, "message": "No subscription found for script"})
        );
    }
}
