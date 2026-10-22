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
//! `:members` after the scenario opens the Members tab, a builder selected;
//! `:invite` the invitation window with three residents (one already in);
//! `:notices` the Notices tab, its two latest notices unread (`:compose`
//! with a new notice open); `:land` the "Land and L$" tab (`:land-details`,
//! `:land-sales` on those pages): two parcels, our 512 m² given out of the
//! 1 024 we may give, and eight weeks of accounts; `:experiences` the
//! Experiences tab: the group owns the sandbox's doors, whose profile reads
//! and changes our permission.
//!
//! The other demo groups get generated members and the three roles every
//! group has.

use super::{DEMO_AGENT, DEMO_GROUP1, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use aurora_net::experience::{ExperienceDetails, ExperiencePermission, ExperiencePermissionChange};
use aurora_net::group_money::{
    AccountBody, AccountLine, AccountReply, AccountRequest, AccountSummary, GroupLandReply, GroupParcel, LandTotals, Ledger, Transaction,
    trans,
};
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

/// `<scenario>:admin`: the window opens on the Administration tab
/// (`:identity` on its Group identity section).
pub fn admin_tab() -> bool {
    std::env::var("AURORA_DEMO_GROUP_PROFILE").is_ok_and(|v| matches!(v.split(':').nth(1), Some("admin" | "identity")))
}

pub fn identity_section() -> bool {
    std::env::var("AURORA_DEMO_GROUP_PROFILE").is_ok_and(|v| v.split(':').nth(1) == Some("identity"))
}

/// `<scenario>:land`: the window opens on the "Land and L$" tab, on the
/// planning page (`:land-details`, `:land-sales` on the others).
pub fn land_tab() -> Option<Ledger> {
    let v = std::env::var("AURORA_DEMO_GROUP_PROFILE").ok()?;
    match v.split(':').nth(1)? {
        "land" => Some(Ledger::Planning),
        "land-details" => Some(Ledger::Details),
        "land-sales" => Some(Ledger::Sales),
        _ => None,
    }
}

/// `<scenario>:experiences`: the window opens on the Experiences tab.
pub fn experiences_tab() -> bool {
    std::env::var("AURORA_DEMO_GROUP_PROFILE").is_ok_and(|v| v.split(':').nth(1) == Some("experiences"))
}

/// The experience "Aurora Builders" owns: the sandbox's doors, land-scope.
pub const GROUP_EXPERIENCE: Uuid = Uuid::from_u128(0x6E60_0000_0000_4000_8000_0000_0000_0001);

/// The agent's permission for the group's experience during the run.
static GROUP_EXPERIENCE_PERMISSION: Mutex<Option<ExperiencePermission>> = Mutex::new(None);

fn group_experience() -> ExperienceDetails {
    ExperienceDetails {
        id: GROUP_EXPERIENCE,
        name: aurora_i18n::tr!("demo-group-experience").into(),
        description: aurora_i18n::tr!("demo-group-experience-description").into(),
        properties: 0,
        maturity: aurora_net::search::SIM_ACCESS_PG,
        owner: DEMO_LOUP,
        group: DEMO_GROUP1,
        slurl: "secondlife://Aurora%20D%C3%A9mo/64/200/24".into(),
    }
}

/// `<scenario>:members`: the window opens on the Members tab, a builder
/// selected.
pub fn members_tab() -> Option<Uuid> {
    let v = std::env::var("AURORA_DEMO_GROUP_PROFILE").ok()?;
    (v.split(':').nth(1) == Some("members")).then_some(DEMO_NOVA)
}

/// `<scenario>:invite`: the invitation window, with two residents who are
/// not in the group and one who is ("already in the group" once sent).
pub fn invitees() -> Option<Vec<Uuid>> {
    let v = std::env::var("AURORA_DEMO_GROUP_PROFILE").ok()?;
    (v.split(':').nth(1) == Some("invite")).then(|| vec![super::DEMO_FRIEND, Uuid::from_u128(MEMBER | 5000), DEMO_NOVA])
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
        // AURORA_DEMO_SOCIAL_MENU: someone to invite into it
        _ if super::social_menu::step().is_some() => DEFAULT_OFFICER,
        _ => DEFAULT_MEMBER,
    }
}

