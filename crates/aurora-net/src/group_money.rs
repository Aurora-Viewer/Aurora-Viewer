//! A group's land and L$ (LLPanelGroupLandMoney, indra/newview/
//! llpanelgrouplandmoney.cpp, llviewermessage.cpp send_places_query /
//! process_places_reply, llagent.cpp setGroupContribution, originally LGPL
//! 2.1): the parcels the group owns (PlacesQuery with DFQ_GROUP_OWNED,
//! answered by PlacesReply, through the event queue on Second Life), the
//! land its members contribute (SetGroupContribution), and its accounts
//! (GroupAccountSummaryRequest, GroupAccountDetailsRequest,
//! GroupAccountTransactionsRequest, by week).

use aurora_llsd::Llsd;
use aurora_msg::msgs;
use aurora_msg::{field_str, str_field};
use uuid::Uuid;

/// DFQ_GROUP_OWNED (llqueryflags.h): the parcels owned by the group of the
/// query id.
pub const DFQ_GROUP_OWNED: u32 = 1 << 8;
/// DFQ_AGENT_OWNED, LLFloaterLandHoldings::onOpen.
pub const DFQ_AGENT_OWNED: u32 = 1 << 6;
/// LLParcel::C_ANY: no category filter.
const CATEGORY_ANY: i8 = -1;

/// SUMMARY_INTERVAL and SUMMARY_MAX (lluiconstants.h): the accounts go by
/// week, eight weeks back at most.
pub const INTERVAL_DAYS: i32 = 7;
pub const MAX_INTERVAL: i32 = 8;

/// Transaction types named in the sales (lltransactiontypes.h).
pub mod trans {
    pub const OBJECT_SALE: i32 = 5000;
    pub const GIFT: i32 = 5001;
    pub const LAND_PASS_SALE: i32 = 5006;
    pub const PAY_OBJECT: i32 = 5008;
    pub const EVENT_FEE: i32 = 9003;
    pub const EVENT_PRIZE: i32 = 9004;
}

/// One parcel of the group (a QueryData block of PlacesReply).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupParcel {
    pub name: String,
    pub desc: String,
    pub actual_area: i32,
    pub billable_area: i32,
    /// Global position, in metres.
    pub global_x: f32,
    pub global_y: f32,
    pub global_z: f32,
    pub sim_name: String,
    /// ProductSKU: the land type, named by ProductInfoRequest (empty on the
    /// UDP form).
    pub sku: String,
}

/// The land the group uses and holds (processGroupLand's special first
/// block, owner nil).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LandTotals {
    /// "Total land in use".
    pub in_use: i32,
    /// "Total contribution".
    pub contributed: i32,
}

impl LandTotals {
    /// "Land available": negative when more land is in use than given
    /// (the "more land credits are needed" warning).
    pub fn available(&self) -> i32 {
        self.contributed - self.in_use
    }
}

/// PlacesReply for a group (its id is the query id).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupLandReply {
    pub group: Uuid,
    pub transaction: Uuid,
    pub totals: Option<LandTotals>,
    pub parcels: Vec<GroupParcel>,
}

/// The blocks of a PlacesReply, in either form.
struct Block {
    owner: Uuid,
    parcel: GroupParcel,
}

/// LLPanelGroupLandMoney::impl::processGroupLand: a first block without an
/// owner holds the totals (its ActualArea the contribution; the land in use
/// is its BillableArea when it is alone, else the sum of the parcels'),
/// the parcels follow.
fn group_land(group: Uuid, transaction: Uuid, blocks: Vec<Block>) -> GroupLandReply {
    if group.is_nil() {
        // A null query id identifies the agent's own holdings, not group
        // totals. A nil owner is the server's empty-list sentinel.
        return GroupLandReply {
            group,
            transaction,
            totals: None,
            parcels: blocks.into_iter().filter(|b| !b.owner.is_nil()).map(|b| b.parcel).collect(),
        };
    }
    let mut totals = None;
    let mut parcels: Vec<GroupParcel> = Vec::new();
    let mut blocks = blocks.into_iter();
    let mut first = blocks.next();
    if let Some(b) = first.take_if(|b| b.owner.is_nil()) {
        totals = Some(b.parcel);
    }
    parcels.extend(first.map(|b| b.parcel));
    parcels.extend(blocks.map(|b| b.parcel));
    let totals = totals.map(|t| LandTotals {
        in_use: if parcels.is_empty() {
            t.billable_area
        } else {
            parcels.iter().map(|p| p.billable_area).sum()
        },
        contributed: t.actual_area,
    });
    GroupLandReply {
        group,
        transaction,
        totals,
        parcels,
    }
}

