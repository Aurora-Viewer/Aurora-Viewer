//! A group's land and L$ while its profile shows them, as
//! LLPanelGroupLandMoney keeps them (indra/newview/llpanelgrouplandmoney.cpp,
//! originally LGPL 2.1): the parcels of the latest PlacesQuery (its
//! transaction id; the totals are taken from any reply, as processGroupLand
//! does), and the three pages of the accounts, each with its own request id
//! (the handler's mPanelID) and week. A reply for another week than the one
//! shown is "out of date" and dropped; the sales can come in several
//! packets, appended.

use aurora_net::NetCommand;
use aurora_net::group_money::{
    AccountBody, AccountReply, AccountRequest, GroupLandReply, GroupParcel, INTERVAL_DAYS, LandTotals, Ledger, MAX_INTERVAL,
};
use std::collections::HashMap;
use uuid::Uuid;

/// One page of the accounts.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    request: Uuid,
    /// Weeks back (mCurrentInterval): 0 is this week.
    pub interval: i32,
    /// StartDate of the reply, "YYYY-MM-DD".
    pub start_date: String,
    /// None while loading ("Loading...").
    pub body: Option<AccountBody>,
}

impl Page {
    /// getCanClickEarlier.
    pub fn can_go_earlier(&self) -> bool {
        self.interval < MAX_INTERVAL
    }

    /// getCanClickLater.
    pub fn can_go_later(&self) -> bool {
        self.interval > 0
    }
}

/// What the tab shows of one group.
#[derive(Debug, Clone, Default)]
pub struct GroupMoneyData {
    transaction: Uuid,
    /// The parcels of the latest query; None until it answered.
    pub parcels: Option<Vec<GroupParcel>>,
    pub totals: Option<LandTotals>,
    pages: HashMap<Ledger, Page>,
}

impl GroupMoneyData {
    pub fn page(&self, ledger: Ledger) -> Option<&Page> {
        self.pages.get(&ledger)
    }
}

#[derive(Default)]
pub struct GroupMoney {
    groups: HashMap<Uuid, GroupMoneyData>,
    out: Vec<NetCommand>,
}

impl GroupMoney {
    pub fn get(&self, group: &Uuid) -> Option<&GroupMoneyData> {
        self.groups.get(group)
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }

    /// The tab opened, or "Refresh" (LLPanelGroupLandMoney::update(GC_ALL)):
    /// the parcels, then the page shown.
    pub fn open(&mut self, group: Uuid, ledger: Ledger) {
        if group.is_nil() {
            return;
        }
        self.request_land(group);
        self.request(group, ledger);
    }

    /// LLGroupActions::refresh: what was asked is asked again.
    pub fn refresh(&mut self, group: Uuid) {
        let Some(g) = self.groups.get(&group) else {
            return;
        };
        let ledgers: Vec<Ledger> = g.pages.keys().copied().collect();
        self.request_land(group);
        for l in ledgers {
            self.request(group, l);
        }
    }

    /// Forget a group (after leaving it).
    pub fn clear(&mut self, group: &Uuid) {
        self.groups.remove(group);
    }

    /// LLFloaterLandHoldings::onOpen: the null query id selects our parcels.
    pub fn request_owned_land(&mut self) {
        self.request_land(Uuid::nil());
    }

    /// requestGroupLandInfo: a new transaction, the list emptied.
    fn request_land(&mut self, group: Uuid) {
        // LLFloaterLandHoldings::onOpen uses null query and transaction
        // UUIDs; group panels correlate their replies with a fresh UUID.
        let transaction = if group.is_nil() { Uuid::nil() } else { Uuid::new_v4() };
        let g = self.groups.entry(group).or_default();
        g.transaction = transaction;
        g.parcels = None;
        self.out.push(NetCommand::GroupLandQuery { group, transaction });
    }

    /// A page shown (onClickTab → requestData): its week asked, "Loading..."
    /// meanwhile.
    pub fn request(&mut self, group: Uuid, ledger: Ledger) {
        let g = self.groups.entry(group).or_default();
        let page = g.pages.entry(ledger).or_insert_with(|| Page {
            request: Uuid::new_v4(),
            interval: 0,
            start_date: String::new(),
            body: None,
        });
        page.body = None;
        self.out.push(NetCommand::GroupAccount(AccountRequest {
            ledger,
            group,
            request: page.request,
            interval: page.interval,
        }));
    }