/// The demo group's ban list during the run.
static BANS: Mutex<Vec<(Uuid, f64)>> = Mutex::new(Vec::new());

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
    // a group created during the run: us, its owner, and the three roles a
    // new group gets
    if created(group).is_some() {
        let mut roles = vec![
            role(Uuid::nil(), "Everyone", member_title, "", DEFAULT_MEMBER),
            role(OWNERS, "Owners", founder_title, "", u64::MAX),
            role(OFFICERS, "Officers", officer_title, "", DEFAULT_OFFICER),
        ];
        roles[0].members = 1;
        roles[1].members = 1;
        return Some(Spec {
            members: vec![member(DEMO_AGENT, founder_title, LastSeen::Online, u64::MAX, true, 0)],
            roles,
            pairs: vec![(OWNERS, DEMO_AGENT)],
        });
    }
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
        members.push(member(
            DEMO_AGENT,
            my_title,
            LastSeen::Online,
            my_powers(),
            my_owner,
            MY_CONTRIBUTION,
        ));
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

/// The body of a demo notice and its attachment (name, asset type).
fn notice_body(id: u128) -> (String, Option<(&'static str, u8)>) {
    match id {
        1 => (
            aurora_i18n::tr!("demo-group-notice-workshop-body").into(),
            Some((aurora_i18n::tr!("demo-group-notice-workshop-kit"), 6)),
        ),
        2 => (aurora_i18n::tr!("demo-group-notice-sandbox-body").into(), None),
        3 => (aurora_i18n::tr!("demo-group-notice-rules-body").into(), None),
        _ => (
            aurora_i18n::tr!("demo-group-notice-photos-body").into(),
            Some((aurora_i18n::tr!("demo-group-notice-photos-album"), 0)),
        ),
    }
}

/// GroupNoticeRequest: the notice comes back as IM_GROUP_NOTICE_REQUESTED,
/// its attachment in the binary bucket (has inventory, asset type, group,
/// item name).
fn notice_im(notice: &Uuid) -> Option<NetEvent> {
    let n = notices().into_iter().find(|n| n.id == *notice)?;
    let (body, attachment) = notice_body(notice.as_u128() & 0xFF);
    let mut bucket = vec![u8::from(attachment.is_some()), attachment.map_or(0, |a| a.1)];
    bucket.extend_from_slice(DEMO_GROUP1.as_bytes());
    bucket.extend_from_slice(attachment.map_or("", |a| a.0).as_bytes());
    bucket.push(0);
    // from the group, the sender named in the message (the server's form:
    // process_improved_im looks the sender up by name)
    Some(NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: DEMO_GROUP1,
        from_name: n.from,
        to_agent_id: DEMO_AGENT,
        dialog: 37,
        session_id: Uuid::from_u128(0x6E51_0000_0000_0000_0000_0000_0000_0000 | (notice.as_u128() & 0xFF)),
        message: format!("{}|{body}", n.subject),
        offline: false,
        from_group: false,
        binary_bucket: bucket,
        timestamp: n.timestamp,
    }))
}

/// `<scenario>:notices`: the Notices tab, the two latest notices unread;
/// `:compose` on a new notice.
pub fn notices_tab() -> Option<(Uuid, Vec<Uuid>, bool)> {
    let v = std::env::var("AURORA_DEMO_GROUP_PROFILE").ok()?;
    let mode = v.split(':').nth(1)?;
    matches!(mode, "notices" | "compose").then(|| (DEMO_GROUP1, read_but_two(), mode == "compose"))
}

/// The first group's notices read, all but the two latest.
pub fn read_but_two() -> Vec<Uuid> {
    notices().iter().skip(2).map(|n| n.id).collect()
}

/// Our land given to "Aurora Builders" (AgentGroupDataUpdate), in m².
pub const MY_CONTRIBUTION: i32 = 512;
/// The land the demo account may hold and holds (MoneyBalanceReply): 512
/// m² left, so 1 024 m² we may give to the group.
pub const LAND_CREDIT: i32 = 4096;
pub const LAND_COMMITTED: i32 = 3584;

