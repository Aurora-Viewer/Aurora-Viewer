//! Demo group server: what the dataserver answers about the demo groups
//! (GroupMembersRequest, GroupRoleDataRequest, GroupRoleMembersRequest,
//! GroupTitlesRequest, GroupNoticesListRequest, JoinGroupRequest), so that
//! the group profile shows a full group offline. "Aurora Builders" has its
//! named members (the founder, an officer, a builder…) among 1248; our own
//! place in it follows `AURORA_DEMO_GROUP_PROFILE`:
//! - `1` or `member`: a builder (the default);
//! - `officer`: an officer, who may invite and sees the roles and donations;
//! - `owner`: an owner, every ability;
//! - `guest`: not a member: the profile opens on "Boreal Photographers",
//!   with an open enrollment and a fee, to join.
//!
//! `:members` after the scenario opens the Members tab, a builder selected.
//!
//! The other demo groups get generated members and the three roles every
//! group has.

use super::{DEMO_AGENT, DEMO_GROUP1, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use aurora_net::groups::{GroupMember, GroupRole, GroupTitle, LastSeen, MembersPage, NoticeSummary, RoleMembersPage, RolesPage, powers};
use aurora_net::{GroupMembership, NetCommand, NetEvent};
use std::collections::HashSet;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    Member,
    Officer,
    Owner,
    Guest,
}

pub fn scenario() -> Scenario {
    let v = std::env::var("AURORA_DEMO_GROUP_PROFILE").unwrap_or_default();
    match v.split(':').next().unwrap_or_default() {
        "officer" | "officier" => Scenario::Officer,
        "owner" | "proprietaire" | "propriétaire" => Scenario::Owner,
        "guest" | "visiteur" | "non-member" => Scenario::Guest,
        _ => Scenario::Member,
    }
}

/// `<scenario>:members`: the window opens on the Members tab, a builder
/// selected.
pub fn members_tab() -> Option<Uuid> {
    let v = std::env::var("AURORA_DEMO_GROUP_PROFILE").ok()?;
    (v.split(':').nth(1) == Some("members")).then_some(DEMO_NOVA)
}

/// The group the profile opens on in this scenario.
pub fn shown_group() -> Uuid {
    match scenario() {
        Scenario::Guest => super::search::boreal(),
        _ => DEMO_GROUP1,
    }
}

/// Ids of the generated members; the named ones come first.
const MEMBER: u128 = 0x6E6E_0000_0000_0000_0000_0000_0000_0000;
const LUNE: u128 = MEMBER | 1;
const PIXEL: u128 = MEMBER | 2;
const IRIS: u128 = MEMBER | 3;
/// The roles of "Aurora Builders" (the "everyone" role is nil).
const OWNERS: Uuid = Uuid::from_u128(0x6E40_0000_0000_0000_0000_0000_0000_0001);
const OFFICERS: Uuid = Uuid::from_u128(0x6E40_0000_0000_0000_0000_0000_0000_0002);
const BUILDERS: Uuid = Uuid::from_u128(0x6E40_0000_0000_0000_0000_0000_0000_0003);

/// GP_DEFAULT_MEMBER and GP_DEFAULT_OFFICER of roles_constants.h, as a new
/// group's roles have them.
const DEFAULT_MEMBER: u64 =
    powers::ACCOUNTING_ACCOUNTABLE | (1 << 28) | powers::NOTICES_RECEIVE | powers::SESSION_JOIN | powers::SESSION_VOICE;
const DEFAULT_OFFICER: u64 = DEFAULT_MEMBER
    | powers::GROUP_CHANGE_IDENTITY
    | powers::MEMBER_INVITE
    | powers::MEMBER_EJECT
    | powers::MEMBER_OPTIONS
    | powers::ROLE_ASSIGN_MEMBER_LIMITED
    | powers::NOTICES_SEND
    | powers::MEMBER_VISIBLE_IN_DIR;

/// Our powers in "Aurora Builders" (AgentGroupDataUpdate).
pub fn my_powers() -> u64 {
    match scenario() {
        Scenario::Owner => u64::MAX,
        Scenario::Officer => DEFAULT_OFFICER,
        _ => DEFAULT_MEMBER,
    }
}

