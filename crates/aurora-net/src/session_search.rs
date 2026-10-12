//! Session side of the directory search (search.rs): the queries of
//! FSPanelSearch*::find sent reliably to the agent's region, their replies
//! (DirPeopleReply, DirGroupsReply, DirPlacesReply, DirEventsReply,
//! DirClassifiedReply by UDP, DirLandReply by the event queue), and the
//! detail requests the profiles did not need: EventInfoRequest
//! (FSFloaterSearch::onSelectedEvent), GroupProfileRequest
//! (LLGroupMgr::sendGroupPropertiesRequest) and the land types of
//! ProductInfoRequest (LLProductInfoRequestManager).

use super::{Session, emit};
use crate::search::{self, DirMessage};
use crate::types::{NetCommand, NetEvent};
use aurora_llsd::Llsd;
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg};
use std::net::SocketAddr;

impl Session<'_> {
    /// Commands of this module; false when `c` is not one of them.
    pub(super) fn on_search_command(&mut self, c: &NetCommand) -> bool {
        match c {
            NetCommand::DirQuery { id, start, query } => {
                match search::message(self.agent_id(), self.session_id(), *id, *start, query) {
                    DirMessage::Find(m) => self.send_main(&m, true),
                    DirMessage::Places(m) => self.send_main(&m, true),
                    DirMessage::Land(m) => self.send_main(&m, true),
                    DirMessage::Classifieds(m) => self.send_main(&m, true),
                }
                log::debug!("search: query {id} from row {start}");
            }
            NetCommand::EventInfoRequest(event) => {
                let mut m = EventInfoRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.event_data.event_id = *event;
                self.send_main(&m, true);
            }
            NetCommand::GroupProfileRequest(group) => {
                let mut m = GroupProfileRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.group_data.group_id = *group;
                self.send_main(&m, true);
            }
            NetCommand::ProductInfoRequest => self.product_info_request(),
            _ => return false,
        }
        true
    }

    /// LLProductInfoRequestManager::getLandDescriptionsCoro: GET of the
    /// region's ProductInfoRequest capability.
    fn product_info_request(&mut self) {
        let Some(url) = self.main_cap("ProductInfoRequest") else {
            log::debug!("ProductInfoRequest not available: land types stay unknown");
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(r) if r.status().is_success() => match r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()) {
                    Some(v) => {
                        let _ = events.send(NetEvent::ProductInfo(search::product_info(&v)));
                    }
                    None => log::warn!("ProductInfoRequest: unreadable reply"),
                },
                Ok(r) => log::warn!("ProductInfoRequest: HTTP {}", r.status().as_u16()),
                Err(e) => log::warn!("ProductInfoRequest: {e}"),
            }
        });
    }

    /// Replies of this module; false when the packet is not one of them. As
    /// in every processSearchReply, a reply addressed to another agent is
    /// dropped.
    pub(super) fn dispatch_search(&mut self, _from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        let id = pkt.id;
        let me = self.agent_id();
        let reply = if id == DirPeopleReply::ID {
            let m: DirPeopleReply = pkt.decode()?;
            (m.agent_data.agent_id == me).then(|| search::people_reply(&m))
        } else if id == DirGroupsReply::ID {
            let m: DirGroupsReply = pkt.decode()?;
            (m.agent_data.agent_id == me).then(|| search::groups_reply(&m))
        } else if id == DirPlacesReply::ID {
            let m: DirPlacesReply = pkt.decode()?;
            (m.agent_data.agent_id == me).then(|| search::places_reply(&m)).flatten()
        } else if id == DirLandReply::ID {
            let m: DirLandReply = pkt.decode()?;
            (m.agent_data.agent_id == me).then(|| search::land_reply(&m))
        } else if id == DirEventsReply::ID {
            let m: DirEventsReply = pkt.decode()?;
            (m.agent_data.agent_id == me).then(|| search::events_reply(&m))
        } else if id == DirClassifiedReply::ID {
            let m: DirClassifiedReply = pkt.decode()?;
            (m.agent_data.agent_id == me).then(|| search::classifieds_reply(&m))
        } else if id == EventInfoReply::ID {
            let m: EventInfoReply = pkt.decode()?;
            emit(self.sh, NetEvent::EventInfo(Box::new(search::event_info(&m))));
            return Ok(true);
        } else if id == GroupProfileReply::ID {
            let m: GroupProfileReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(self.sh, NetEvent::GroupProfile(Box::new(search::group_profile(&m))));
            }
            return Ok(true);
        } else {
            return Ok(false);
        };
        if let Some(reply) = reply {
            emit(self.sh, NetEvent::DirReply(Box::new(reply)));
        }
        Ok(true)
    }

    /// Event-queue messages of this module; false when not one of them.
    pub(super) fn on_search_eq(&mut self, message: &str, b: &Llsd) -> bool {
        if message != "DirLandReply" {
            return false;
        }
        let (to, reply) = search::land_reply_llsd(b);
        if to == self.agent_id() {
            emit(self.sh, NetEvent::DirReply(Box::new(reply)));
        }
        true
    }
}