/// PlacesReply of "Aurora Builders": the totals block, then the sandbox of
/// the plaza and a workshop in the sandbox region (sent in two packets, as
/// a long list is).
fn group_land(group: Uuid, transaction: Uuid) -> Vec<NetEvent> {
    let parcel = |name: &str, sim: &str, (x, y): (f32, f32), area: i32| GroupParcel {
        name: name.into(),
        desc: String::new(),
        actual_area: area,
        billable_area: area,
        global_x: x,
        global_y: y,
        global_z: 24.0,
        sim_name: sim.into(),
        sku: "023".into(),
    };
    if group.is_nil() {
        return vec![NetEvent::GroupLand(Box::new(GroupLandReply {
            group,
            transaction,
            totals: None,
            parcels: vec![parcel(
                aurora_i18n::tr!("demo-group-land-workshop"),
                aurora_i18n::tr!("common-aurora-demo"),
                (256_140.0, 256_130.0),
                512,
            )],
        }))];
    }
    if group != DEMO_GROUP1 {
        return vec![NetEvent::GroupLand(Box::new(GroupLandReply {
            group,
            transaction,
            totals: Some(LandTotals::default()),
            parcels: Vec::new(),
        }))];
    }
    let sandbox = parcel(
        aurora_i18n::tr!("demo-group-land-sandbox"),
        aurora_i18n::tr!("common-aurora-demo"),
        (256_000.0 + 64.0, 256_000.0 + 200.0),
        4096,
    );
    let workshop = parcel(
        aurora_i18n::tr!("demo-group-land-workshop"),
        aurora_i18n::tr!("common-sandbox"),
        (255_744.0 + 30.0, 255_744.0 + 80.0),
        512,
    );
    let totals = LandTotals {
        in_use: sandbox.billable_area + workshop.billable_area,
        contributed: 5120,
    };
    vec![
        NetEvent::GroupLand(Box::new(GroupLandReply {
            group,
            transaction,
            totals: Some(totals),
            parcels: vec![sandbox],
        })),
        NetEvent::GroupLand(Box::new(GroupLandReply {
            group,
            transaction,
            totals: None,
            parcels: vec![workshop],
        })),
    ]
}

/// Days since 1970, in SLT.
fn today() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    (now - 8 * 3600).div_euclid(86_400)
}

/// Days since 1970 (SLT) of the Monday `weeks` back.
fn week_start(weeks: i32) -> i64 {
    let today = today();
    // 1970-01-01 was a Thursday
    today - (today + 3).rem_euclid(7) - 7 * i64::from(weeks)
}

fn date(days: i64) -> String {
    let (y, m, d) = aurora_net::profile::civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The accounts of "Aurora Builders", by week: the plaza's sales and the
/// members' gifts, less the land fees; nothing older than six weeks.
fn group_account(r: &AccountRequest) -> Vec<NetEvent> {
    let weeks = if r.ledger == Ledger::Planning { 0 } else { r.interval };
    let start = week_start(weeks);
    let reply = |body: AccountBody| {
        NetEvent::GroupAccount(Box::new(AccountReply {
            group: r.group,
            request: r.request,
            interval_days: aurora_net::group_money::INTERVAL_DAYS,
            interval: weeks,
            start_date: date(start),
            body,
        }))
    };
    let mine = r.group == DEMO_GROUP1;
    let w = weeks.max(0);
    let sales = if mine && w < 6 { 1050 - 150 * w } else { 0 };
    let gifts = if mine && w % 2 == 0 && w < 6 { 500 } else { 0 };
    let fee = if mine { -300 } else { 0 };
    match r.ledger {
        Ledger::Planning => vec![reply(AccountBody::Summary(AccountSummary {
            start_date: date(start),
            balance: if mine { 1250 } else { 0 },
            total_credits: sales + gifts,
            total_debits: fee,
            next_stipend: date(start + 7),
        }))],
        Ledger::Details => {
            let line = |key: &str, amount: i32| AccountLine {
                description: key.into(),
                amount,
            };
            vec![reply(AccountBody::Details(vec![
                line(aurora_i18n::tr!("demo-group-money-sales"), sales),
                line(aurora_i18n::tr!("demo-group-money-gifts"), gifts),
                line(aurora_i18n::tr!("demo-group-money-land-fee"), fee),
            ]))]
        }
        Ledger::Sales => {
            // this week's sales are not later than today
            let at = |day: i64, time: &str| format!("{} {time}", date((start + day).min(today())));
            let sale = |time: String, user: String, kind: i32, item: &str, amount: i32| Transaction {
                time,
                user,
                kind,
                item: item.into(),
                amount,
            };
            let example = aurora_i18n::tr!("demo-example");
            let mut list = Vec::new();
            if sales > 0 {
                list.push(sale(
                    at(1, "14:22"),
                    format!("Nova {example}"),
                    trans::OBJECT_SALE,
                    aurora_i18n::tr!("demo-group-notice-workshop-kit"),
                    250,
                ));
                list.push(sale(
                    at(2, "09:05"),
                    "tesstouch Resident".into(),
                    trans::PAY_OBJECT,
                    aurora_i18n::tr!("demo-group-money-tip-jar"),
                    sales - 300,
                ));
                list.push(sale(
                    at(3, "21:40"),
                    format!("Iris {example}"),
                    trans::LAND_PASS_SALE,
                    aurora_i18n::tr!("demo-group-land-sandbox"),
                    50,
                ));
            }
            if gifts > 0 {
                list.push(sale(at(4, "18:12"), format!("Lune {example}"), trans::GIFT, "", gifts));
            }
            // two packets, the second appended to the first
            let second = list.split_off(list.len().min(2));
            let mut events = vec![reply(AccountBody::Sales(list))];
            if !second.is_empty() {
                events.push(reply(AccountBody::Sales(second)));
            }
            events
        }
    }
}

/// The charter of "Aurora Builders", with a place link.
fn charter() -> String {
    format!(
        "{}\n{} http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/64/200/24",
        aurora_i18n::tr!("demo-resident-in-aurora-viewer-s-offline-demo"),
        aurora_i18n::tr!("demo-group-charter")
    )
}

/// `AURORA_DEMO_GROUP_CREATE=<mode>[:classic]`: the "Create a group" window
/// - `form`: empty, as opened from People › Groups;
/// - `cost`: filled, "Create" clicked: the fee to confirm;
/// - `refused`: sent with the name of a demo group: the server refuses it;
/// - `limit`: the account at its group limit (its three groups);
/// - `created`: sent: the new group's profile, the fee taken from the balance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CreateScenario {
    Form,
    Cost,
    Refused,
    Limit,
    Created,
}