/// Groups joined and left during the run: the group list follows them.
static JOINED: Mutex<Vec<GroupMembership>> = Mutex::new(Vec::new());
static LEFT: Mutex<Option<HashSet<Uuid>>> = Mutex::new(None);
static LISTED_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The group list of AgentDataUpdateRequest after this run's joins and
/// departures; true the first time (the demo's other first replies, a chat
/// invitation, come only then).
pub fn adjust_groups(list: &mut Vec<GroupMembership>) -> bool {
    if let Ok(left) = LEFT.lock()
        && let Some(left) = left.as_ref()
    {
        list.retain(|g| !left.contains(&g.id));
    }
    if let Ok(joined) = JOINED.lock() {
        list.extend(joined.iter().cloned());
    }
    // a guest is not in the profile's group
    if scenario() == Scenario::Guest {
        list.retain(|g| g.id != shown_group());
    }
    !LISTED_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed)
}

fn first_names() -> [&'static str; 24] {
    [
        "Aria", "Bjorn", "Cléo", "Dario", "Elsa", "Faye", "Gaël", "Hana", "Ivo", "Juno", "Kira", "Liam", "Mila", "Nils", "Oona", "Pavel",
        "Quinn", "Rhea", "Sven", "Talia", "Ulla", "Vito", "Wren", "Yara",
    ]
}

fn last_names() -> [&'static str; 6] {
    ["Resident", "Resident", "Resident", "Moonwalker", "Nordwind", "Starlight"]
}

/// The name of a generated member ("Lune Example", "pixelboutique"…), for
/// the demo's name replies: (first, last, display name).
pub fn member_name(id: &Uuid) -> Option<(&'static str, &'static str, &'static str)> {
    let n = id.as_u128();
    if n & !0xFFFF_FFFF != MEMBER {
        return None;
    }
    Some(match n {
        LUNE => ("Lune", aurora_i18n::tr!("demo-example"), ""),
        PIXEL => ("pixelboutique", "Resident", ""),
        IRIS => ("Iris", aurora_i18n::tr!("demo-example"), ""),
        _ => {
            let i = (n & 0xFFFF_FFFF) as usize;
            (first_names()[i % 24], last_names()[(i / 24) % 6], "")
        }
    })
}

/// A date `days` ago as the dataserver writes it (MM/DD/YYYY).
fn days_ago(days: i64) -> LastSeen {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = aurora_net::profile::civil_from_days((now - 8 * 3600).div_euclid(86_400) - days);
    LastSeen::Date(y as u16, m as u8, d as u8)
}

struct Spec {
    members: Vec<GroupMember>,
    roles: Vec<GroupRole>,
    pairs: Vec<(Uuid, Uuid)>,
}

fn role(id: Uuid, name: &str, title: &str, description: &str, powers: u64) -> GroupRole {
    GroupRole {
        id,
        name: name.into(),
        title: title.into(),
        description: description.into(),
        powers,
        members: 0,
    }
}

fn member(id: Uuid, title: &str, last_seen: LastSeen, powers: u64, owner: bool, contribution: i32) -> GroupMember {
    GroupMember {
        id,
        contribution,
        powers,
        title: title.into(),
        last_seen,
        is_owner: owner,
    }
}