/// send_places_query(group, transaction, "", DFQ_GROUP_OWNED, C_ANY, "").
pub fn places_query(agent: Uuid, session: Uuid, group: Uuid, transaction: Uuid) -> msgs::PlacesQuery {
    let mut m = msgs::PlacesQuery::default();
    m.agent_data.agent_id = agent;
    m.agent_data.session_id = session;
    m.agent_data.query_id = group;
    m.transaction_data.transaction_id = transaction;
    m.query_data.query_text = str_field("");
    m.query_data.query_flags = if group.is_nil() { DFQ_AGENT_OWNED } else { DFQ_GROUP_OWNED };
    m.query_data.category = CATEGORY_ANY;
    m.query_data.sim_name = str_field("");
    m
}

/// The UDP form, without ProductSKU: (agent, reply).
pub fn places_reply(m: &msgs::PlacesReply) -> (Uuid, GroupLandReply) {
    let blocks = m
        .query_data
        .iter()
        .map(|q| Block {
            owner: q.owner_id,
            parcel: GroupParcel {
                name: field_str(&q.name),
                desc: field_str(&q.desc),
                actual_area: q.actual_area,
                billable_area: q.billable_area,
                global_x: q.global_x,
                global_y: q.global_y,
                global_z: q.global_z,
                sim_name: field_str(&q.sim_name),
                sku: String::new(),
            },
        })
        .collect();
    (
        m.agent_data.agent_id,
        group_land(m.agent_data.query_id, m.transaction_data.transaction_id, blocks),
    )
}

/// PlacesReply as Second Life sends it, through the event queue
/// (UDPDeprecated in the template): the same blocks as LLSD, plus
/// ProductSKU. (agent, reply).
pub fn places_reply_llsd(b: &Llsd) -> (Uuid, GroupLandReply) {
    let blocks = b["QueryData"]
        .as_array()
        .iter()
        .map(|q| Block {
            owner: q["OwnerID"].as_uuid(),
            parcel: GroupParcel {
                name: q["Name"].to_string_value(),
                desc: q["Desc"].to_string_value(),
                actual_area: q["ActualArea"].as_i32(),
                billable_area: q["BillableArea"].as_i32(),
                global_x: q["GlobalX"].as_f32(),
                global_y: q["GlobalY"].as_f32(),
                global_z: q["GlobalZ"].as_f32(),
                sim_name: q["SimName"].to_string_value(),
                sku: q["ProductSKU"].to_string_value(),
            },
        })
        .collect();
    (
        b["AgentData"][0]["AgentID"].as_uuid(),
        group_land(
            b["AgentData"][0]["QueryID"].as_uuid(),
            b["TransactionData"][0]["TransactionID"].as_uuid(),
            blocks,
        ),
    )
}

/// LLAgent::setGroupContribution.
pub fn contribution_message(agent: Uuid, session: Uuid, group: Uuid, contribution: i32) -> msgs::SetGroupContribution {
    let mut m = msgs::SetGroupContribution::default();
    m.agent_data.agent_id = agent;
    m.agent_data.session_id = session;
    m.data.group_id = group;
    m.data.contribution = contribution;
    m
}

/// The three pages of the group's L$.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Ledger {
    /// GroupAccountSummaryRequest, always the current week.
    #[default]
    Planning,
    /// GroupAccountDetailsRequest.
    Details,
    /// GroupAccountTransactionsRequest.
    Sales,
}

/// What a request asks: `interval` weeks back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountRequest {
    pub ledger: Ledger,
    pub group: Uuid,
    pub request: Uuid,
    pub interval: i32,
}

/// The message of one page.
pub enum AccountMessage {
    Summary(msgs::GroupAccountSummaryRequest),
    Details(msgs::GroupAccountDetailsRequest),
    Sales(msgs::GroupAccountTransactionsRequest),
}

/// The request of a page (requestData of its LLGroupMoney…TabEventHandler):
/// the planning page always asks the current week.
pub fn account_message(agent: Uuid, session: Uuid, r: &AccountRequest) -> AccountMessage {
    macro_rules! fill {
        ($m:expr, $interval:expr) => {{
            let mut m = $m;
            m.agent_data.agent_id = agent;
            m.agent_data.session_id = session;
            m.agent_data.group_id = r.group;
            m.money_data.request_id = r.request;
            m.money_data.interval_days = INTERVAL_DAYS;
            m.money_data.current_interval = $interval;
            m
        }};
    }
    match r.ledger {
        Ledger::Planning => AccountMessage::Summary(fill!(msgs::GroupAccountSummaryRequest::default(), 0)),
        Ledger::Details => AccountMessage::Details(fill!(msgs::GroupAccountDetailsRequest::default(), r.interval)),
        Ledger::Sales => AccountMessage::Sales(fill!(msgs::GroupAccountTransactionsRequest::default(), r.interval)),
    }
}

