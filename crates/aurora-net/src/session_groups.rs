//! Session side of a group's data (groups.rs): the requests of LLGroupMgr
//! (sendCapGroupMembersRequest and its GroupMembersRequest fallback,
//! sendGroupRoleDataRequest, sendGroupRoleMembersRequest,
//! sendGroupTitlesRequest, sendGroupTitleUpdate, sendGroupMemberJoin,
//! sendGroupMemberInvites), LLPanelGroupNotices' GroupNoticesListRequest,
//! LLPanelGroupLandMoney's land and accounts (group_money.rs), sent
//! reliably to the agent's region, and their replies.

use super::{Session, emit};
use crate::group_money;
use crate::groups;
use crate::types::{NetCommand, NetEvent};
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg};

impl Session<'_> {
    /// Commands of this module; false when `c` is not one of them.
    pub(super) fn on_groups_command(&mut self, c: &NetCommand) -> bool {
        match c {
            NetCommand::GroupMembersRequest { group, request } => self.group_members_request(*group, *request),
            NetCommand::GroupRoleDataRequest { group, request } => {
                let mut m = GroupRoleDataRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.group_data.group_id = *group;
                m.group_data.request_id = *request;
                self.send_main(&m, true);
            }
            NetCommand::GroupRoleMembersRequest { group, request } => {
                let mut m = GroupRoleMembersRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.group_data.group_id = *group;
                m.group_data.request_id = *request;
                self.send_main(&m, true);
            }
            NetCommand::GroupTitlesRequest { group, request } => {
                let mut m = GroupTitlesRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.group_id = *group;
                m.agent_data.request_id = *request;
                self.send_main(&m, true);
            }
            NetCommand::GroupTitleUpdate { group, role } => {
                let mut m = GroupTitleUpdate::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.agent_data.group_id = *group;
                m.agent_data.title_role_id = *role;
                self.send_main(&m, true);
            }
            NetCommand::GroupNoticesListRequest(group) => {
                let mut m = GroupNoticesListRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.group_id = *group;
                self.send_main(&m, true);
            }
            NetCommand::EjectGroupMembers { group, members } => {
                for m in groups::eject_messages(self.agent_id(), self.session_id(), *group, members) {
                    self.send_main(&m, true);
                }
            }
            NetCommand::GroupRoleChanges { group, changes } => {
                for m in groups::role_change_messages(self.agent_id(), self.session_id(), *group, changes) {
                    self.send_main(&m, true);
                }
            }
            NetCommand::GroupBans {
                group,
                action,
                ids,
                refresh,
            } => self.group_bans(*group, *action, ids.clone(), *refresh),
            NetCommand::GroupRoleUpdate { group, updates } => {
                for m in groups::role_update_messages(self.agent_id(), self.session_id(), *group, updates) {
                    self.send_main(&m, true);
                }
            }
            NetCommand::UpdateGroupInfo { group, info } => {
                let m = groups::group_info_message(self.agent_id(), self.session_id(), *group, info);
                self.send_main(&m, true);
            }
            NetCommand::CreateGroup { name, info } => {
                log::info!("creating group \"{name}\"");
                let m = groups::create_group_message(self.agent_id(), self.session_id(), name, info);
                self.send_main(&m, true);
            }
            NetCommand::GroupNoticeRequest(notice) => {
                // LLPanelGroupNotices::onSelectNotice
                let mut m = GroupNoticeRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.data.group_notice_id = *notice;
                self.send_main(&m, true);
            }
            NetCommand::JoinGroup(group) => {
                let mut m = JoinGroupRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.group_data.group_id = *group;
                self.send_main(&m, true);
            }
            NetCommand::InviteToGroup { group, invites } => {
                for m in groups::invite_messages(self.agent_id(), self.session_id(), *group, invites) {
                    self.send_main(&m, true);
                }
            }
            NetCommand::GroupLandQuery { group, transaction } => {
                let m = group_money::places_query(self.agent_id(), self.session_id(), *group, *transaction);
                self.send_main(&m, true);
            }
            NetCommand::SetGroupContribution { group, contribution } => {
                let m = group_money::contribution_message(self.agent_id(), self.session_id(), *group, *contribution);
                self.send_main(&m, true);
            }
            NetCommand::GroupAccount(r) => match group_money::account_message(self.agent_id(), self.session_id(), r) {
                group_money::AccountMessage::Summary(m) => self.send_main(&m, true),
                group_money::AccountMessage::Details(m) => self.send_main(&m, true),
                group_money::AccountMessage::Sales(m) => self.send_main(&m, true),
            },
            _ => return false,
        }
        true
    }

    /// LLGroupMgr::sendCapGroupMembersRequest: the whole list from the
    /// GroupMemberData capability, or GroupMembersRequest when the region
    /// has none ("Falling back to UDP fetch"). A failed request answers
    /// nothing, as in groupMembersRequestCoro: the window asks again when
    /// it is opened again.
    fn group_members_request(&mut self, group: uuid::Uuid, request: uuid::Uuid) {
        let Some(url) = self.main_cap("GroupMemberData") else {
            log::info!("group members: no GroupMemberData capability, by UDP");
            let mut m = GroupMembersRequest::default();
            m.agent_data.agent_id = self.agent_id();
            m.agent_data.session_id = self.session_id();
            m.group_data.group_id = group;
            m.group_data.request_id = request;
            self.send_main(&m, true);
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            match crate::caps::post_llsd(&http, &url, &groups::member_data_body(group)).await {
                Ok(v) => match groups::member_data_response(&v, request) {
                    Some(page) => {
                        log::info!("group members: {} from GroupMemberData", page.members.len());
                        let _ = events.send(NetEvent::GroupMembers(Box::new(page)));
                    }
                    None => log::warn!("group members: no member data received"),
                },
                Err(e) => log::warn!("group members: error receiving member data ({e})"),
            }
        });
    }

    /// LLGroupMgr::sendGroupBanRequest: the GroupAPIv1 capability, with the
    /// group in the query; a POST answers the list when it changed it.
    fn group_bans(&mut self, group: uuid::Uuid, action: Option<i32>, ids: Vec<uuid::Uuid>, refresh: bool) {
        let Some(cap) = self.main_cap("GroupAPIv1") else {
            log::info!("group bans: no GroupAPIv1 capability");
            return;
        };
        let url = format!("{cap}?group_id={group}");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let mut reply = match action {
                Some(action) => crate::caps::post_llsd(&http, &url, &groups::ban_body(action, &ids)).await,
                None => crate::caps::get_llsd(&http, &url).await,
            };
            // postGroupBanRequestCoro: with BAN_UPDATE, the list again
            if refresh && reply.is_ok() {
                reply = crate::caps::get_llsd(&http, &url).await;
            }
            match reply {
                Ok(v) => {
                    if let Some((g, bans)) = groups::ban_list(&v) {
                        let group = if g.is_nil() { group } else { g };
                        let _ = events.send(NetEvent::GroupBanList { group, bans });
                    }
                }
                Err(e) => log::warn!("group bans: {e}"),
            }
        });
    }

    /// Replies of this module; false when the packet is not one of them. As
    /// in LLGroupMgr, a reply addressed to another agent is dropped.
    pub(super) fn dispatch_groups(&mut self, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        let id = pkt.id;
        let me = self.agent_id();
        if id == GroupMembersReply::ID {
            let m: GroupMembersReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(self.sh, NetEvent::GroupMembers(Box::new(groups::members_reply(&m))));
            }
        } else if id == GroupRoleDataReply::ID {
            let m: GroupRoleDataReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(self.sh, NetEvent::GroupRoles(Box::new(groups::roles_reply(&m))));
            }
        } else if id == GroupRoleMembersReply::ID {
            let m: GroupRoleMembersReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(self.sh, NetEvent::GroupRoleMembers(Box::new(groups::role_members_reply(&m))));
            }
        } else if id == GroupTitlesReply::ID {
            let m: GroupTitlesReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(
                    self.sh,
                    NetEvent::GroupTitles {
                        group: m.agent_data.group_id,
                        request: m.agent_data.request_id,
                        titles: groups::titles_reply(&m),
                    },
                );
            }
        } else if id == GroupNoticesListReply::ID {
            let m: GroupNoticesListReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(
                    self.sh,
                    NetEvent::GroupNotices {
                        group: m.agent_data.group_id,
                        notices: groups::notices_reply(&m),
                    },
                );
            }
        } else if id == JoinGroupReply::ID {
            let m: JoinGroupReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(
                    self.sh,
                    NetEvent::GroupJoined {
                        group: m.group_data.group_id,
                        success: m.group_data.success,
                    },
                );
            }
        } else if id == CreateGroupReply::ID {
            let m: CreateGroupReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                let d = &m.reply_data;
                let message = aurora_msg::field_str(&d.message);
                log::info!("group creation: success {}, {message:?}", d.success);
                emit(
                    self.sh,
                    NetEvent::GroupCreated {
                        group: d.group_id,
                        success: d.success,
                        message,
                    },
                );
            }
        } else if id == EjectGroupMemberReply::ID {
            // processEjectGroupMemberReply: a failure refreshes the group
            let m: EjectGroupMemberReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(
                    self.sh,
                    NetEvent::GroupEjected {
                        group: m.group_data.group_id,
                        success: m.eject_data.success,
                    },
                );
            }
        } else if id == PlacesReply::ID {
            let m: PlacesReply = pkt.decode()?;
            let (to, reply) = group_money::places_reply(&m);
            self.on_group_land(to, reply);
        } else if id == GroupAccountSummaryReply::ID {
            let (to, reply) = group_money::summary_reply(&pkt.decode()?);
            self.on_group_account(to, reply);
        } else if id == GroupAccountDetailsReply::ID {
            let (to, reply) = group_money::details_reply(&pkt.decode()?);
            self.on_group_account(to, reply);
        } else if id == GroupAccountTransactionsReply::ID {
            let (to, reply) = group_money::transactions_reply(&pkt.decode()?);
            self.on_group_account(to, reply);
        } else if id == LeaveGroupReply::ID {
            let m: LeaveGroupReply = pkt.decode()?;
            if m.agent_data.agent_id == me {
                emit(
                    self.sh,
                    NetEvent::GroupLeft {
                        group: m.group_data.group_id,
                        success: m.group_data.success,
                    },
                );
            }
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// Event-queue messages of this module (PlacesReply, UDPDeprecated);
    /// false when not one of them.
    pub(super) fn on_groups_eq(&mut self, message: &str, b: &aurora_llsd::Llsd) -> bool {
        if message != "PlacesReply" {
            return false;
        }
        let (to, reply) = group_money::places_reply_llsd(b);
        self.on_group_land(to, reply);
        true
    }

    /// process_places_reply: a nil query id is the land holdings' (not
    /// ported), any other the group panel's of that group.
    fn on_group_land(&mut self, to: uuid::Uuid, reply: group_money::GroupLandReply) {
        if to != self.agent_id() {
            log::warn!("PlacesReply for another agent");
        } else if reply.group.is_nil() {
            log::debug!("PlacesReply for the land holdings: not shown");
        } else {
            emit(self.sh, NetEvent::GroupLand(Box::new(reply)));
        }
    }

    /// processGroupAccount…Reply: "Got group L$ history reply for another
    /// agent!" is dropped.
    fn on_group_account(&mut self, to: uuid::Uuid, reply: group_money::AccountReply) {
        if to == self.agent_id() {
            emit(self.sh, NetEvent::GroupAccount(Box::new(reply)));
        } else {
            log::warn!("group L$ reply for another agent");
        }
    }
}