/// The members, roles and pairs of a demo group.
fn spec(group: &Uuid) -> Option<Spec> {
    let founder_title = aurora_i18n::tr!("demo-group-founder");
    let officer_title = aurora_i18n::tr!("demo-group-officer");
    let member_title = aurora_i18n::tr!("demo-group-member");
    let builder_title = aurora_i18n::tr!("demo-builder");
    let count = super::search::member_count(group)?.max(1) as usize;
    let founder = super::search::founder(group)?;
    let mut members = Vec::new();
    let mut pairs = Vec::new();
    let mut roles = vec![
        role(Uuid::nil(), "Everyone", member_title, "", DEFAULT_MEMBER),
        role(OWNERS, "Owners", founder_title, "", u64::MAX),
        role(OFFICERS, "Officers", officer_title, "", DEFAULT_OFFICER),
    ];
    if *group == DEMO_GROUP1 {
        roles.push(role(BUILDERS, "Builders", builder_title, "", DEFAULT_MEMBER));
        let (my_title, my_owner) = match scenario() {
            Scenario::Owner => (founder_title, true),
            Scenario::Officer => (officer_title, false),
            _ => (builder_title, false),
        };
        members.push(member(DEMO_AGENT, my_title, LastSeen::Online, my_powers(), my_owner, 1024));
        pairs.push((BUILDERS, DEMO_AGENT));
        match scenario() {
            Scenario::Owner => pairs.extend([(OWNERS, DEMO_AGENT), (OFFICERS, DEMO_AGENT)]),
            Scenario::Officer => pairs.push((OFFICERS, DEMO_AGENT)),
            _ => {}
        }
        // the one the names show as "Nova ♪" is the officer, "Tess ✨" a builder
        members.push(member(DEMO_LOUP, founder_title, LastSeen::Online, u64::MAX, true, 4096));
        members.push(member(DEMO_TESS, officer_title, LastSeen::Online, DEFAULT_OFFICER, false, 0));
        members.push(member(DEMO_NOVA, builder_title, LastSeen::Online, DEFAULT_MEMBER, false, 512));
        members.push(member(
            Uuid::from_u128(LUNE),
            member_title,
            LastSeen::Online,
            DEFAULT_MEMBER,
            false,
            0,
        ));
        members.push(member(Uuid::from_u128(PIXEL), member_title, days_ago(3), DEFAULT_MEMBER, false, 0));
        members.push(member(Uuid::from_u128(IRIS), member_title, days_ago(32), DEFAULT_MEMBER, false, 0));
        pairs.extend([
            (OWNERS, DEMO_LOUP),
            (OFFICERS, DEMO_LOUP),
            (OFFICERS, DEMO_TESS),
            (BUILDERS, DEMO_NOVA),
        ]);
    } else {
        members.push(member(founder, founder_title, days_ago(1), u64::MAX, true, 0));
        pairs.push((OWNERS, founder));
    }
    let mut n = 4u128;
    while members.len() < count {
        let id = Uuid::from_u128(MEMBER | n);
        let i = n as i64;
        // a few builders and officers among them, the others last seen
        // over the past year
        let (title, extra) = match i % 37 {
            0 => (officer_title, Some(OFFICERS)),
            1..=4 if *group == DEMO_GROUP1 => (builder_title, Some(BUILDERS)),
            _ => (member_title, None),
        };
        let seen = days_ago(1 + (i * 7919) % 365);
        let contribution = if extra.is_some() { ((i % 8) * 256) as i32 } else { 0 };
        members.push(member(id, title, seen, DEFAULT_MEMBER, false, contribution));
        if let Some(r) = extra {
            pairs.push((r, id));
        }
        n += 1;
    }
    for r in &mut roles {
        r.members = if r.id.is_nil() {
            members.len() as u32
        } else {
            pairs.iter().filter(|(role, _)| *role == r.id).count() as u32
        };
    }
    Some(Spec { members, roles, pairs })
}

/// The titles we can wear in "Aurora Builders": those of our roles.
fn titles() -> Vec<GroupTitle> {
    let t = |title: &str, role: Uuid, selected: bool| GroupTitle {
        title: title.into(),
        role,
        selected,
    };
    let mut v = vec![
        t(aurora_i18n::tr!("demo-group-member"), Uuid::nil(), false),
        t(aurora_i18n::tr!("demo-builder"), BUILDERS, scenario() != Scenario::Owner),
    ];
    match scenario() {
        Scenario::Owner => {
            v.push(t(aurora_i18n::tr!("demo-group-officer"), OFFICERS, false));
            v.push(t(aurora_i18n::tr!("demo-group-founder"), OWNERS, true));
        }
        Scenario::Officer => v.push(t(aurora_i18n::tr!("demo-group-officer"), OFFICERS, false)),
        _ => {}
    }
    v
}

/// The notices of "Aurora Builders", newest first.
fn notices() -> Vec<NoticeSummary> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let n = |id: u128, ago: u32, from: &str, subject: &str, attachment: bool| NoticeSummary {
        id: Uuid::from_u128(0x6E50_0000_0000_0000_0000_0000_0000_0000 | id),
        timestamp: now.saturating_sub(ago),
        from: from.into(),
        subject: subject.into(),
        has_attachment: attachment,
        asset_type: if attachment { 0 } else { 255 },
    };
    let orion = format!("Orion {}", aurora_i18n::tr!("demo-example"));
    vec![
        n(1, 20 * 60, &orion, aurora_i18n::tr!("demo-group-notice-workshop"), true),
        n(
            2,
            26 * 3600,
            &format!("Nova {}", aurora_i18n::tr!("demo-example")),
            aurora_i18n::tr!("demo-group-notice-sandbox"),
            false,
        ),
        n(3, 11 * 86_400, &orion, aurora_i18n::tr!("demo-group-notice-rules"), false),
        n(
            4,
            24 * 86_400,
            "tesstouch Resident",
            aurora_i18n::tr!("demo-group-notice-photos"),
            true,
        ),
    ]
}

/// The charter of "Aurora Builders", with a place link.
fn charter() -> String {
    format!(
        "{}\n{} http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/64/200/24",
        aurora_i18n::tr!("demo-resident-in-aurora-viewer-s-offline-demo"),
        aurora_i18n::tr!("demo-group-charter")
    )
}

