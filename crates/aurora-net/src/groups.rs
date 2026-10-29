//! The data of one group as LLGroupMgr keeps it (indra/newview/llgroupmgr.cpp,
//! roles_constants.h, llpanelgroupnotices.cpp, originally LGPL 2.1): its
//! members (GroupMembersReply, or the GroupMemberData capability), roles
//! (GroupRoleDataReply), role / member pairs (GroupRoleMembersReply), the
//! titles we can wear (GroupTitlesReply) and its notice list
//! (GroupNoticesListReply). The requests carry a random request id that the
//! replies repeat: a reply to an older request is dropped by the viewer.

use aurora_llsd::{Llsd, llsd_map};
use aurora_msg::msgs;
use aurora_msg::{field_str, str_field};
use uuid::Uuid;

/// Group abilities (roles_constants.h GP_*), the bits of a role's or a
/// member's powers.
pub mod powers {
    pub const MEMBER_INVITE: u64 = 1 << 1;
    pub const MEMBER_EJECT: u64 = 1 << 2;
    pub const MEMBER_OPTIONS: u64 = 1 << 3;
    pub const ROLE_CREATE: u64 = 1 << 4;
    pub const ROLE_DELETE: u64 = 1 << 5;
    pub const ROLE_PROPERTIES: u64 = 1 << 6;
    pub const ROLE_ASSIGN_MEMBER_LIMITED: u64 = 1 << 7;
    pub const ROLE_ASSIGN_MEMBER: u64 = 1 << 8;
    pub const ROLE_REMOVE_MEMBER: u64 = 1 << 9;
    pub const ROLE_CHANGE_ACTIONS: u64 = 1 << 10;
    pub const GROUP_CHANGE_IDENTITY: u64 = 1 << 11;
    pub const SESSION_JOIN: u64 = 1 << 16;
    pub const SESSION_VOICE: u64 = 1 << 27;
    pub const ACCOUNTING_ACCOUNTABLE: u64 = 1 << 40;
    pub const NOTICES_SEND: u64 = 1 << 42;
    pub const NOTICES_RECEIVE: u64 = 1 << 43;
    pub const MEMBER_VISIBLE_IN_DIR: u64 = 1 << 47;
    pub const GROUP_BAN_ACCESS: u64 = 1 << 51;
}

/// Where a member was last seen (the OnlineStatus string: "Online", or
/// the date of their last login as MM/DD/YYYY).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastSeen {
    Online,
    /// (year, month, day).
    Date(u16, u8, u8),
    /// Hidden, or a text we do not read: shown as it came.
    Other(String),
}

impl LastSeen {
    /// The OnlineStatus text (LLGroupMgr::processGroupMembersReply then
    /// formatDateString reads "Online" or a MM/DD/YYYY date).
    pub fn parse(text: &str) -> LastSeen {
        let text = text.trim();
        if text == "Online" {
            return LastSeen::Online;
        }
        let parts: Vec<&str> = text.split('/').collect();
        if let [m, d, y] = parts[..]
            && let (Ok(m), Ok(d), Ok(y)) = (m.parse::<u8>(), d.parse::<u8>(), y.parse::<u16>())
            && (1..=12).contains(&m)
            && (1..=31).contains(&d)
        {
            return LastSeen::Date(y, m, d);
        }
        LastSeen::Other(text.to_owned())
    }

    /// Most recent first: online, then by date, the unreadable texts last
    /// (the order of the "status" column, sorted on the reformatted dates).
    pub fn rank(&self) -> (u8, u16, u8, u8) {
        match self {
            LastSeen::Online => (2, 0, 0, 0),
            LastSeen::Date(y, m, d) => (1, *y, *m, *d),
            LastSeen::Other(_) => (0, 0, 0, 0),
        }
    }
}

/// One member (LLGroupMemberData).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMember {
    pub id: Uuid,
    /// Land donated to the group, in m².
    pub contribution: i32,
    pub powers: u64,
    pub title: String,
    pub last_seen: LastSeen,
    pub is_owner: bool,
}

/// One role (LLGroupRoleData). The "everyone" role has the nil id.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRole {
    pub id: Uuid,
    pub name: String,
    pub title: String,
    pub description: String,
    pub powers: u64,
    pub members: u32,
}

/// A title we can wear in the group, the one of one of our roles
/// (LLGroupTitle).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupTitle {
    pub title: String,
    pub role: Uuid,
    pub selected: bool,
}

/// One line of the notice list (LLPanelGroupNotices::processNotices).
#[derive(Debug, Clone, PartialEq)]
pub struct NoticeSummary {
    pub id: Uuid,
    /// Unix time.
    pub timestamp: u32,
    /// The legacy name of the sender ("First Last").
    pub from: String,
    pub subject: String,
    pub has_attachment: bool,
    pub asset_type: u8,
}