    /// onClickEarlier / onClickLater: one week back or forward, within the
    /// eight weeks the server keeps.
    pub fn step(&mut self, group: Uuid, ledger: Ledger, earlier: bool) {
        if ledger == Ledger::Planning {
            return;
        }
        let Some(page) = self.groups.get_mut(&group).and_then(|g| g.pages.get_mut(&ledger)) else {
            return;
        };
        if earlier && page.can_go_earlier() {
            page.interval += 1;
        } else if !earlier && page.can_go_later() {
            page.interval -= 1;
        } else {
            return;
        }
        self.request(group, ledger);
    }

    /// processGroupLand: the totals of any reply; the parcels only for the
    /// latest query.
    pub fn on_land(&mut self, reply: &GroupLandReply) {
        let Some(g) = self.groups.get_mut(&reply.group) else {
            log::info!("group land: no panel for {}", reply.group);
            return;
        };
        if let Some(t) = reply.totals {
            g.totals = Some(t);
        }
        if reply.transaction != g.transaction {
            return;
        }
        g.parcels.get_or_insert_with(Vec::new).extend(reply.parcels.iter().cloned());
    }

    /// processReply of each handler: found by its request id, for this
    /// group and the week shown.
    pub fn on_account(&mut self, reply: &AccountReply) {
        let Some((ledger, page)) = self
            .groups
            .get_mut(&reply.group)
            .and_then(|g| g.pages.iter_mut().find(|(_, p)| p.request == reply.request))
        else {
            log::warn!("group L$ reply for a page not shown");
            return;
        };
        let interval = if *ledger == Ledger::Planning { 0 } else { page.interval };
        if reply.interval_days != INTERVAL_DAYS || reply.interval != interval {
            log::info!("group L$: out of date packet {} {}", reply.interval_days, reply.interval);
            return;
        }
        page.start_date.clone_from(&reply.start_date);
        match (&mut page.body, &reply.body) {
            // the sales of one week can take several packets
            (Some(AccountBody::Sales(list)), AccountBody::Sales(more)) => list.extend(more.iter().cloned()),
            (body, new) => *body = Some(new.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::group_money::{AccountSummary, Transaction};

    const G: Uuid = Uuid::from_u128(7);

    fn sent(m: &mut GroupMoney) -> Vec<NetCommand> {
        m.take_commands()
    }

    fn parcel(name: &str) -> GroupParcel {
        GroupParcel {
            name: name.into(),
            desc: String::new(),
            actual_area: 512,
            billable_area: 512,
            global_x: 0.0,
            global_y: 0.0,
            global_z: 0.0,
            sim_name: "Aurora Démo".into(),
            sku: String::new(),
        }
    }

    fn request_of(cmds: &[NetCommand], ledger: Ledger) -> AccountRequest {
        cmds.iter()
            .find_map(|c| match c {
                NetCommand::GroupAccount(r) if r.ledger == ledger => Some(*r),
                _ => None,
            })
            .expect("request sent")
    }

    #[test]
    fn opening_asks_the_parcels_and_the_page_and_keeps_only_the_latest_query() {
        let mut m = GroupMoney::default();
        m.open(G, Ledger::Planning);
        let cmds = sent(&mut m);
        let NetCommand::GroupLandQuery { transaction: first, .. } = cmds[0] else {
            panic!("the land first");
        };
        request_of(&cmds, Ledger::Planning);
        m.open(G, Ledger::Planning);
        let NetCommand::GroupLandQuery { transaction: second, .. } = sent(&mut m)[0] else {
            panic!("the land again");
        };
        let totals = LandTotals {
            in_use: 4608,
            contributed: 5120,
        };
        // the older query's totals are kept, not its parcels
        m.on_land(&GroupLandReply {
            group: G,
            transaction: first,
            totals: Some(totals),
            parcels: vec![parcel("old")],
        });
        assert_eq!(m.get(&G).and_then(|g| g.totals), Some(totals));
        assert_eq!(m.get(&G).and_then(|g| g.parcels.clone()), None);
        m.on_land(&GroupLandReply {
            group: G,
            transaction: second,
            totals: None,
            parcels: vec![parcel("Sandbox")],
        });
        m.on_land(&GroupLandReply {
            group: G,
            transaction: second,
            totals: None,
            parcels: vec![parcel("Workshop")],
        });
        let names: Vec<String> = m
            .get(&G)
            .and_then(|g| g.parcels.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Sandbox", "Workshop"]);
    }

    #[test]
    fn personal_holdings_use_null_query_ids_and_stay_apart_from_groups() {
        let mut money = GroupMoney::default();
        money.request_owned_land();
        let cmds = sent(&mut money);
        assert!(matches!(cmds.as_slice(), [NetCommand::GroupLandQuery { group, transaction }] if group.is_nil() && transaction.is_nil()));
        money.on_land(&GroupLandReply {
            group: Uuid::nil(),
            transaction: Uuid::nil(),
            totals: None,
            parcels: vec![parcel("Home")],
        });
        assert_eq!(money.get(&Uuid::nil()).unwrap().parcels.as_ref().unwrap()[0].name, "Home");
        assert!(money.get(&G).is_none());
        money.request_owned_land();
        assert!(money.get(&Uuid::nil()).unwrap().parcels.is_none());
    }

    #[test]
    fn weeks_go_back_eight_at_most_and_older_weeks_are_dropped() {
        let mut m = GroupMoney::default();
        m.request(G, Ledger::Sales);
        let r = request_of(&sent(&mut m), Ledger::Sales);
        assert_eq!(r.interval, 0);
        m.step(G, Ledger::Sales, false);
        assert!(sent(&mut m).is_empty(), "no later week than this one");
        for _ in 0..10 {
            m.step(G, Ledger::Sales, true);
        }
        let page = m.get(&G).and_then(|g| g.page(Ledger::Sales)).expect("page");
        assert_eq!(page.interval, MAX_INTERVAL);
        assert!(!page.can_go_earlier() && page.can_go_later());
        assert_eq!(sent(&mut m).len(), MAX_INTERVAL as usize);
        let sale = |item: &str| Transaction {
            time: "2026-10-06".into(),
            user: "Nova".into(),
            kind: aurora_net::group_money::trans::OBJECT_SALE,
            item: item.into(),
            amount: 25,
        };
        let reply = |interval: i32, item: &str| AccountReply {
            group: G,
            request: r.request,
            interval_days: 7,
            interval,
            start_date: "2026-08-18".into(),
            body: AccountBody::Sales(vec![sale(item)]),
        };
        // a late answer for this week is out of date
        m.on_account(&reply(0, "late"));
        assert_eq!(m.get(&G).and_then(|g| g.page(Ledger::Sales)).and_then(|p| p.body.clone()), None);
        m.on_account(&reply(MAX_INTERVAL, "chair"));
        m.on_account(&reply(MAX_INTERVAL, "lamp"));
        let Some(AccountBody::Sales(list)) = m.get(&G).and_then(|g| g.page(Ledger::Sales)).and_then(|p| p.body.clone()) else {
            panic!("sales shown");
        };
        assert_eq!(list.iter().map(|s| s.item.as_str()).collect::<Vec<_>>(), ["chair", "lamp"]);
    }

    #[test]
    fn the_planning_page_stays_on_this_week() {
        let mut m = GroupMoney::default();
        m.request(G, Ledger::Planning);
        let r = request_of(&sent(&mut m), Ledger::Planning);
        m.step(G, Ledger::Planning, true);
        assert!(sent(&mut m).is_empty());
        m.on_account(&AccountReply {
            group: G,
            request: r.request,
            interval_days: 7,
            interval: 0,
            start_date: "2026-10-06".into(),
            body: AccountBody::Summary(AccountSummary {
                balance: 1250,
                ..Default::default()
            }),
        });
        let page = m.get(&G).and_then(|g| g.page(Ledger::Planning)).expect("page");
        assert_eq!(page.start_date, "2026-10-06");
        assert!(matches!(
            page.body,
            Some(AccountBody::Summary(AccountSummary { balance: 1250, .. }))
        ));
        // another page's request id finds nothing
        m.on_account(&AccountReply {
            request: Uuid::from_u128(99),
            ..AccountReply {
                group: G,
                request: r.request,
                interval_days: 7,
                interval: 0,
                start_date: String::new(),
                body: AccountBody::Details(Vec::new()),
            }
        });
        assert!(matches!(
            m.get(&G).and_then(|g| g.page(Ledger::Planning)).and_then(|p| p.body.clone()),
            Some(AccountBody::Summary(_))
        ));
    }
}
