//! Session side of the offline instant messages: the request made once
//! after login (LLIMProcessing::requestOfflineMessages: the ReadOfflineMsgs
//! capability when it and the capabilities that answer offline offers are
//! there, else the RetrieveInstantMessages message) and the answers to a
//! friendship offer or a group invitation that came that way
//! (accept_friendship_coro, decline_friendship_coro,
//! response_group_invitation_coro in llviewermessage.cpp).

use super::Session;
use crate::offline_im;
use crate::types::{NetCommand, NetEvent};
use aurora_llsd::llsd_map;
use aurora_msg::msgs::*;

impl Session<'_> {
    /// Commands of this module; false when `c` is not one of them.
    pub(super) fn on_offline_command(&mut self, c: &NetCommand) -> bool {
        match c {
            NetCommand::RequestOfflineMessages => self.request_offline_messages(),
            NetCommand::RequestOfflineMessagesLegacy => self.request_offline_messages_legacy(),
            NetCommand::AnswerOfflineFriendship {
                from,
                transaction,
                folder,
                accept,
                agent_name,
            } => self.answer_offline_friendship(*from, *transaction, *folder, *accept, agent_name),
            NetCommand::AnswerOfflineGroupInvite { group, accept } => self.answer_offline_group_invite(*group, *accept),
            _ => return false,
        }
        true
    }

    /// LLIMProcessing::requestOfflineMessages.
    fn request_offline_messages(&mut self) {
        let caps: Vec<Option<String>> = offline_im::REQUIRED_CAPS.iter().map(|c| self.main_cap(c)).collect();
        let Some(url) = caps.iter().all(Option::is_some).then(|| caps[0].clone()).flatten() else {
            // the offline messages capability gives no session id for
            // friendship offers and group invitations: without the
            // capabilities that answer those, the legacy message
            self.request_offline_messages_legacy();
            return;
        };
        log::info!("requesting offline messages (capability)");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let contents = match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(bytes) if bytes.is_empty() => Ok(aurora_llsd::Llsd::Undef),
                    Ok(bytes) => aurora_llsd::from_xml(&bytes).map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                },
                Ok(resp) => Err(format!("HTTP {}", resp.status())),
                Err(e) => Err(e.to_string()),
            };
            match contents {
                // "Error requesting offline messages via capability...
                // Falling back to legacy method."
                Err(e) => {
                    log::warn!("offline messages: capability failed ({e}); falling back to the legacy request");
                    let _ = events.send(NetEvent::OfflineMessagesFailed);
                }
                Ok(contents) => match offline_im::parse(&contents) {
                    Ok(messages) => {
                        log::info!("offline messages: {}", messages.len());
                        for m in messages {
                            let _ = events.send(NetEvent::InstantMessage(m));
                        }
                    }
                    // Firestorm stops there too, without the legacy request
                    Err(e) => log::warn!("offline messages: {e}"),
                },
            }
        });
    }

    /// LLIMProcessing::requestOfflineMessagesLegacy: the simulator then
    /// sends each message as an ImprovedInstantMessage.
    fn request_offline_messages_legacy(&mut self) {
        log::info!("requesting offline messages (legacy)");
        let mut m = RetrieveInstantMessages::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        self.send_main(&m, true);
    }

    /// friendship_offer_callback for an offer received while offline: by
    /// capability when the region has it, else by message when the offer
    /// came with a transaction id.
    fn answer_offline_friendship(&mut self, from: uuid::Uuid, transaction: uuid::Uuid, folder: uuid::Uuid, accept: bool, agent_name: &str) {
        let cap = self.main_cap(if accept { "AcceptFriendship" } else { "DeclineFriendship" });
        let Some(cap) = cap else {
            if transaction.is_nil() {
                log::warn!("friendship offer: neither capability nor transaction id, cannot answer");
            } else if accept {
                let mut m = AcceptFriendship::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.transaction_block.transaction_id = transaction;
                m.folder_data = vec![accept_friendship::FolderData { folder_id: folder }];
                self.send_main(&m, true);
            } else {
                let mut m = DeclineFriendship::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.transaction_block.transaction_id = transaction;
                self.send_main(&m, true);
            }
            return;
        };
        let http = self.sh.caps_http.clone();
        let url = if accept {
            offline_im::accept_friendship_url(&cap, &from, agent_name)
        } else {
            offline_im::decline_friendship_url(&cap, &from)
        };
        tokio::spawn(async move {
            let request = if accept {
                // an empty LLSD body
                http.post(&url)
                    .header("Content-Type", "application/llsd+xml")
                    .body(aurora_llsd::to_xml(&aurora_llsd::Llsd::Undef))
            } else {
                http.delete(&url)
            };
            let what = if accept { "accept" } else { "decline" };
            match request.header("Accept", "application/llsd+xml").send().await {
                Ok(resp) if resp.status().is_success() => {
                    let ok = resp
                        .bytes()
                        .await
                        .ok()
                        .and_then(|b| aurora_llsd::from_xml(&b).ok())
                        .is_some_and(|v| v["success"].as_bool());
                    if !ok {
                        log::warn!("friendship offer {what}: the server did not process it");
                    }
                }
                Ok(resp) => log::warn!("friendship offer {what}: HTTP {}", resp.status()),
                Err(e) => log::warn!("friendship offer {what}: {e}"),
            }
        });
    }

    /// send_join_group_response with use_offline_cap.
    fn answer_offline_group_invite(&mut self, group: uuid::Uuid, accept: bool) {
        let Some(url) = self.main_cap(if accept { "AcceptGroupInvite" } else { "DeclineGroupInvite" }) else {
            // "if sim has no this cap, we can do nothing - regular request will fail"
            log::warn!("group invitation: no capability, cannot answer an offline invitation");
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            match crate::caps::post_llsd(&http, &url, &llsd_map! { "group" => group }).await {
                Ok(v) if v["success"].as_bool() => {
                    // accepted: the group list is asked again
                    if accept {
                        let _ = events.send(NetEvent::OfflineGroupInviteAccepted(group));
                    }
                }
                Ok(_) => log::warn!("group invitation: the server did not process the answer"),
                Err(e) => log::warn!("group invitation answer: {e}"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::caps::REQUESTED_CAPS;

    #[test]
    fn the_capabilities_of_offline_messages_are_requested() {
        for cap in [
            "ReadOfflineMsgs",
            "AcceptFriendship",
            "DeclineFriendship",
            "AcceptGroupInvite",
            "DeclineGroupInvite",
        ] {
            assert!(REQUESTED_CAPS.contains(&cap), "{cap}");
        }
    }
}