pub fn create_scenario() -> Option<CreateScenario> {
    let v = std::env::var("AURORA_DEMO_GROUP_CREATE").ok()?;
    Some(match v.split(':').next().unwrap_or_default() {
        "cost" | "cout" | "coût" => CreateScenario::Cost,
        "refused" | "refuse" | "refusé" => CreateScenario::Refused,
        "limit" | "limite" => CreateScenario::Limit,
        "created" | "cree" | "créé" => CreateScenario::Created,
        _ => CreateScenario::Form,
    })
}

/// `<mode>:classic`: the classic look (Firestorm's creation panel, and the
/// classic profile of the new group).
pub fn create_classic() -> bool {
    std::env::var("AURORA_DEMO_GROUP_CREATE").is_ok_and(|v| v.split(':').any(|p| p == "classic"))
}

/// The fee of a new group by membership in the creation scenarios (the
/// login's `premium_packages`; the demo's own figures).
const CREATE_COSTS: [(&str, i32, i32); 3] = [("Base", 100, 42), ("Premium", 50, 70), ("Premium_Plus", 0, 140)];

/// What the login says of the account in the creation scenarios: a Base
/// membership, its fee and group limit (the demo's three groups for
/// `limit`), and the other memberships.
pub fn login_benefits() -> Option<Vec<(&'static str, aurora_llsd::Llsd)>> {
    use aurora_llsd::llsd_map;
    let scenario = create_scenario()?;
    let benefits = |cost: i32, limit: i32| llsd_map! { "create_group_cost" => cost, "group_membership_limit" => limit };
    let (_, cost, limit) = CREATE_COSTS[0];
    let limit = if scenario == CreateScenario::Limit { 3 } else { limit };
    let mut packages = aurora_llsd::Llsd::new_map();
    for (name, cost, limit) in CREATE_COSTS {
        packages.insert(name, llsd_map! { "benefits" => benefits(cost, limit) });
    }
    Some(vec![
        ("account_type", "Base".into()),
        ("account_level_benefits", benefits(cost, limit)),
        ("premium_packages", packages),
    ])
}

/// The name the scenarios type: a new one, or a demo group's (`refused`).
pub fn create_name(scenario: CreateScenario) -> &'static str {
    match scenario {
        CreateScenario::Refused => "Aurora Builders",
        _ => "Lagoon Painters",
    }
}

/// The groups created during the run (their profile).
static CREATED: Mutex<Vec<aurora_net::search::GroupProfile>> = Mutex::new(Vec::new());

fn created(group: &Uuid) -> Option<aurora_net::search::GroupProfile> {
    CREATED.lock().ok()?.iter().find(|g| g.id == *group).cloned()
}