/// GroupAccountSummaryReply (the "Planning" page).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccountSummary {
    /// StartDate, "YYYY-MM-DD".
    pub start_date: String,
    pub balance: i32,
    pub total_credits: i32,
    pub total_debits: i32,
    /// TaxDate: the next stipend day, "YYYY-MM-DD".
    pub next_stipend: String,
}

/// One line of the details: a description and its amount.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountLine {
    pub description: String,
    pub amount: i32,
}

/// One sale (a HistoryData block of GroupAccountTransactionsReply).
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    pub time: String,
    pub user: String,
    pub kind: i32,
    pub item: String,
    pub amount: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccountBody {
    Summary(AccountSummary),
    Details(Vec<AccountLine>),
    /// One packet of sales; several can answer one request.
    Sales(Vec<Transaction>),
}

/// A reply to one of the account requests.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountReply {
    pub group: Uuid,
    pub request: Uuid,
    pub interval_days: i32,
    pub interval: i32,
    /// StartDate, "YYYY-MM-DD".
    pub start_date: String,
    pub body: AccountBody,
}

/// (agent, reply).
pub fn summary_reply(m: &msgs::GroupAccountSummaryReply) -> (Uuid, AccountReply) {
    let d = &m.money_data;
    let start_date = field_str(&d.start_date);
    (
        m.agent_data.agent_id,
        AccountReply {
            group: m.agent_data.group_id,
            request: d.request_id,
            interval_days: d.interval_days,
            interval: d.current_interval,
            start_date: start_date.clone(),
            body: AccountBody::Summary(AccountSummary {
                start_date,
                balance: d.balance,
                total_credits: d.total_credits,
                total_debits: d.total_debits,
                next_stipend: field_str(&d.tax_date),
            }),
        },
    )
}

/// (agent, reply).
pub fn details_reply(m: &msgs::GroupAccountDetailsReply) -> (Uuid, AccountReply) {
    let d = &m.money_data;
    (
        m.agent_data.agent_id,
        AccountReply {
            group: m.agent_data.group_id,
            request: d.request_id,
            interval_days: d.interval_days,
            interval: d.current_interval,
            start_date: field_str(&d.start_date),
            body: AccountBody::Details(
                m.history_data
                    .iter()
                    .map(|h| AccountLine {
                        description: field_str(&h.description),
                        amount: h.amount,
                    })
                    .collect(),
            ),
        },
    )
}

/// (agent, reply).
pub fn transactions_reply(m: &msgs::GroupAccountTransactionsReply) -> (Uuid, AccountReply) {
    let d = &m.money_data;
    (
        m.agent_data.agent_id,
        AccountReply {
            group: m.agent_data.group_id,
            request: d.request_id,
            interval_days: d.interval_days,
            interval: d.current_interval,
            start_date: field_str(&d.start_date),
            body: AccountBody::Sales(
                m.history_data
                    .iter()
                    .map(|h| Transaction {
                        time: field_str(&h.time),
                        user: field_str(&h.user),
                        kind: h.type_,
                        item: field_str(&h.item),
                        amount: h.amount,
                    })
                    .collect(),
            ),
        },
    )
}