/// A page of members: one GroupMembersReply packet, or the whole list of
/// the GroupMemberData capability.
#[derive(Debug, Clone, PartialEq)]
pub struct MembersPage {
    pub group: Uuid,
    pub request: Uuid,
    /// The total the group has (MemberCount); the list is complete once
    /// that many arrived.
    pub total: i32,
    pub members: Vec<GroupMember>,
    /// The capability answered with the whole list at once.
    pub whole: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RolesPage {
    pub group: Uuid,
    pub request: Uuid,
    pub total: i32,
    pub roles: Vec<GroupRole>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleMembersPage {
    pub group: Uuid,
    pub request: Uuid,
    pub total_pairs: u32,
    /// (role, member).
    pub pairs: Vec<(Uuid, Uuid)>,
}

/// The roles the server names in English, as LLGroupMgr renames them
/// (lldatagroups.cpp creates them under these names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinRole {
    Everyone,
    Officers,
    Owners,
}

impl BuiltinRole {
    pub fn of(name: &str) -> Option<BuiltinRole> {
        match name {
            "Everyone" => Some(BuiltinRole::Everyone),
            "Officers" => Some(BuiltinRole::Officers),
            "Owners" => Some(BuiltinRole::Owners),
            _ => None,
        }
    }
}

/// LLGroupMgr::processGroupMembersReply; the null ids are skipped.
pub fn members_reply(m: &msgs::GroupMembersReply) -> MembersPage {
    MembersPage {
        group: m.group_data.group_id,
        request: m.group_data.request_id,
        total: m.group_data.member_count,
        members: if m.group_data.member_count > 0 {
            m.member_data
                .iter()
                .filter(|d| !d.agent_id.is_nil())
                .map(|d| GroupMember {
                    id: d.agent_id,
                    contribution: d.contribution,
                    powers: d.agent_powers,
                    title: field_str(&d.title),
                    last_seen: LastSeen::parse(&field_str(&d.online_status)),
                    is_owner: d.is_owner,
                })
                .collect()
        } else {
            Vec::new()
        },
        whole: false,
    }
}

/// LLGroupMgr::processGroupRoleDataReply.
pub fn roles_reply(m: &msgs::GroupRoleDataReply) -> RolesPage {
    RolesPage {
        group: m.group_data.group_id,
        request: m.group_data.request_id,
        total: m.group_data.role_count,
        roles: m
            .role_data
            .iter()
            .map(|r| GroupRole {
                id: r.role_id,
                name: field_str(&r.name),
                title: field_str(&r.title),
                description: field_str(&r.description),
                powers: r.powers,
                members: r.members,
            })
            .collect(),
    }
}

/// LLGroupMgr::processGroupRoleMembersReply: the pairs with a null id are
/// counted but not kept.
pub fn role_members_reply(m: &msgs::GroupRoleMembersReply) -> RoleMembersPage {
    RoleMembersPage {
        group: m.agent_data.group_id,
        request: m.agent_data.request_id,
        total_pairs: m.agent_data.total_pairs,
        pairs: m.member_data.iter().map(|d| (d.role_id, d.member_id)).collect(),
    }
}

/// LLGroupMgr::processGroupTitlesReply: titles left empty are not offered.
pub fn titles_reply(m: &msgs::GroupTitlesReply) -> Vec<GroupTitle> {
    m.group_data
        .iter()
        .map(|d| GroupTitle {
            title: field_str(&d.title),
            role: d.role_id,
            selected: d.selected,
        })
        .filter(|t| !t.title.is_empty())
        .collect()
}

/// LLPanelGroupNotices::processNotices: a single null notice means "no
/// notices".
pub fn notices_reply(m: &msgs::GroupNoticesListReply) -> Vec<NoticeSummary> {
    if m.data.len() == 1 && m.data[0].notice_id.is_nil() {
        return Vec::new();
    }
    m.data
        .iter()
        .filter(|d| !d.notice_id.is_nil())
        .map(|d| NoticeSummary {
            id: d.notice_id,
            timestamp: d.timestamp,
            from: field_str(&d.from_name),
            subject: field_str(&d.subject),
            has_attachment: d.has_attachment,
            asset_type: d.asset_type,
        })
        .collect()
}

/// The body of a GroupMemberData request (LLGroupMgr::groupMembersRequestCoro
/// with no page size and no sorting, as sendCapGroupMembersRequest is called
/// by the group panels).
pub fn member_data_body(group: Uuid) -> Llsd {
    llsd_map! { "group_id" => group }
}

/// A hexadecimal power mask of the capability ("powers", "default_powers").
fn hex_powers(v: &Llsd) -> u64 {
    u64::from_str_radix(v.as_str().trim(), 16).unwrap_or(0)
}

/// LLGroupMgr::processCapGroupMembersResponse: every member, the title an
/// index into "titles" (the first by default), the powers those of
/// "defaults" unless given, a member without "last_login" of unknown status.
pub fn member_data_response(v: &Llsd, request: Uuid) -> Option<MembersPage> {
    let group = v["group_id"].as_uuid();
    let members = v["members"].as_map()?;
    let titles = v["titles"].as_array();
    let default_title = titles.first().map(|t| t.to_string_value()).unwrap_or_default();
    let default_powers = hex_powers(&v["defaults"]["default_powers"]);
    let list: Vec<GroupMember> = members
        .iter()
        .filter_map(|(id, info)| {
            let id = Uuid::parse_str(id).ok().filter(|id| !id.is_nil())?;
            Some(GroupMember {
                id,
                contribution: info["donated_square_meters"].as_i32(),
                powers: if info.has("powers") {
                    hex_powers(&info["powers"])
                } else {
                    default_powers
                },
                title: if info.has("title") {
                    usize::try_from(info["title"].as_i32())
                        .ok()
                        .and_then(|i| titles.get(i))
                        .map(|t| t.to_string_value())
                        .unwrap_or_default()
                } else {
                    default_title.clone()
                },
                last_seen: if info.has("last_login") {
                    LastSeen::parse(info["last_login"].as_str())
                } else {
                    LastSeen::Other("unknown".into())
                },
                is_owner: info.has("owner"),
            })
        })
        .collect();
    Some(MembersPage {
        group,
        request,
        total: list.len() as i32,
        members: list,
        whole: true,
    })
}

/// Roles a member can be invited into (LLPanelGroupInvite::impl::addRoleNames):
/// every role for an owner, with "assign members to any role" every role
/// but the owners', with "assign members to our roles" the roles we have;
/// "everyone" always.
pub fn invite_roles<'a>(
    roles: &'a [GroupRole],
    my_roles: &[Uuid],
    my_powers: u64,
    owner_role: Uuid,
    i_am_owner: bool,
) -> Vec<&'a GroupRole> {
    let any = my_powers & powers::ROLE_ASSIGN_MEMBER != 0;
    let limited = my_powers & powers::ROLE_ASSIGN_MEMBER_LIMITED != 0;
    roles
        .iter()
        .filter(|r| i_am_owner || (any && r.id != owner_role) || (limited && my_roles.contains(&r.id)) || r.id.is_nil())
        .collect()
}

