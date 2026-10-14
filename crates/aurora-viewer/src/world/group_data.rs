//! What the viewer knows of the groups whose profile is open, as LLGroupMgr
//! keeps it (indra/newview/llgroupmgr.cpp, originally LGPL 2.1): the member
//! list, the roles, which member has which role, the titles we can wear and
//! the notice list. Each request carries a new request id and only the
//! replies to the latest one are kept (the "stale group or request id"
//! checks); the role / member pairs are asked once the members and the
//! roles are complete, as sendGroupRoleMembersRequest waits for them.

use aurora_net::groups::{GroupMember, GroupRole, GroupTitle, MembersPage, NoticeSummary, RoleMembersPage, RolesPage};
use aurora_net::{NetCommand, NetEvent};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

/// Groups kept at most (MAX_CACHED_GROUPS): the least recently opened one
/// goes first.
const MAX_CACHED_GROUPS: usize = 20;

/// The notices read, per account: a group's notices are unread once they
/// arrived after its list was first seen and were never opened.
pub const NOTICES_READ_FILE: &str = "group_notices_read.xml";

/// IM_GROUP_NOTICE_INVENTORY_ACCEPTED / _DECLINED (llinstantmessage.h).
const NOTICE_INVENTORY_ACCEPTED: u8 = 33;
const NOTICE_INVENTORY_DECLINED: u8 = 34;

/// A notice opened (IM_GROUP_NOTICE_REQUESTED, LLPanelGroupNotices::showNotice).
#[derive(Debug, Clone, PartialEq)]
pub struct NoticeView {
    /// The notice asked for (the instant message does not name it).
    pub id: Uuid,
    pub subject: String,
    pub message: String,
    /// The sender's legacy name.
    pub from_name: String,
    pub timestamp: u32,
    /// The attachment's name and asset type.
    pub attachment: Option<(String, u8)>,
    /// Who to answer the attachment's offer, and its transaction.
    offer: Option<(Uuid, Uuid)>,
    /// The attachment was saved to the inventory.
    pub saved: bool,
}

/// One group (LLGroupMgrGroupData).
#[derive(Debug, Default)]
pub struct GroupData {
    pub members: HashMap<Uuid, GroupMember>,
    /// MemberCount of the replies.
    pub member_total: i32,
    pub members_complete: bool,
    member_request: Option<Uuid>,
    /// By id; the "everyone" role has the nil id.
    pub roles: HashMap<Uuid, GroupRole>,
    role_total: i32,
    pub roles_complete: bool,
    role_request: Option<Uuid>,
    /// The roles of each member, "everyone" included once complete.
    pub member_roles: HashMap<Uuid, Vec<Uuid>>,
    pub role_members_complete: bool,
    role_members_request: Option<Uuid>,
    received_pairs: u32,
    /// Asked before the members and roles were complete (mPendingRoleMemberRequest).
    pending_role_members: bool,
    pub titles: Vec<GroupTitle>,
    titles_request: Option<Uuid>,
    /// The notice list, newest first; None until it answered.
    pub notices: Option<Vec<NoticeSummary>>,
    /// The notice shown, and the one asked for.
    pub notice: Option<NoticeView>,
    asked_notice: Option<Uuid>,
    accessed: u64,
}

impl GroupData {
    /// The members online now (OnlineStatus "Online").
    pub fn online(&self) -> impl Iterator<Item = &GroupMember> {
        self.members
            .values()
            .filter(|m| m.last_seen == aurora_net::groups::LastSeen::Online)
    }

    /// The title we wear (the selected one of GroupTitlesReply).
    pub fn selected_title(&self) -> Option<&GroupTitle> {
        self.titles.iter().find(|t| t.selected)
    }

    /// The roles of `member`: the owners', the officers', the others by
    /// name, "everyone" last.
    pub fn roles_of(&self, member: &Uuid) -> Vec<&GroupRole> {
        use aurora_net::groups::BuiltinRole;
        let mut v: Vec<&GroupRole> = self
            .member_roles
            .get(member)
            .into_iter()
            .flatten()
            .filter_map(|r| self.roles.get(r))
            .collect();
        v.sort_by_cached_key(|r| {
            let rank = match BuiltinRole::of(&r.name) {
                _ if r.id.is_nil() => 3,
                Some(BuiltinRole::Owners) => 0,
                Some(BuiltinRole::Officers) => 1,
                _ => 2,
            };
            (rank, r.name.to_lowercase())
        });
        v
    }
}