pub fn reply(cmd: &NetCommand) -> Option<Vec<NetEvent>> {
    Some(match cmd {
        NetCommand::GroupProfileRequest(group) if *group == DEMO_GROUP1 => {
            let mut events = super::search::reply(cmd)?;
            for e in &mut events {
                if let NetEvent::GroupProfile(g) = e {
                    g.charter = charter();
                    g.insignia = Uuid::nil();
                    g.founder = DEMO_LOUP;
                    g.powers = my_powers();
                    g.owner_role = OWNERS;
                    g.member_title = match scenario() {
                        Scenario::Owner => aurora_i18n::tr!("demo-group-founder").into(),
                        _ => aurora_i18n::tr!("demo-builder").into(),
                    };
                    if scenario() == Scenario::Guest {
                        g.member_title.clear();
                    }
                }
            }
            events
        }
        NetCommand::GroupMembersRequest { group, request } => {
            let s = spec(group)?;
            vec![NetEvent::GroupMembers(Box::new(MembersPage {
                group: *group,
                request: *request,
                total: s.members.len() as i32,
                members: s.members,
                whole: true,
            }))]
        }
        NetCommand::GroupRoleDataRequest { group, request } => {
            let s = spec(group)?;
            vec![NetEvent::GroupRoles(Box::new(RolesPage {
                group: *group,
                request: *request,
                total: s.roles.len() as i32,
                roles: s.roles,
            }))]
        }
        NetCommand::GroupRoleMembersRequest { group, request } => {
            let s = spec(group)?;
            vec![NetEvent::GroupRoleMembers(Box::new(RoleMembersPage {
                group: *group,
                request: *request,
                total_pairs: s.pairs.len() as u32,
                pairs: s.pairs,
            }))]
        }
        NetCommand::GroupTitlesRequest { group, request } => vec![NetEvent::GroupTitles {
            group: *group,
            request: *request,
            titles: if *group == DEMO_GROUP1 && scenario() != Scenario::Guest {
                titles()
            } else {
                Vec::new()
            },
        }],
        NetCommand::GroupNoticesListRequest(group) => vec![NetEvent::GroupNotices {
            group: *group,
            notices: if *group == DEMO_GROUP1 { notices() } else { Vec::new() },
        }],
        // the join is accepted: the group joins the list
        NetCommand::JoinGroup(group) => {
            let name = super::search::reply(&NetCommand::GroupProfileRequest(*group))
                .and_then(|e| {
                    e.into_iter().find_map(|e| match e {
                        NetEvent::GroupProfile(g) => Some(g.name),
                        _ => None,
                    })
                })
                .unwrap_or_default();
            let g = GroupMembership {
                id: *group,
                name,
                insignia: Uuid::nil(),
                powers: DEFAULT_MEMBER,
                accept_notices: true,
                list_in_profile: true,
                contribution: 0,
            };
            if let Ok(mut joined) = JOINED.lock() {
                joined.push(g.clone());
            }
            if let Ok(mut left) = LEFT.lock() {
                left.get_or_insert_with(HashSet::new).remove(group);
            }
            vec![
                NetEvent::Groups(vec![g]),
                NetEvent::GroupJoined {
                    group: *group,
                    success: true,
                },
            ]
        }
        NetCommand::LeaveGroup(group) => {
            if let Ok(mut left) = LEFT.lock() {
                left.get_or_insert_with(HashSet::new).insert(*group);
            }
            if let Ok(mut joined) = JOINED.lock() {
                joined.retain(|g| g.id != *group);
            }
            vec![
                NetEvent::GroupDropped(*group),
                NetEvent::GroupLeft {
                    group: *group,
                    success: true,
                },
            ]
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aurora_builders_has_its_named_members_among_all() {
        let s = spec(&DEMO_GROUP1).expect("demo group");
        assert_eq!(s.members.len(), 1248);
        let online = s.members.iter().filter(|m| m.last_seen == LastSeen::Online).count();
        assert_eq!(online, 5, "us, the founder, an officer, a builder and Lune");
        assert!(s.members.iter().any(|m| m.id == DEMO_LOUP && m.is_owner));
        assert!(
            s.pairs
                .iter()
                .all(|(r, m)| s.roles.iter().any(|x| x.id == *r) && s.members.iter().any(|x| x.id == *m))
        );
        let ids: HashSet<Uuid> = s.members.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), s.members.len(), "each member once");
        assert_eq!(member_name(&Uuid::from_u128(LUNE)).map(|n| n.0), Some("Lune"));
        assert!(member_name(&DEMO_LOUP).is_none());
    }
}