/// The attachment of a notice received by instant message (IM_GROUP_NOTICE,
/// IM_GROUP_NOTICE_REQUESTED: notice_bucket_header_t of
/// llimprocessing.cpp): whether it has one, its asset type, the group, the
/// item's name.
#[derive(Debug, Clone, PartialEq)]
pub struct NoticeBucket {
    pub has_inventory: bool,
    pub asset_type: u8,
    pub group: Uuid,
    pub item_name: String,
}

/// The header and a null-terminated name, or None when malformed ("Malformed
/// group notice binary bucket").
pub fn notice_bucket(bucket: &[u8]) -> Option<NoticeBucket> {
    if bucket.len() < 2 + 16 + 1 || bucket.last() != Some(&0) {
        return None;
    }
    let group = Uuid::from_slice(&bucket[2..18]).ok()?;
    let name = &bucket[18..bucket.len() - 1];
    let name = name.split(|b| *b == 0).next().unwrap_or_default();
    Some(NoticeBucket {
        has_inventory: bucket[0] != 0,
        asset_type: bucket[1],
        group,
        item_name: String::from_utf8_lossy(name).into_owned(),
    })
}

/// A notice kept while we were offline (ReadOfflineMsgs) names its group in
/// the entry's `asset_id` and its attachment as text, "asset type|wearable
/// type|name", empty without one (the `aux_id` branch of process_improved_im,
/// llimprocessing.cpp, originally LGPL 2.1). It comes back here in the form
/// of a live notice's bucket, so that one parser reads both.
pub fn notice_bucket_from_offline(group: Uuid, text: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(text);
    let text = text.trim_end_matches('\0');
    // has_inventory = binary_bucket_size > 1
    let mut parts = text.split('|');
    let attachment = (!text.is_empty()).then(|| {
        let asset_type = parts.next().and_then(|t| t.trim().parse::<i32>().ok()).unwrap_or(0);
        parts.next();
        (asset_type as u8, parts.next().unwrap_or_default())
    });
    let mut out = vec![u8::from(attachment.is_some()), attachment.map_or(0, |a| a.0)];
    out.extend_from_slice(group.as_bytes());
    out.extend_from_slice(attachment.map_or("", |a| a.1).as_bytes());
    out.push(0);
    out
}

/// "subject|message" of a notice (the first "|" splits them).
pub fn notice_text(message: &str) -> (String, String) {
    match message.split_once('|') {
        Some((subject, body)) => (subject.to_owned(), body.to_owned()),
        None => (message.to_owned(), String::new()),
    }
}