/// "YYYY-MM-DD" (LLDateUtil::secondsSinceEpochFromString("%Y-%m-%d")):
/// (year, month, day).
pub fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let mut parts = text.trim().splitn(3, '-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.get(..2).unwrap_or_default().trim().parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn block(owner: u128, name: &str, actual: i32, billable: i32) -> msgs::places_reply::QueryData {
        msgs::places_reply::QueryData {
            owner_id: Uuid::from_u128(owner),
            name: str_field(name),
            actual_area: actual,
            billable_area: billable,
            global_x: 256_064.0,
            global_y: 256_200.0,
            sim_name: str_field("Aurora Démo"),
            ..Default::default()
        }
    }

    #[test]
    fn the_first_block_without_owner_holds_the_totals() {
        let g = Uuid::from_u128(7);
        let mut m = msgs::PlacesReply::default();
        m.agent_data.query_id = g;
        m.transaction_data.transaction_id = Uuid::from_u128(9);
        m.query_data = vec![
            block(0, "", 5120, 0),
            block(7, "Sandbox", 4096, 4096),
            block(7, "Workshop", 512, 512),
        ];
        let (_, r) = places_reply(&m);
        assert_eq!((r.group, r.transaction), (g, Uuid::from_u128(9)));
        assert_eq!(
            r.totals,
            Some(LandTotals {
                in_use: 4608,
                contributed: 5120
            })
        );
        assert_eq!(r.totals.map(|t| t.available()), Some(512));
        assert_eq!(r.parcels.len(), 2);
        assert_eq!(r.parcels[1].name, "Workshop");
        assert_eq!(r.parcels[0].sim_name, "Aurora Démo");
    }

    #[test]
    fn owned_land_uses_the_agent_query_and_never_group_totals() {
        let query = places_query(Uuid::from_u128(1), Uuid::from_u128(2), Uuid::nil(), Uuid::from_u128(3));
        assert_eq!(query.agent_data.query_id, Uuid::nil());
        assert_eq!(query.query_data.query_flags, DFQ_AGENT_OWNED);
        assert_eq!(query.query_data.category, -1);
        let mut reply = msgs::PlacesReply {
            query_data: vec![block(1, "Home", 512, 512), block(1, "Workshop", 1024, 2048)],
            ..Default::default()
        };
        let (_, result) = places_reply(&reply);
        assert_eq!(result.totals, None);
        assert_eq!(result.parcels.len(), 2);
        assert_eq!(result.parcels[0].name, "Home");
        reply.query_data = vec![block(0, "", 0, 0)];
        assert!(places_reply(&reply).1.parcels.is_empty());
    }

    #[test]
    fn a_group_without_parcels_reads_its_land_in_use_from_the_totals_block() {
        let mut m = msgs::PlacesReply {
            query_data: vec![block(0, "", 1024, 2048)],
            ..Default::default()
        };
        m.agent_data.query_id = Uuid::from_u128(7);
        let (_, r) = places_reply(&m);
        assert_eq!(
            r.totals,
            Some(LandTotals {
                in_use: 2048,
                contributed: 1024
            })
        );
        assert_eq!(r.totals.map(|t| t.available()), Some(-1024));
        assert!(r.parcels.is_empty());
        // a later packet carries parcels only
        m.query_data = vec![block(7, "Sandbox", 4096, 4096)];
        let (_, r) = places_reply(&m);
        assert_eq!((r.totals, r.parcels.len()), (None, 1));
    }

    #[test]
    fn the_event_queue_form_carries_the_land_type() {
        let agent = Uuid::from_u128(1);
        let b = llsd_map! {
            "AgentData" => Llsd::Array(vec![llsd_map! { "AgentID" => agent, "QueryID" => Uuid::from_u128(7) }]),
            "TransactionData" => Llsd::Array(vec![llsd_map! { "TransactionID" => Uuid::from_u128(9) }]),
            "QueryData" => Llsd::Array(vec![
                llsd_map! { "OwnerID" => Uuid::nil(), "ActualArea" => 5120, "BillableArea" => 0 },
                llsd_map! {
                    "OwnerID" => Uuid::from_u128(7),
                    "Name" => "Bac à sable",
                    "ActualArea" => 4096,
                    "BillableArea" => 4096,
                    "GlobalX" => 256_064.0,
                    "GlobalY" => 256_200.0,
                    "SimName" => "Aurora Démo",
                    "ProductSKU" => "023",
                },
            ]),
        };
        let (to, r) = places_reply_llsd(&b);
        assert_eq!((to, r.group, r.transaction), (agent, Uuid::from_u128(7), Uuid::from_u128(9)));
        assert_eq!(r.totals.map(|t| (t.in_use, t.contributed)), Some((4096, 5120)));
        assert_eq!(r.parcels[0].sku, "023");
        assert_eq!(r.parcels[0].global_x, 256_064.0);
    }

    #[test]
    fn the_planning_page_always_asks_the_current_week() {
        let r = AccountRequest {
            ledger: Ledger::Planning,
            group: Uuid::from_u128(7),
            request: Uuid::from_u128(3),
            interval: 4,
        };
        let AccountMessage::Summary(m) = account_message(Uuid::from_u128(1), Uuid::from_u128(2), &r) else {
            panic!("a summary request");
        };
        assert_eq!(
            (m.agent_data.group_id, m.money_data.request_id),
            (Uuid::from_u128(7), Uuid::from_u128(3))
        );
        assert_eq!((m.money_data.interval_days, m.money_data.current_interval), (7, 0));
        let r = AccountRequest {
            ledger: Ledger::Sales,
            ..r
        };
        let AccountMessage::Sales(m) = account_message(Uuid::from_u128(1), Uuid::from_u128(2), &r) else {
            panic!("a transactions request");
        };
        assert_eq!(m.money_data.current_interval, 4);
    }

    #[test]
    fn dates_read_year_month_day() {
        assert_eq!(parse_date("2026-10-06"), Some((2026, 10, 6)));
        assert_eq!(parse_date(" 2026-10-06 "), Some((2026, 10, 6)));
        assert_eq!(parse_date("2026-13-06"), None);
        assert_eq!(parse_date(""), None);
    }
}