/// The dataserver's answer to CreateGroupRequest: refused when a group has
/// the name already, at the group limit or without the fee in the balance;
/// else the group is ours (its only member, an owner), the fee taken.
pub fn create(name: &str, info: &aurora_net::groups::GroupInfo, balance: i32, fee: i32, can_join: bool) -> Vec<NetEvent> {
    let taken = super::search::group_named(name)
        || [
            "Aurora Builders",
            aurora_i18n::tr!("demo-northern-wolves"),
            aurora_i18n::tr!("common-plaza-market"),
        ]
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
        || CREATED.lock().is_ok_and(|c| c.iter().any(|g| g.name.eq_ignore_ascii_case(name)));
    let refused = if taken {
        Some(aurora_i18n::tr!("demo-group-create-taken"))
    } else if !can_join {
        Some(aurora_i18n::tr!("demo-group-create-limit"))
    } else if balance < fee {
        Some(aurora_i18n::tr!("demo-group-create-funds"))
    } else {
        None
    };
    if let Some(message) = refused {
        log::info!("demo group: creation of \"{name}\" refused");
        return vec![NetEvent::GroupCreated {
            group: Uuid::nil(),
            success: false,
            message: message.to_owned(),
        }];
    }
    let mut store = CREATED.lock().unwrap_or_else(|e| e.into_inner());
    let id = Uuid::from_u128(0x6E00_C4EA_7E00_0000_0000_0000_0000_0000 | (store.len() as u128 + 1));
    store.push(aurora_net::search::GroupProfile {
        id,
        name: name.to_owned(),
        charter: info.charter.clone(),
        show_in_list: info.show_in_list,
        member_title: aurora_i18n::tr!("demo-group-founder").into(),
        powers: u64::MAX,
        insignia: info.insignia,
        founder: DEMO_AGENT,
        membership_fee: info.membership_fee,
        open_enrollment: info.open_enrollment,
        members: 1,
        roles: 3,
        allow_publish: info.allow_publish,
        mature_publish: info.mature_publish,
        owner_role: OWNERS,
    });
    drop(store);
    let g = GroupMembership {
        id,
        name: name.to_owned(),
        insignia: info.insignia,
        powers: u64::MAX,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    };
    if let Ok(mut joined) = JOINED.lock() {
        joined.push(g.clone());
    }
    log::info!("demo group: \"{name}\" created for L$ {fee}");
    vec![
        NetEvent::Groups(vec![g]),
        NetEvent::Balance(balance - fee),
        NetEvent::GroupCreated {
            group: id,
            success: true,
            message: String::new(),
        },
    ]
}