#[derive(Default)]
pub struct GroupMgr {
    pub groups: HashMap<Uuid, GroupData>,
    clock: u64,
    /// Notices read, by group (NOTICES_READ_FILE).
    read: HashMap<Uuid, HashSet<Uuid>>,
    read_path: Option<PathBuf>,
    pub out: Vec<NetCommand>,
}

impl GroupMgr {
    pub fn get(&self, group: &Uuid) -> Option<&GroupData> {
        self.groups.get(group)
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }

    /// createGroupData, the least recently opened group dropped past
    /// MAX_CACHED_GROUPS (LLGroupMgr::addGroup).
    fn data(&mut self, group: Uuid) -> &mut GroupData {
        if !self.groups.contains_key(&group)
            && self.groups.len() >= MAX_CACHED_GROUPS
            && let Some(oldest) = self.groups.iter().min_by_key(|(_, g)| g.accessed).map(|(id, _)| *id)
        {
            self.groups.remove(&oldest);
        }
        self.clock += 1;
        let clock = self.clock;
        let g = self.groups.entry(group).or_default();
        g.accessed = clock;
        g
    }

    /// The profile window opened on `group` (LLPanelGroupGeneral::activate,
    /// then the members and roles tabs' activate): the titles, then what
    /// is not complete yet; the notice list for a member
    /// (LLPanelGroupNotices::activate).
    pub fn open(&mut self, group: Uuid, member: bool) {
        if group.is_nil() {
            return;
        }
        self.request_titles(group);
        let g = self.data(group);
        let (members, roles, pairs) = (g.members_complete, g.roles_complete, g.role_members_complete);
        if !members {
            self.request_members(group);
        }
        if !roles {
            self.request_roles(group);
        }
        if !pairs {
            self.request_role_members(group);
        }
        if member {
            self.request_notices(group);
        }
    }

    /// LLGroupActions::refresh: everything forgotten and asked again.
    pub fn refresh(&mut self, group: Uuid, member: bool) {
        self.groups.remove(&group);
        self.open(group, member);
    }

    /// Forget a group (LLGroupMgr::clearGroupData), after joining or leaving.
    pub fn clear(&mut self, group: &Uuid) {
        self.groups.remove(group);
    }

    /// LLGroupMgr::sendCapGroupMembersRequest: a new request id marks the
    /// list pending.
    fn request_members(&mut self, group: Uuid) {
        let request = Uuid::new_v4();
        let g = self.data(group);
        g.members.clear();
        g.members_complete = false;
        g.member_request = Some(request);
        self.out.push(NetCommand::GroupMembersRequest { group, request });
    }

    /// sendGroupRoleDataRequest: not again while one is pending.
    fn request_roles(&mut self, group: Uuid) {
        let g = self.data(group);
        if g.role_request.is_some() {
            return;
        }
        let request = Uuid::new_v4();
        g.roles.clear();
        g.roles_complete = false;
        g.role_request = Some(request);
        self.out.push(NetCommand::GroupRoleDataRequest { group, request });
    }

    /// sendGroupRoleMembersRequest: kept pending until the members and the
    /// roles are complete.
    fn request_role_members(&mut self, group: Uuid) {
        let g = self.data(group);
        if g.role_members_request.is_some() {
            return;
        }
        if !g.members_complete || !g.roles_complete {
            g.pending_role_members = true;
            return;
        }
        let request = Uuid::new_v4();
        g.member_roles.clear();
        g.received_pairs = 0;
        g.role_members_complete = false;
        g.role_members_request = Some(request);
        self.out.push(NetCommand::GroupRoleMembersRequest { group, request });
    }

    /// sendGroupTitlesRequest: the list starts again.
    fn request_titles(&mut self, group: Uuid) {
        let request = Uuid::new_v4();
        let g = self.data(group);
        g.titles.clear();
        g.titles_request = Some(request);
        self.out.push(NetCommand::GroupTitlesRequest { group, request });
    }

    /// onClickRefreshNotices: the list starts again.
    pub fn request_notices(&mut self, group: Uuid) {
        self.data(group).notices = None;
        self.out.push(NetCommand::GroupNoticesListRequest(group));
    }