/// The instant message text and binary bucket of send_group_notice: the
/// subject and the message joined by "|", the attachment as an LLSD map
/// of its item and owner, or the empty bucket.
pub fn notice_to_send(subject: &str, message: &str, item: Option<(Uuid, Uuid)>) -> (String, Vec<u8>) {
    let text = format!("{subject}|{message}");
    let bucket = match item {
        // the serialized map, without its terminating null
        Some((item, owner)) => aurora_llsd::to_xml(&llsd_map! { "item_id" => item, "owner_id" => owner }),
        // EMPTY_BINARY_BUCKET
        None => vec![0],
    };
    (text, bucket)
}

/// Ejections per EjectGroupMemberRequest and role changes per
/// GroupRoleChanges: Firestorm starts a new message when one is full
/// (isSendFull); these keep each one well under a packet.
const EJECTS_PER_MESSAGE: usize = 50;
const ROLE_CHANGES_PER_MESSAGE: usize = 30;

/// LLGroupMgr::sendGroupMemberEjects: the members, by messages. Our own id
/// is left out ("Can't use 'eject' to leave a group").
pub fn eject_messages(agent: Uuid, session: Uuid, group: Uuid, members: &[Uuid]) -> Vec<msgs::EjectGroupMemberRequest> {
    let members: Vec<Uuid> = members.iter().copied().filter(|m| *m != agent && !m.is_nil()).collect();
    members
        .chunks(EJECTS_PER_MESSAGE)
        .map(|chunk| {
            let mut m = msgs::EjectGroupMemberRequest::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.group_data.group_id = group;
            m.eject_data = chunk
                .iter()
                .map(|id| msgs::eject_group_member_request::EjectData { ejectee_id: *id })
                .collect();
            m
        })
        .collect()
}

/// A member added to or removed from a role (LLRoleMemberChange: RMC_ADD 0,
/// RMC_REMOVE 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoleChange {
    pub role: Uuid,
    pub member: Uuid,
    pub add: bool,
}

/// LLGroupMgr::sendGroupRoleMemberChanges.
pub fn role_change_messages(agent: Uuid, session: Uuid, group: Uuid, changes: &[RoleChange]) -> Vec<msgs::GroupRoleChanges> {
    changes
        .chunks(ROLE_CHANGES_PER_MESSAGE)
        .map(|chunk| {
            let mut m = msgs::GroupRoleChanges::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.agent_data.group_id = group;
            m.role_change = chunk
                .iter()
                .map(|c| msgs::group_role_changes::RoleChange {
                    role_id: c.role,
                    member_id: c.member,
                    change: if c.add { 0 } else { 1 },
                })
                .collect();
            m
        })
        .collect()
}

/// LLRoleChangeType: how a role changed (GroupRoleUpdate's UpdateType).
pub mod role_change {
    pub const DATA: u8 = 1;
    pub const POWERS: u8 = 2;
    pub const ALL: u8 = 3;
    pub const CREATE: u8 = 4;
    pub const DELETE: u8 = 5;
}

/// One role changed (LLRoleData with its mChangeType).
#[derive(Debug, Clone, PartialEq)]
pub struct RoleUpdate {
    pub role: Uuid,
    pub name: String,
    pub title: String,
    pub description: String,
    pub powers: u64,
    pub kind: u8,
}

/// Roles per GroupRoleUpdate (Firestorm starts a new message when one is
/// full; this keeps each one under a packet with long descriptions).
const ROLE_UPDATES_PER_MESSAGE: usize = 3;

/// packRoleUpdateMessageBlock for each changed role.
pub fn role_update_messages(agent: Uuid, session: Uuid, group: Uuid, updates: &[RoleUpdate]) -> Vec<msgs::GroupRoleUpdate> {
    updates
        .chunks(ROLE_UPDATES_PER_MESSAGE)
        .map(|chunk| {
            let mut m = msgs::GroupRoleUpdate::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.agent_data.group_id = group;
            m.role_data = chunk
                .iter()
                .map(|u| msgs::group_role_update::RoleData {
                    role_id: u.role,
                    name: str_field(&u.name),
                    description: str_field(&u.description),
                    title: str_field(&u.title),
                    powers: u.powers,
                    update_type: u.kind,
                })
                .collect();
            m
        })
        .collect()
}

/// What UpdateGroupInfo sends (LLGroupMgr::sendUpdateGroupInfo).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupInfo {
    pub charter: String,
    pub show_in_list: bool,
    pub insignia: Uuid,
    pub membership_fee: i32,
    pub open_enrollment: bool,
    pub allow_publish: bool,
    pub mature_publish: bool,
}

pub fn group_info_message(agent: Uuid, session: Uuid, group: Uuid, info: &GroupInfo) -> msgs::UpdateGroupInfo {
    let mut m = msgs::UpdateGroupInfo::default();
    m.agent_data.agent_id = agent;
    m.agent_data.session_id = session;
    let d = &mut m.group_data;
    d.group_id = group;
    d.charter = str_field(&info.charter);
    d.show_in_list = info.show_in_list;
    d.insignia_id = info.insignia;
    d.membership_fee = info.membership_fee;
    d.open_enrollment = info.open_enrollment;
    d.allow_publish = info.allow_publish;
    d.mature_publish = info.mature_publish;
    m
}