pub fn reply(cmd: &NetCommand) -> Option<Vec<NetEvent>> {
    Some(match cmd {
        NetCommand::GroupProfileRequest(group) if created(group).is_some() => {
            vec![NetEvent::GroupProfile(Box::new(created(group)?))]
        }
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
        NetCommand::GroupNoticeRequest(notice) => vec![notice_im(notice)?],
        // the dataserver ejects them; a ban answers the group's ban list
        NetCommand::EjectGroupMembers { group, members } => {
            log::info!("demo group: {} member(s) ejected", members.len());
            vec![NetEvent::GroupEjected {
                group: *group,
                success: true,
            }]
        }
        NetCommand::GroupBans { group, action, ids, .. } => {
            log::info!("demo group: ban action {action:?} for {} resident(s)", ids.len());
            let mut list = BANS.lock().map(|b| b.clone()).unwrap_or_default();
            if list.is_empty() {
                list = vec![
                    (Uuid::from_u128(MEMBER | 9000), 1_788_000_000.0),
                    (Uuid::from_u128(MEMBER | 9001), 1_789_500_000.0),
                ];
            }
            match action {
                Some(a) if *a == aurora_net::groups::ban::CREATE => list.extend(ids.iter().map(|id| (*id, 1_790_000_000.0))),
                Some(a) if *a == aurora_net::groups::ban::DELETE => list.retain(|(id, _)| !ids.contains(id)),
                _ => {}
            }
            if let Ok(mut b) = BANS.lock() {
                b.clone_from(&list);
            }
            vec![NetEvent::GroupBanList { group: *group, bans: list }]
        }
        NetCommand::GroupRoleUpdate { updates, .. } => {
            log::info!("demo group: {} role update(s)", updates.len());
            Vec::new()
        }
        NetCommand::UpdateGroupInfo { info, .. } => {
            log::info!(
                "demo group: identity saved (open enrollment {}, fee {})",
                info.open_enrollment,
                info.membership_fee
            );
            Vec::new()
        }
        NetCommand::GroupRoleChanges { changes, .. } => {
            log::info!("demo group: {} role change(s)", changes.len());
            Vec::new()
        }
        // a notice sent to the group: every member gets it, us too
        NetCommand::SendImDialog {
            to,
            dialog: 32,
            message,
            bucket,
            ..
        } if *to == DEMO_GROUP1 => {
            log::info!("demo group: notice sent ({} bytes attached)", bucket.len().saturating_sub(1));
            vec![NetEvent::InstantMessage(aurora_net::InstantMessage {
                from_agent_id: DEMO_GROUP1,
                from_name: "Aurora Demo".into(),
                to_agent_id: DEMO_AGENT,
                dialog: 32,
                session_id: Uuid::new_v4(),
                message: message.clone(),
                offline: false,
                from_group: true,
                binary_bucket: Vec::new(),
                timestamp: 0,
            })]
        }
        // a notice's attachment saved: the item lands in the folder
        NetCommand::SendImDialog { dialog: 33, .. } => {
            log::info!("demo group: notice attachment accepted");
            Vec::new()
        }
        NetCommand::GroupLandQuery { group, transaction } => group_land(*group, *transaction),
        // getGroupExperiencesCoro: "Aurora Builders" owns one
        NetCommand::GroupExperiences(group) => vec![NetEvent::GroupExperiences {
            group: *group,
            ids: if *group == DEMO_GROUP1 {
                vec![GROUP_EXPERIENCE]
            } else {
                Vec::new()
            },
        }],
        NetCommand::ExperienceInfo(ids) if ids.contains(&GROUP_EXPERIENCE) => vec![NetEvent::ExperienceInfo(
            ids.iter()
                .map(|id| {
                    if *id == GROUP_EXPERIENCE {
                        group_experience()
                    } else {
                        ExperienceDetails::unknown(*id)
                    }
                })
                .collect(),
        )],
        NetCommand::ExperiencePermission { experience, change } if *experience == GROUP_EXPERIENCE => {
            let mut stored = GROUP_EXPERIENCE_PERMISSION.lock().ok();
            let permission = match change {
                ExperiencePermissionChange::Query => stored.as_ref().and_then(|s| **s).unwrap_or(ExperiencePermission::Unset),
                ExperiencePermissionChange::Allow => ExperiencePermission::Allowed,
                ExperiencePermissionChange::Block => ExperiencePermission::Blocked,
                ExperiencePermissionChange::Forget => ExperiencePermission::Unset,
            };
            if let Some(s) = stored.as_mut() {
                **s = Some(permission);
            }
            log::info!("demo group: experience permission {change:?} -> {permission:?}");
            vec![NetEvent::ExperiencePermission {
                experience: *experience,
                change: *change,
                result: Ok(permission),
            }]
        }
        NetCommand::GroupAccount(r) => group_account(r),
        // the dataserver keeps it and answers nothing
        NetCommand::SetGroupContribution { contribution, .. } => {
            log::info!("demo group: land contribution set to {contribution} m²");
            Vec::new()
        }
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
    fn the_dataserver_creates_a_group_and_takes_its_fee() {
        let info = aurora_net::groups::GroupInfo {
            show_in_list: true,
            ..Default::default()
        };
        let refused = |events: Vec<NetEvent>| matches!(events.as_slice(), [NetEvent::GroupCreated { success: false, .. }]);
        assert!(refused(create("aurora builders", &info, 1000, 100, true)), "a demo group's name");
        assert!(refused(create("Test Fee Group", &info, 50, 100, true)), "not enough L$");
        assert!(refused(create("Test Limit Group", &info, 1000, 100, false)), "the group limit");
        let events = create("Test Created Group", &info, 1000, 100, true);
        let Some(NetEvent::GroupCreated { group, success: true, .. }) = events.last() else {
            panic!("created: {events:?}");
        };
        assert!(events.iter().any(|e| matches!(e, NetEvent::Balance(900))));
        assert!(events.iter().any(|e| matches!(e, NetEvent::Groups(g) if g[0].id == *group)));
        // its profile and its data: us, its owner
        let profile = reply(&NetCommand::GroupProfileRequest(*group)).expect("profile");
        assert!(matches!(&profile[0], NetEvent::GroupProfile(g) if g.name == "Test Created Group" && g.founder == DEMO_AGENT));
        let s = spec(group).expect("roles and members");
        assert_eq!(s.members.len(), 1);
        assert!(s.members[0].is_owner);
        assert!(
            refused(create("Test Created Group", &info, 1000, 100, true)),
            "the name is taken now"
        );
    }

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