    /// LLPanelGroupNotices::onSelectNotice: GroupNoticeRequest, and the
    /// notice counts as read.
    pub fn open_notice(&mut self, group: Uuid, notice: Uuid) {
        self.data(group).asked_notice = Some(notice);
        self.out.push(NetCommand::GroupNoticeRequest(notice));
        if self.read.entry(group).or_default().insert(notice) {
            self.save_read();
        }
    }

    /// The notices read in a group, set as a whole (the demo's).
    pub fn set_read(&mut self, group: Uuid, ids: Vec<Uuid>) {
        self.read.insert(group, ids.into_iter().collect());
    }

    /// The notices of a group not read yet.
    pub fn unread(&self, group: &Uuid) -> usize {
        let read = self.read.get(group);
        self.groups
            .get(group)
            .and_then(|g| g.notices.as_ref())
            .map_or(0, |list| list.iter().filter(|n| !read.is_some_and(|r| r.contains(&n.id))).count())
    }

    pub fn is_read(&self, group: &Uuid, notice: &Uuid) -> bool {
        self.read.get(group).is_some_and(|r| r.contains(notice))
    }

    /// The account's notices read: an LLSD map of group ids to arrays of
    /// notice ids.
    pub fn load_read(&mut self, path: PathBuf) {
        // what this session already set (the demo's) stays over the file
        let set = std::mem::take(&mut self.read);
        self.read = std::fs::read(&path)
            .ok()
            .and_then(|b| aurora_llsd::from_xml(&b).ok())
            .and_then(|v| {
                v.as_map().map(|m| {
                    m.iter()
                        .filter_map(|(g, ids)| {
                            let g = Uuid::parse_str(g).ok()?;
                            Some((g, ids.as_array().iter().map(|i| i.as_uuid()).filter(|i| !i.is_nil()).collect()))
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
        self.read.extend(set);
        self.read_path = Some(path);
    }

    fn save_read(&self) {
        let Some(path) = &self.read_path else {
            return;
        };
        let mut map = aurora_llsd::Map::new();
        for (g, ids) in &self.read {
            let mut ids: Vec<Uuid> = ids.iter().copied().collect();
            ids.sort();
            map.insert(
                g.to_string(),
                aurora_llsd::Llsd::from(ids.into_iter().map(aurora_llsd::Llsd::from).collect::<Vec<_>>()),
            );
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&aurora_llsd::Llsd::Map(map))) {
            log::warn!("group notices read: {e}");
        }
    }

    /// IM_GROUP_NOTICE_REQUESTED: the notice asked for, shown; the previous
    /// one's attachment, still offered, is declined (showNotice).
    pub fn on_notice_im(&mut self, im: &aurora_net::InstantMessage) -> bool {
        let Some(bucket) = aurora_net::groups::notice_bucket(&im.binary_bucket) else {
            log::warn!("group notice: malformed binary bucket");
            return true;
        };
        let Some(g) = self.groups.get_mut(&bucket.group) else {
            return true;
        };
        let (subject, message) = aurora_net::groups::notice_text(&im.message);
        log::info!("group notice shown: {subject} (attachment: {})", bucket.has_inventory);
        let id = g.asked_notice.unwrap_or_default();
        if let Some(prev) = g.notice.take()
            && let (Some((from, tx)), false) = (prev.offer, prev.saved)
        {
            self.out.push(offer_answer(from, tx, None));
        }
        let timestamp = g
            .notices
            .as_ref()
            .and_then(|l| l.iter().find(|n| n.id == id))
            .map_or(im.timestamp, |n| n.timestamp);
        g.notice = Some(NoticeView {
            id,
            subject,
            message,
            from_name: im.from_name.clone(),
            timestamp,
            attachment: bucket.has_inventory.then(|| (bucket.item_name.clone(), bucket.asset_type)),
            offer: bucket.has_inventory.then_some((im.from_agent_id, im.session_id)),
            saved: false,
        });
        true
    }

    /// "Open attachment" (LLOfferInfo::forceResponse(IOR_ACCEPT)): into
    /// `folder`, the inventory folder of its type.
    pub fn save_attachment(&mut self, group: Uuid, folder: Uuid) {
        let Some(n) = self.groups.get_mut(&group).and_then(|g| g.notice.as_mut()) else {
            return;
        };
        if let (Some((from, tx)), false) = (n.offer, n.saved) {
            n.saved = true;
            self.out.push(offer_answer(from, tx, Some(folder)));
        }
    }

    /// onClickSendMessage: the notice sent, shown at once in the list (the
    /// server's copy comes with the next refresh).
    pub fn add_sent_notice(&mut self, group: Uuid, subject: String, from: String, has_attachment: bool) {
        let id = Uuid::new_v4();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32);
        let list = self.data(group).notices.get_or_insert_with(Vec::new);
        list.insert(
            0,
            NoticeSummary {
                id,
                timestamp,
                from,
                subject,
                has_attachment,
                asset_type: 0,
            },
        );
        self.read.entry(group).or_default().insert(id);
    }

    /// LLGroupMgr::sendGroupTitleUpdate: sent, and our copy follows at once.
    pub fn set_title(&mut self, group: Uuid, role: Uuid) {
        self.out.push(NetCommand::GroupTitleUpdate { group, role });
        if let Some(g) = self.groups.get_mut(&group) {
            for t in &mut g.titles {
                t.selected = t.role == role;
            }
        }
    }

    /// Whether the event was one of ours.
    pub fn on_event(&mut self, ev: &NetEvent) -> bool {
        match ev {
            NetEvent::GroupMembers(page) => self.on_members(page),
            NetEvent::GroupRoles(page) => self.on_roles(page),
            NetEvent::GroupRoleMembers(page) => self.on_role_members(page),
            NetEvent::GroupTitles { group, request, titles } => match self.groups.get_mut(group) {
                Some(g) if g.titles_request == Some(*request) => g.titles.extend(titles.iter().cloned()),
                _ => log::warn!("group titles: stale group or request id"),
            },
            NetEvent::GroupNotices { group, notices } => {
                if let Some(g) = self.groups.get_mut(group) {
                    // the list can arrive in several packets, and more than
                    // once: each notice once, newest first
                    let list = g.notices.get_or_insert_with(Vec::new);
                    for n in notices {
                        if !list.iter().any(|k| k.id == n.id) {
                            list.push(n.clone());
                        }
                    }
                    list.sort_by_key(|n| std::cmp::Reverse(n.timestamp));
                    // the first list of a group: what it holds is not new
                    if !self.read.contains_key(group) {
                        self.read.insert(*group, list.iter().map(|n| n.id).collect());
                        self.save_read();
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// processGroupMembersReply / processCapGroupMembersResponse.
    fn on_members(&mut self, page: &MembersPage) {
        let Some(g) = self.groups.get_mut(&page.group).filter(|g| g.member_request == Some(page.request)) else {
            log::warn!("group members: stale group or request id");
            return;
        };
        if page.whole {
            g.members.clear();
        }
        g.member_total = page.total;
        for m in &page.members {
            g.members.insert(m.id, m.clone());
        }
        if g.members.len() == page.total.max(0) as usize {
            g.members_complete = true;
            g.member_request = None;
            // "if group members are loaded while titles are missing, load
            // the titles"
            if g.titles.is_empty() {
                self.request_titles(page.group);
            }
            self.after_complete(page.group);
        }
    }

    /// processGroupRoleDataReply.
    fn on_roles(&mut self, page: &RolesPage) {
        let Some(g) = self.groups.get_mut(&page.group).filter(|g| g.role_request == Some(page.request)) else {
            log::warn!("group roles: stale group or request id");
            return;
        };
        g.role_total = page.total;
        for r in &page.roles {
            g.roles.insert(r.id, r.clone());
        }
        if g.roles.len() == g.role_total.max(0) as usize {
            g.roles_complete = true;
            g.role_request = None;
            self.after_complete(page.group);
        }
    }

    /// The role / member pairs asked while the members or roles were
    /// missing go out once both are complete.
    fn after_complete(&mut self, group: Uuid) {
        if let Some(g) = self.groups.get_mut(&group)
            && g.pending_role_members
            && g.members_complete
            && g.roles_complete
        {
            g.pending_role_members = false;
            self.request_role_members(group);
        }
    }

    /// processGroupRoleMembersReply: pairs of an unknown role or member are
    /// dropped; once all arrived, everyone also has the "everyone" role.
    fn on_role_members(&mut self, page: &RoleMembersPage) {
        let Some(g) = self
            .groups
            .get_mut(&page.group)
            .filter(|g| g.role_members_request == Some(page.request))
        else {
            log::warn!("group role members: stale group or request id");
            return;
        };
        if page.total_pairs > 0 {
            for (role, member) in &page.pairs {
                if role.is_nil() || member.is_nil() {
                    continue;
                }
                if g.roles.contains_key(role) && g.members.contains_key(member) {
                    g.member_roles.entry(*member).or_default().push(*role);
                }
            }
            g.received_pairs += page.pairs.len() as u32;
        }
        if g.received_pairs == page.total_pairs {
            if g.roles.contains_key(&Uuid::nil()) {
                for m in g.members.keys() {
                    g.member_roles.entry(*m).or_default().push(Uuid::nil());
                }
            } else {
                log::warn!("group role members: everyone role not found");
            }
            g.role_members_complete = true;
            g.role_members_request = None;
        }
    }
}

/// The answer to a notice attachment's offer (sendReceiveResponse with
/// IM_GROUP_NOTICE: accepted with the destination folder, or declined).
fn offer_answer(from: Uuid, tx: Uuid, folder: Option<Uuid>) -> NetCommand {
    NetCommand::SendImDialog {
        to: from,
        dialog: if folder.is_some() {
            NOTICE_INVENTORY_ACCEPTED
        } else {
            NOTICE_INVENTORY_DECLINED
        },
        id: tx,
        message: String::new(),
        bucket: folder.map_or_else(|| vec![0], |f| f.as_bytes().to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::groups::LastSeen;

    const G: Uuid = Uuid::from_u128(1);

    fn member(id: u128, online: bool) -> GroupMember {
        GroupMember {
            id: Uuid::from_u128(id),
            contribution: 0,
            powers: 0,
            title: "Member".into(),
            last_seen: if online { LastSeen::Online } else { LastSeen::Date(2026, 9, 12) },
            is_owner: false,
        }
    }

    fn role(id: u128, name: &str) -> GroupRole {
        GroupRole {
            id: Uuid::from_u128(id),
            name: name.into(),
            title: String::new(),
            description: String::new(),
            powers: 0,
            members: 0,
        }
    }

    fn request_of(cmds: &[NetCommand], pick: fn(&NetCommand) -> Option<Uuid>) -> Uuid {
        cmds.iter().find_map(pick).expect("request sent")
    }

    #[test]
    fn opening_asks_titles_members_roles_and_notices_then_pairs_once_both_are_complete() {
        let mut mgr = GroupMgr::default();
        mgr.open(G, true);
        let cmds = mgr.take_commands();
        assert!(matches!(cmds[0], NetCommand::GroupTitlesRequest { group, .. } if group == G));
        assert!(cmds.iter().any(|c| matches!(c, NetCommand::GroupNoticesListRequest(g) if *g == G)));
        assert!(
            !cmds.iter().any(|c| matches!(c, NetCommand::GroupRoleMembersRequest { .. })),
            "pairs wait"
        );
        let members = request_of(&cmds, |c| match c {
            NetCommand::GroupMembersRequest { request, .. } => Some(*request),
            _ => None,
        });
        let roles = request_of(&cmds, |c| match c {
            NetCommand::GroupRoleDataRequest { request, .. } => Some(*request),
            _ => None,
        });

        // a stale reply is dropped
        let stale = MembersPage {
            group: G,
            request: Uuid::from_u128(99),
            total: 1,
            members: vec![member(10, true)],
            whole: false,
        };
        assert!(mgr.on_event(&NetEvent::GroupMembers(Box::new(stale))));
        assert!(mgr.get(&G).unwrap().members.is_empty());

        // members in two packets
        for m in [member(10, true), member(11, false)] {
            mgr.on_event(&NetEvent::GroupMembers(Box::new(MembersPage {
                group: G,
                request: members,
                total: 2,
                members: vec![m],
                whole: false,
            })));
        }
        let g = mgr.get(&G).unwrap();
        assert!(g.members_complete);
        assert_eq!(g.online().count(), 1);
        // the titles again ("members loaded while titles are missing"), the
        // pairs still waiting for the roles
        let cmds = mgr.take_commands();
        assert!(cmds.iter().all(|c| matches!(c, NetCommand::GroupTitlesRequest { .. })), "{cmds:?}");

        mgr.on_event(&NetEvent::GroupRoles(Box::new(RolesPage {
            group: G,
            request: roles,
            total: 2,
            roles: vec![role(0, "Everyone"), role(5, "Officers")],
        })));
        let cmds = mgr.take_commands();
        let pairs = request_of(&cmds, |c| match c {
            NetCommand::GroupRoleMembersRequest { request, .. } => Some(*request),
            _ => None,
        });
        mgr.on_event(&NetEvent::GroupRoleMembers(Box::new(RoleMembersPage {
            group: G,
            request: pairs,
            total_pairs: 2,
            pairs: vec![(Uuid::from_u128(5), Uuid::from_u128(10)), (Uuid::from_u128(6), Uuid::from_u128(10))],
        })));
        let g = mgr.get(&G).unwrap();
        assert!(g.role_members_complete);
        let names: Vec<&str> = g.roles_of(&Uuid::from_u128(10)).iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Officers", "Everyone"], "unknown role dropped, everyone last");
        assert_eq!(g.roles_of(&Uuid::from_u128(11)).len(), 1);

        // a second opening asks only the titles (and the notices)
        mgr.open(G, false);
        let cmds = mgr.take_commands();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], NetCommand::GroupTitlesRequest { .. }));
    }

    #[test]
    fn the_capability_list_replaces_and_completes() {
        let mut mgr = GroupMgr::default();
        mgr.open(G, false);
        let cmds = mgr.take_commands();
        assert!(
            !cmds.iter().any(|c| matches!(c, NetCommand::GroupNoticesListRequest(_))),
            "no notices for a guest"
        );
        let request = request_of(&cmds, |c| match c {
            NetCommand::GroupMembersRequest { request, .. } => Some(*request),
            _ => None,
        });
        mgr.on_event(&NetEvent::GroupMembers(Box::new(MembersPage {
            group: G,
            request,
            total: 3,
            members: vec![member(10, true), member(11, true), member(12, false)],
            whole: true,
        })));
        let g = mgr.get(&G).unwrap();
        assert!(g.members_complete);
        assert_eq!(g.online().count(), 2);
    }

    #[test]
    fn titles_follow_our_choice_and_notices_come_once_newest_first() {
        let mut mgr = GroupMgr::default();
        mgr.open(G, true);
        let cmds = mgr.take_commands();
        let request = request_of(&cmds, |c| match c {
            NetCommand::GroupTitlesRequest { request, .. } => Some(*request),
            _ => None,
        });
        let title = |role: u128, selected| GroupTitle {
            title: format!("t{role}"),
            role: Uuid::from_u128(role),
            selected,
        };
        mgr.on_event(&NetEvent::GroupTitles {
            group: G,
            request,
            titles: vec![title(0, true), title(5, false)],
        });
        mgr.set_title(G, Uuid::from_u128(5));
        assert!(matches!(mgr.take_commands()[..], [NetCommand::GroupTitleUpdate { role, .. }] if role == Uuid::from_u128(5)));
        assert_eq!(mgr.get(&G).unwrap().selected_title().unwrap().role, Uuid::from_u128(5));

        let notice = |id: u128, t: u32| NoticeSummary {
            id: Uuid::from_u128(id),
            timestamp: t,
            from: String::new(),
            subject: String::new(),
            has_attachment: false,
            asset_type: 0,
        };
        mgr.on_event(&NetEvent::GroupNotices {
            group: G,
            notices: vec![notice(1, 10), notice(2, 30)],
        });
        mgr.on_event(&NetEvent::GroupNotices {
            group: G,
            notices: vec![notice(2, 30), notice(3, 20)],
        });
        let ids: Vec<u128> = mgr
            .get(&G)
            .unwrap()
            .notices
            .as_ref()
            .unwrap()
            .iter()
            .map(|n| n.id.as_u128())
            .collect();
        assert_eq!(ids, [2, 3, 1]);
    }

    #[test]
    fn at_most_twenty_groups_are_kept() {
        let mut mgr = GroupMgr::default();
        for i in 1..=21u128 {
            mgr.open(Uuid::from_u128(i), false);
        }
        assert_eq!(mgr.groups.len(), MAX_CACHED_GROUPS);
        assert!(mgr.get(&Uuid::from_u128(1)).is_none(), "the oldest one went");
        assert!(mgr.get(&Uuid::from_u128(21)).is_some());
    }
}