/// The rules of a new group's name (LLPanelGroupCreate::onBtnCreate,
/// lldbstrings.h, LLTextValidate::validateASCIINoLeadingSpace, originally
/// LGPL 2.1).
pub mod name {
    /// DB_GROUP_NAME_MIN_LEN.
    pub const MIN: usize = 4;
    /// DB_GROUP_NAME_STR_LEN, also the field's `max_length_bytes`.
    pub const MAX: usize = 35;

    /// What the name field accepts while typing: ASCII from the space up,
    /// not starting with a space, at most `MAX` bytes. A refused edit leaves
    /// the field as it was, as the prevalidation does.
    pub fn accepts(text: &str) -> bool {
        text.len() <= MAX && !text.starts_with(' ') && text.chars().all(|c| (' '..='\u{7f}').contains(&c))
    }

    /// The name sent: trimmed, within `MIN`..=`MAX` (else
    /// GroupNameLengthWarning).
    pub fn checked(text: &str) -> Option<&str> {
        let name = text.trim();
        (MIN..=MAX).contains(&name.len()).then_some(name)
    }
}

/// CreateGroupRequest (LLGroupMgr::sendCreateGroupRequest): the trimmed name
/// and what UpdateGroupInfo would send.
pub fn create_group_message(agent: Uuid, session: Uuid, name: &str, info: &GroupInfo) -> msgs::CreateGroupRequest {
    let mut m = msgs::CreateGroupRequest::default();
    m.agent_data.agent_id = agent;
    m.agent_data.session_id = session;
    let d = &mut m.group_data;
    d.name = str_field(name);
    d.charter = str_field(&info.charter);
    d.show_in_list = info.show_in_list;
    d.insignia_id = info.insignia;
    d.membership_fee = info.membership_fee;
    d.open_enrollment = info.open_enrollment;
    d.allow_publish = info.allow_publish;
    d.mature_publish = info.mature_publish;
    m
}

/// The ban actions of the GroupAPIv1 capability (LLGroupMgr::EBanAction).
pub mod ban {
    pub const CREATE: i32 = 1;
    pub const DELETE: i32 = 2;
}

/// The body of a ban POST (postGroupBanRequestCoro).
pub fn ban_body(action: i32, ids: &[Uuid]) -> Llsd {
    llsd_map! {
        "ban_action" => action,
        "ban_ids" => Llsd::Array(ids.iter().map(|id| Llsd::from(*id)).collect()),
    }
}

/// processGroupBanRequest: the group and its banned residents with the date
/// of each ban (Unix time, 0 when not given).
pub fn ban_list(v: &Llsd) -> Option<(Uuid, Vec<(Uuid, f64)>)> {
    let group = v["group_id"].as_uuid();
    let list = v["ban_list"].as_map()?;
    let bans = list
        .iter()
        .filter_map(|(id, entry)| Some((Uuid::parse_str(id).ok()?, entry["ban_date"].as_f64())))
        .collect();
    Some((group, bans))
}

/// Invitations per InviteGroupRequest (MAX_GROUP_INVITES, the server's cap).
pub const MAX_GROUP_INVITES: usize = 100;

/// LLGroupMgr::sendGroupMemberInvites: the (invitee, role) pairs, a new
/// message every MAX_GROUP_INVITES of them.
pub fn invite_messages(agent: Uuid, session: Uuid, group: Uuid, invites: &[(Uuid, Uuid)]) -> Vec<msgs::InviteGroupRequest> {
    invites
        .chunks(MAX_GROUP_INVITES)
        .map(|chunk| {
            let mut m = msgs::InviteGroupRequest::default();
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.group_data.group_id = group;
            m.invite_data = chunk
                .iter()
                .map(|(invitee, role)| msgs::invite_group_request::InviteData {
                    invitee_id: *invitee,
                    role_id: *role,
                })
                .collect();
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_msg::str_field;
    use msgs::{group_members_reply, group_notices_list_reply, group_role_data_reply, group_role_members_reply, group_titles_reply};

    #[test]
    fn a_new_group_name_follows_firestorm_rules() {
        assert!(name::accepts("Aurora Builders"));
        assert!(name::accepts(""));
        assert!(!name::accepts(" Leading"), "no leading space");
        assert!(!name::accepts("Café"), "ASCII only");
        assert!(!name::accepts("Tab\tname"));
        assert!(name::accepts(&"x".repeat(35)));
        assert!(!name::accepts(&"x".repeat(36)), "35 bytes at most");
        assert_eq!(name::checked("  Wolves  "), Some("Wolves"));
        assert_eq!(name::checked("Abc "), None, "four characters at least, once trimmed");
        assert_eq!(name::checked("Abcd"), Some("Abcd"));
    }

    #[test]
    fn create_group_request_carries_the_name_and_the_identity() {
        let info = GroupInfo {
            charter: "We build.".into(),
            show_in_list: true,
            insignia: Uuid::from_u128(7),
            membership_fee: 50,
            open_enrollment: true,
            allow_publish: false,
            mature_publish: true,
        };
        let m = create_group_message(Uuid::from_u128(1), Uuid::from_u128(2), "Wolves", &info);
        assert_eq!(m.agent_data.agent_id, Uuid::from_u128(1));
        assert_eq!(m.group_data.name, str_field("Wolves"));
        assert_eq!(m.group_data.charter, str_field("We build."));
        assert!(m.group_data.show_in_list && m.group_data.open_enrollment && m.group_data.mature_publish);
        assert!(!m.group_data.allow_publish);
        assert_eq!(m.group_data.membership_fee, 50);
        assert_eq!(m.group_data.insignia_id, Uuid::from_u128(7));
    }

    #[test]
    fn online_status_reads_online_dates_and_the_rest() {
        assert_eq!(LastSeen::parse("Online"), LastSeen::Online);
        assert_eq!(LastSeen::parse("12/25/2008"), LastSeen::Date(2008, 12, 25));
        assert_eq!(LastSeen::parse("unknown"), LastSeen::Other("unknown".into()));
        assert_eq!(LastSeen::parse("13/01/2008"), LastSeen::Other("13/01/2008".into()));
        let mut v = [
            LastSeen::parse("01/02/2020"),
            LastSeen::Online,
            LastSeen::parse("x"),
            LastSeen::parse("12/25/2021"),
        ];
        v.sort_by_key(|s| std::cmp::Reverse(s.rank()));
        assert_eq!(v[0], LastSeen::Online);
        assert_eq!(v[1], LastSeen::Date(2021, 12, 25));
        assert_eq!(v[3], LastSeen::Other("x".into()));
    }

    #[test]
    fn members_reply_keeps_the_page_and_skips_null_members() {
        let mut m = msgs::GroupMembersReply::default();
        m.group_data.group_id = Uuid::from_u128(1);
        m.group_data.request_id = Uuid::from_u128(2);
        m.group_data.member_count = 3;
        let member = |id: u128, status: &str, owner: bool| group_members_reply::MemberData {
            agent_id: Uuid::from_u128(id),
            contribution: 512,
            online_status: str_field(status),
            agent_powers: powers::MEMBER_INVITE,
            title: str_field("Builder"),
            is_owner: owner,
        };
        m.member_data = vec![
            member(10, "Online", true),
            member(0, "Online", false),
            member(11, "09/12/2026", false),
        ];
        let page = members_reply(&m);
        assert_eq!((page.group, page.request, page.total), (Uuid::from_u128(1), Uuid::from_u128(2), 3));
        assert_eq!(page.members.len(), 2);
        assert!(page.members[0].is_owner);
        assert_eq!(page.members[0].last_seen, LastSeen::Online);
        assert_eq!(page.members[1].last_seen, LastSeen::Date(2026, 9, 12));
        assert_eq!(page.members[1].title, "Builder");
        assert_eq!(page.members[1].contribution, 512);
        assert!(!page.whole);
    }

    #[test]
    fn role_data_titles_and_pairs() {
        let mut r = msgs::GroupRoleDataReply::default();
        r.group_data.role_count = 2;
        r.role_data = vec![
            group_role_data_reply::RoleData {
                role_id: Uuid::nil(),
                name: str_field("Everyone"),
                title: str_field("Member"),
                description: str_field(""),
                powers: powers::NOTICES_RECEIVE,
                members: 3,
            },
            group_role_data_reply::RoleData {
                role_id: Uuid::from_u128(5),
                name: str_field("Owners"),
                title: str_field("Founder"),
                description: str_field("The owners"),
                powers: u64::MAX,
                members: 1,
            },
        ];
        let page = roles_reply(&r);
        assert_eq!(page.total, 2);
        assert_eq!(BuiltinRole::of(&page.roles[0].name), Some(BuiltinRole::Everyone));
        assert_eq!(BuiltinRole::of(&page.roles[1].name), Some(BuiltinRole::Owners));
        assert_eq!(BuiltinRole::of("Builders"), None);

        let mut p = msgs::GroupRoleMembersReply::default();
        p.agent_data.total_pairs = 1;
        p.member_data = vec![group_role_members_reply::MemberData {
            role_id: Uuid::from_u128(5),
            member_id: Uuid::from_u128(10),
        }];
        assert_eq!(role_members_reply(&p).pairs, vec![(Uuid::from_u128(5), Uuid::from_u128(10))]);

        let t = msgs::GroupTitlesReply {
            group_data: vec![
                group_titles_reply::GroupData {
                    title: str_field("Member"),
                    role_id: Uuid::nil(),
                    selected: false,
                },
                group_titles_reply::GroupData {
                    title: str_field(""),
                    role_id: Uuid::from_u128(6),
                    selected: false,
                },
                group_titles_reply::GroupData {
                    title: str_field("Founder"),
                    role_id: Uuid::from_u128(5),
                    selected: true,
                },
            ],
            ..Default::default()
        };
        let titles = titles_reply(&t);
        assert_eq!(titles.len(), 2, "an empty title is not offered");
        assert!(titles[1].selected && titles[1].role == Uuid::from_u128(5));
    }

    #[test]
    fn notice_list_with_the_dummy_entry_is_empty() {
        let mut m = msgs::GroupNoticesListReply {
            data: vec![group_notices_list_reply::Data::default()],
            ..Default::default()
        };
        assert!(notices_reply(&m).is_empty());
        m.data = vec![group_notices_list_reply::Data {
            notice_id: Uuid::from_u128(1),
            timestamp: 1_790_000_000,
            from_name: str_field("Orion Example"),
            subject: str_field("Workshop"),
            has_attachment: true,
            asset_type: 0,
        }];
        let v = notices_reply(&m);
        assert_eq!(v[0].subject, "Workshop");
        assert!(v[0].has_attachment);
    }

    #[test]
    fn member_data_capability_fills_defaults() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(11);
        let mut members = aurora_llsd::Map::new();
        members.insert(
            a.to_string(),
            llsd_map! { "last_login" => "Online", "title" => 1, "powers" => "a", "owner" => "Y", "donated_square_meters" => 4096 },
        );
        members.insert(b.to_string(), llsd_map! { "last_login" => "09/12/2026" });
        members.insert(Uuid::nil().to_string(), llsd_map! {});
        let v = llsd_map! {
            "group_id" => Uuid::from_u128(1),
            "members" => members,
            "titles" => Llsd::Array(vec!["Member".into(), "Founder".into()]),
            "defaults" => llsd_map! { "default_powers" => "80000000000" },
        };
        let page = member_data_response(&v, Uuid::from_u128(7)).unwrap();
        assert!(page.whole);
        assert_eq!((page.group, page.request, page.total), (Uuid::from_u128(1), Uuid::from_u128(7), 2));
        let first = page.members.iter().find(|m| m.id == a).unwrap();
        assert_eq!(first.title, "Founder");
        assert_eq!(first.powers, 0xa);
        assert!(first.is_owner);
        assert_eq!(first.contribution, 4096);
        let second = page.members.iter().find(|m| m.id == b).unwrap();
        assert_eq!(second.title, "Member");
        assert_eq!(second.powers, 0x800_0000_0000);
        assert!(!second.is_owner);
        assert_eq!(member_data_body(Uuid::from_u128(1))["group_id"].as_uuid(), Uuid::from_u128(1));
        assert!(member_data_response(&llsd_map! {}, Uuid::nil()).is_none());
    }

    #[test]
    fn notice_buckets_and_texts() {
        let group = Uuid::from_u128(9);
        let mut b = vec![1u8, 6];
        b.extend_from_slice(group.as_bytes());
        b.extend_from_slice(b"Texture kit\0");
        let n = notice_bucket(&b).expect("bucket");
        assert!(n.has_inventory);
        assert_eq!((n.asset_type, n.group, n.item_name.as_str()), (6, group, "Texture kit"));
        assert!(notice_bucket(&b[..10]).is_none(), "too short");
        let mut open = b.clone();
        open.pop();
        assert!(notice_bucket(&open).is_none(), "not terminated");
        // an offline notice: the group from the entry, the attachment as text
        let n = notice_bucket(&notice_bucket_from_offline(group, b"6|0|Texture kit\0")).expect("offline bucket");
        assert!(n.has_inventory);
        assert_eq!((n.asset_type, n.group, n.item_name.as_str()), (6, group, "Texture kit"));
        let n = notice_bucket(&notice_bucket_from_offline(group, b"\0")).expect("offline bucket");
        assert!(!n.has_inventory);
        assert_eq!((n.group, n.item_name.as_str()), (group, ""));
        assert_eq!(notice_text("Hi|Body|more"), ("Hi".into(), "Body|more".into()));
        assert_eq!(notice_text("Hi"), ("Hi".into(), String::new()));
        let (text, bucket) = notice_to_send("Hi", "Body", None);
        assert_eq!((text.as_str(), bucket), ("Hi|Body", vec![0]));
        let (_, bucket) = notice_to_send("Hi", "Body", Some((Uuid::from_u128(1), Uuid::from_u128(2))));
        let v = aurora_llsd::from_xml(&bucket).expect("llsd");
        assert_eq!(v["item_id"].as_uuid(), Uuid::from_u128(1));
        assert_eq!(v["owner_id"].as_uuid(), Uuid::from_u128(2));
    }

    #[test]
    fn ejects_role_changes_and_bans() {
        let me = Uuid::from_u128(1000);
        let ids: Vec<Uuid> = (1..=60u128).map(Uuid::from_u128).chain([me]).collect();
        let v = eject_messages(me, Uuid::from_u128(1001), Uuid::from_u128(9), &ids);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].eject_data.len() + v[1].eject_data.len(), 60, "not ourselves");
        assert!(v.iter().all(|m| m.group_data.group_id == Uuid::from_u128(9)));

        let changes = [
            RoleChange {
                role: Uuid::from_u128(5),
                member: Uuid::from_u128(10),
                add: true,
            },
            RoleChange {
                role: Uuid::from_u128(6),
                member: Uuid::from_u128(10),
                add: false,
            },
        ];
        let m = &role_change_messages(me, Uuid::nil(), Uuid::from_u128(9), &changes)[0];
        assert_eq!(m.role_change.iter().map(|c| c.change).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(m.agent_data.group_id, Uuid::from_u128(9));

        let body = ban_body(ban::CREATE, &[Uuid::from_u128(10)]);
        assert_eq!(body["ban_action"].as_i32(), 1);
        assert_eq!(body["ban_ids"].as_array()[0].as_uuid(), Uuid::from_u128(10));
        let mut list = aurora_llsd::Map::new();
        list.insert(Uuid::from_u128(10).to_string(), llsd_map! {});
        let (g, bans) = ban_list(&llsd_map! { "group_id" => Uuid::from_u128(9), "ban_list" => list }).expect("list");
        assert_eq!(g, Uuid::from_u128(9));
        assert_eq!(bans, vec![(Uuid::from_u128(10), 0.0)]);
    }

    #[test]
    fn role_updates_and_group_info() {
        let u = |i: u128, kind| RoleUpdate {
            role: Uuid::from_u128(i),
            name: format!("Role {i}"),
            title: "T".into(),
            description: "D".into(),
            powers: 1 << 4,
            kind,
        };
        let v = role_update_messages(
            Uuid::from_u128(1),
            Uuid::nil(),
            Uuid::from_u128(9),
            &[
                u(1, role_change::CREATE),
                u(2, role_change::DATA),
                u(3, role_change::POWERS),
                u(4, role_change::DELETE),
            ],
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].agent_data.group_id, Uuid::from_u128(9));
        assert_eq!(v[0].role_data[0].update_type, role_change::CREATE);
        assert_eq!(field_str(&v[0].role_data[0].name), "Role 1");
        assert_eq!(v[1].role_data[0].update_type, role_change::DELETE);
        let info = GroupInfo {
            charter: "Hello".into(),
            open_enrollment: true,
            membership_fee: 50,
            ..Default::default()
        };
        let m = group_info_message(Uuid::from_u128(1), Uuid::nil(), Uuid::from_u128(9), &info);
        assert_eq!(field_str(&m.group_data.charter), "Hello");
        assert!(m.group_data.open_enrollment && m.group_data.membership_fee == 50);
    }

    #[test]
    fn invitations_go_by_a_hundred() {
        let pairs: Vec<(Uuid, Uuid)> = (1..=150u128).map(|i| (Uuid::from_u128(i), Uuid::nil())).collect();
        let v = invite_messages(Uuid::from_u128(1000), Uuid::from_u128(1001), Uuid::from_u128(9), &pairs);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].invite_data.len(), 100);
        assert_eq!(v[1].invite_data.len(), 50);
        assert_eq!(v[1].invite_data[0].invitee_id, Uuid::from_u128(101));
        assert!(
            v.iter()
                .all(|m| m.group_data.group_id == Uuid::from_u128(9) && m.agent_data.agent_id == Uuid::from_u128(1000))
        );
        assert!(invite_messages(Uuid::nil(), Uuid::nil(), Uuid::nil(), &[]).is_empty());
    }

    #[test]
    fn invite_roles_follow_the_assign_powers() {
        let role = |id: u128| GroupRole {
            id: Uuid::from_u128(id),
            name: String::new(),
            title: String::new(),
            description: String::new(),
            powers: 0,
            members: 0,
        };
        let roles = [role(0), role(1), role(2), role(3)];
        let owner = Uuid::from_u128(1);
        let ids = |v: Vec<&GroupRole>| v.iter().map(|r| r.id.as_u128()).collect::<Vec<_>>();
        assert_eq!(ids(invite_roles(&roles, &[], 0, owner, false)), [0]);
        assert_eq!(ids(invite_roles(&roles, &[], powers::ROLE_ASSIGN_MEMBER, owner, false)), [0, 2, 3]);
        assert_eq!(
            ids(invite_roles(&roles, &[], powers::ROLE_ASSIGN_MEMBER, owner, true)),
            [0, 1, 2, 3]
        );
        let mine = [Uuid::from_u128(3)];
        assert_eq!(
            ids(invite_roles(&roles, &mine, powers::ROLE_ASSIGN_MEMBER_LIMITED, owner, false)),
            [0, 3]
        );
    }
}
