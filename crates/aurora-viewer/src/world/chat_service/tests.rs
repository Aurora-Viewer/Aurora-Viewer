//! Tests of the history manager, on a fake clock (a base instant plus a
//! duration), a fake World (mutes and names), a fake storage that answers
//! every storage action at once and a fake Chat Service that pages a
//! per-resident history.

use super::manager::{Discovery, MetaState, StateSafety};
use super::*;
use crate::world::chat_log::service::csv::ArchiveState;
use aurora_net::chat_service::{MAX_PASS_BYTES, MAX_PASS_ROWS, TimeUuidKey, direct_conversation_id, format_time_uuid, parse_time_uuid};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::Instant;

const AGENT: Uuid = Uuid::from_u128(0xa9e7);
const LIST_URL: &str = "https://region.invalid/cap/list";
const HISTORY_URL: &str = "https://region.invalid/cap/history";
const CREATED_AT: &str = "2026-08-12T12:34:56Z";

fn res(n: u128) -> Uuid {
    Uuid::from_u128(0x1000 + n)
}

fn ctx(region: u128) -> Context {
    Context {
        region_id: Uuid::from_u128(0xbeef + region),
        list_url: format!("{LIST_URL}/{region}"),
        history_url: format!("{HISTORY_URL}/{region}"),
    }
}

/// Message `n`, `n` seconds after 1970.
fn key(n: u64) -> TimeUuidKey {
    TimeUuidKey {
        ticks: UUID_EPOCH + n * 10_000_000,
        tail: [i8::MIN, 0, 0, 0, 0, 0, 0, 0],
    }
}

fn msg(n: u64) -> String {
    format_time_uuid(&key(n))
}

fn avatar(first: &str, last: &str) -> AvatarName {
    AvatarName {
        username: format!("{first}.{last}").to_lowercase(),
        display_name: format!("{first} {last}"),
        legacy_first: first.to_owned(),
        legacy_last: last.to_owned(),
        ..AvatarName::default()
    }
}

#[derive(Default)]
struct FakeEnv {
    muted: HashSet<Uuid>,
    muted_names: HashSet<String>,
    names: HashMap<Uuid, AvatarName>,
}

impl Env for FakeEnv {
    fn is_muted(&self, id: Uuid) -> bool {
        self.muted.contains(&id)
    }
    fn is_muted_name(&self, id: Uuid, user_name: &str) -> bool {
        self.muted.contains(&id) || self.muted_names.contains(user_name)
    }
    fn cached_name(&self, id: Uuid) -> Option<AvatarName> {
        self.names.get(&id).cloned()
    }
}

fn row(resident: Uuid, n: u64) -> HistoryRow {
    HistoryRow {
        conversation_id: direct_conversation_id(AGENT, resident),
        msg_id: msg(n),
        key: key(n),
        from_id: resident,
        from_name: "Some One".into(),
        message: format!("hello {n}"),
        dialog: 0,
        created_at: CREATED_AT.into(),
    }
}

struct Store {
    state: StateLoad,
    /// Oldest first.
    archives: BTreeMap<Uuid, Vec<HistoryRow>>,
    sweep_ok: bool,
    published: Vec<(Uuid, Vec<HistoryRow>, bool)>,
}

impl Store {
    fn summary(&self, resident: Uuid, cutoff: u64) -> ArchiveScan {
        let Some(rows) = self.archives.get(&resident) else {
            return ArchiveScan::default();
        };
        let kept: Vec<_> = rows.iter().filter(|r| r.key.ticks > cutoff).collect();
        ArchiveScan {
            state: ArchiveState::Valid,
            row_count: kept.len() as u32,
            oldest: kept.first().map(|r| r.key),
            newest: kept.last().map(|r| r.key),
            ..ArchiveScan::default()
        }
    }
}

#[derive(Debug, Clone)]
struct Req {
    id: u64,
    url: String,
    body: Option<Llsd>,
    at: Duration,
}

impl Req {
    fn is_list(&self) -> bool {
        self.body.is_none()
    }
    fn cursor(&self) -> String {
        self.body
            .as_ref()
            .map(|b| b.get("before_msg_id").as_str().to_owned())
            .unwrap_or_default()
    }
    fn conversation(&self) -> String {
        self.body
            .as_ref()
            .map(|b| b.get("conversation_id").as_str().to_owned())
            .unwrap_or_default()
    }
}

struct H {
    m: Manager,
    env: FakeEnv,
    base: Instant,
    t: Duration,
    store: Store,
    /// The fake service: each resident's message numbers.
    server: BTreeMap<Uuid, Vec<u64>>,
    big_rows: bool,
    listed: Vec<Uuid>,
    http: VecDeque<Req>,
    log: Vec<Action>,
}

impl H {
    fn new() -> H {
        let mut h = H {
            m: Manager::new(),
            env: FakeEnv::default(),
            base: Instant::now(),
            t: Duration::ZERO,
            store: Store {
                state: StateLoad::Safe {
                    deleted_before_ticks: 0,
                    cleanup_pending: false,
                },
                archives: BTreeMap::new(),
                sweep_ok: true,
                published: Vec::new(),
            },
            server: BTreeMap::new(),
            big_rows: false,
            listed: Vec::new(),
            http: VecDeque::new(),
            log: Vec::new(),
        };
        h.m.set_gates(Gates {
            consent: true,
            mute_list_server_loaded: true,
        });
        h
    }

    /// A named, listed resident with `messages` on the service.
    fn resident(&mut self, n: u128, messages: impl IntoIterator<Item = u64>) -> Uuid {
        let id = res(n);
        self.env.names.insert(id, avatar("Resident", &format!("N{n}")));
        self.server.insert(id, messages.into_iter().collect());
        self.listed.push(id);
        id
    }

    fn now(&self) -> Instant {
        self.base + self.t
    }

    fn start(&mut self, rollout: bool) {
        self.m.start(AGENT, rollout);
        self.m.set_context(Some(ctx(0)));
        self.pump();
    }

    fn pump(&mut self) {
        for _ in 0..100_000 {
            let actions = self.m.poll(self.now(), &self.env);
            if actions.is_empty() {
                return;
            }
            for a in actions {
                self.handle(a);
            }
        }
        panic!("the manager does not settle");
    }

    /// The fake World: storage answers at once, requests are kept.
    fn handle(&mut self, a: Action) {
        let cutoff = self.m.rt.deleted_before_ticks;
        match &a {
            Action::Http { id, url, body } => self.http.push_back(Req {
                id: *id,
                url: url.clone(),
                body: body.clone(),
                at: self.t,
            }),
            Action::LoadState { ticket } => self.m.state_loaded(*ticket, self.store.state),
            Action::ListArchives { ticket } => {
                let ids = self.store.archives.keys().copied().collect();
                self.m.archives_listed(*ticket, ids, false);
            }
            Action::PrepareArchive { ticket, resident, .. } => {
                let summary = self.store.summary(*resident, cutoff);
                self.m.archive_prepared(
                    *ticket,
                    Some(PreparedArchive {
                        summary,
                        bytes_changed: false,
                    }),
                );
            }
            Action::PublishRows {
                ticket,
                resident,
                rows,
                append,
                ..
            } => {
                self.store.published.push((*resident, rows.clone(), *append));
                let archive = self.store.archives.entry(*resident).or_default();
                if !append {
                    archive.clear();
                }
                archive.extend(rows.iter().cloned());
                let summary = self.store.summary(*resident, cutoff);
                self.m.rows_published(*ticket, Some(summary));
            }
            Action::UpdateIndex { ticket, .. } => self.m.index_updated(*ticket, true),
            Action::BeginDeleteState { ticket, click_ticks } => {
                let previous = match self.store.state {
                    StateLoad::Safe { deleted_before_ticks, .. } => deleted_before_ticks,
                    StateLoad::Unsafe => 0,
                };
                let boundary = (*click_ticks).max(previous);
                self.store.state = StateLoad::Safe {
                    deleted_before_ticks: boundary,
                    cleanup_pending: true,
                };
                self.m.delete_state_written(*ticket, Some(boundary));
            }
            Action::SweepTranscripts { ticket } => {
                if self.store.sweep_ok {
                    self.store.archives.clear();
                }
                self.m.transcripts_swept(*ticket, self.store.sweep_ok);
            }
            Action::WriteState {
                ticket,
                boundary,
                cleanup_pending,
            } => {
                self.store.state = StateLoad::Safe {
                    deleted_before_ticks: *boundary,
                    cleanup_pending: *cleanup_pending,
                };
                self.m.state_written(*ticket, true);
            }
            _ => {}
        }
        self.log.push(a);
    }

    /// Run the clock to the next request, at most `limit` ahead.
    fn next_request_within(&mut self, limit: Duration) -> Option<Req> {
        let end = self.t + limit;
        for _ in 0..100_000 {
            self.pump();
            if let Some(r) = self.http.pop_front() {
                return Some(r);
            }
            let wake = self.m.next_wake(self.now())?;
            let at = wake.saturating_duration_since(self.base);
            if at > end {
                return None;
            }
            assert!(at > self.t, "a wake now that does nothing");
            self.t = at;
        }
        panic!("no request and no end");
    }

    fn next_request(&mut self) -> Req {
        self.next_request_within(Duration::from_secs(100_000)).expect("a request")
    }

    fn advance(&mut self, secs: f64) {
        self.t += Duration::from_secs_f64(secs);
        self.pump();
    }

    fn list_body(&self) -> Llsd {
        let mut list = Llsd::new_array();
        for id in &self.listed {
            let last = self.server.get(id).and_then(|m| m.iter().max()).copied().unwrap_or(1);
            let mut entry = Llsd::new_map();
            entry.insert("conversation_type", "direct");
            entry.insert("conversation_id", direct_conversation_id(AGENT, *id));
            entry.insert("other_participant_id", id.to_string());
            entry.insert("last_msg_id", msg(last));
            list.push(entry);
        }
        list
    }

    fn resident_of(&self, req: &Req) -> Uuid {
        let conversation = req.conversation();
        *self
            .server
            .keys()
            .find(|id| direct_conversation_id(AGENT, **id) == conversation)
            .expect("a known conversation")
    }

    fn page_body(&self, req: &Req) -> Llsd {
        let resident = self.resident_of(req);
        let cursor = req.cursor();
        let below = (!cursor.is_empty()).then(|| parse_time_uuid(&cursor).expect("a cursor"));
        let mut numbers: Vec<u64> = self.server[&resident]
            .iter()
            .copied()
            .filter(|n| below.is_none_or(|b| key(*n) < b))
            .collect();
        numbers.sort_unstable_by(|a, b| b.cmp(a));
        numbers.truncate(100);
        let mut messages = Llsd::new_array();
        for n in numbers {
            let mut m = Llsd::new_map();
            m.insert("conversation_id", direct_conversation_id(AGENT, resident));
            m.insert("msg_id", msg(n));
            m.insert("from_id", resident.to_string());
            if self.big_rows {
                m.insert("from_name", "n".repeat(256));
                m.insert("message", "m".repeat(1024));
            } else {
                m.insert("from_name", "Some One");
                m.insert("message", format!("hello {n}"));
            }
            m.insert("dialog", 0);
            m.insert("created_at", CREATED_AT);
            messages.push(m);
        }
        let mut page = Llsd::new_map();
        page.insert("conversation_id", direct_conversation_id(AGENT, resident));
        page.insert("limit", 100);
        page.insert("messages", messages);
        if !cursor.is_empty() {
            page.insert("before_msg_id", cursor);
        }
        page
    }

    fn answer(&mut self, req: &Req, status: u16, body: Option<Llsd>) {
        self.m.on_response(req.id, status, body);
        self.pump();
    }

    fn answer_ok(&mut self, req: &Req) {
        let body = if req.is_list() { self.list_body() } else { self.page_body(req) };
        self.answer(req, 200, Some(body));
    }

    /// Login, and the first list answered.
    fn login(&mut self) {
        self.start(true);
        let list = self.next_request();
        assert!(list.is_list(), "the list comes first");
        self.answer_ok(&list);
    }

    /// Answer every request until none comes within `limit` seconds.
    fn serve_all(&mut self, limit: f64) -> Vec<Req> {
        let mut seen = Vec::new();
        while let Some(req) = self.next_request_within(Duration::from_secs_f64(limit)) {
            self.answer_ok(&req);
            seen.push(req);
        }
        seen
    }

    fn published(&self) -> &[(Uuid, Vec<HistoryRow>, bool)] {
        &self.store.published
    }

    fn count(&self, f: impl Fn(&Action) -> bool) -> usize {
        self.log.iter().filter(|a| f(a)).count()
    }
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

#[test]
fn login_lists_then_queues_every_conversation_in_list_order() {
    let mut h = H::new();
    let (a, b, c) = (h.resident(3, []), h.resident(1, []), h.resident(2, []));
    h.start(true);
    assert!(h.log.iter().any(|x| matches!(x, Action::LoadState { .. })), "the state first");
    assert!(h.log.iter().any(|x| matches!(x, Action::ListArchives { .. })), "then the archives");
    let list = h.next_request();
    assert!(list.is_list() && list.url == ctx(0).list_url, "a GET of the list capability");
    h.answer_ok(&list);
    assert_eq!(h.m.rt.active_resident, Some(a), "the first listed resident's pass is pacing");
    assert_eq!(h.m.rt.queue, [b, c], "the others queued in list order, not id order");
    let order: Vec<String> = h.serve_all(10.0).iter().map(Req::conversation).collect();
    let expected: Vec<String> = [a, b, c].iter().map(|r| direct_conversation_id(AGENT, *r)).collect();
    assert_eq!(order, expected);
    for r in &h.listed {
        let r = &h.m.rt.residents[r];
        assert_eq!(r.covered_token, r.advertised_token, "covered");
    }
}

/// The grid listed a direct conversation with the account itself (#4499):
/// it is left out, and every other conversation is still fetched.
#[test]
fn a_conversation_with_ourselves_in_the_list_is_left_out() {
    let mut h = H::new();
    let (a, b) = (h.resident(1, [1, 2, 3]), h.resident(2, [4, 5]));
    h.start(true);
    let list = h.next_request();
    assert!(list.is_list(), "the list comes first");
    let Llsd::Array(mut entries) = h.list_body() else {
        panic!("the list is an array");
    };
    let mut own = Llsd::new_map();
    own.insert("conversation_type", "direct");
    own.insert("conversation_id", direct_conversation_id(AGENT, AGENT));
    own.insert("other_participant_id", AGENT.to_string());
    own.insert("last_msg_id", msg(9));
    entries.insert(1, own);
    h.answer(&list, 200, Some(Llsd::Array(entries)));
    assert_eq!(h.m.rt.active_resident, Some(a), "the first resident's pass is pacing");
    assert_eq!(h.m.rt.queue, [b], "the second queued, not ourselves");

    while let Some(req) = h.next_request_within(Duration::from_secs(10)) {
        assert_ne!(
            req.conversation(),
            direct_conversation_id(AGENT, AGENT),
            "no history request for ourselves"
        );
        h.answer_ok(&req);
    }
    let published: HashSet<Uuid> = h.published().iter().map(|(id, ..)| *id).collect();
    assert_eq!(published, HashSet::from([a, b]), "both residents' histories fetched");
    for id in [a, b] {
        let r = &h.m.rt.residents[&id];
        assert_eq!(r.covered_token, r.advertised_token, "covered");
    }
    assert!(!h.m.rt.residents.contains_key(&AGENT), "the agent never becomes a resident");
}

#[test]
fn requests_are_spaced_from_their_start() {
    let mut h = H::new();
    for n in 0..4 {
        h.resident(n, [1, 2, 3]);
    }
    h.start(true);
    let mut starts = Vec::new();
    // The answers come at once: each request waits 2.1 s from the start of
    // the previous one, list and history alike.
    while let Some(req) = h.next_request_within(Duration::from_secs(10)) {
        starts.push(req.at);
        h.answer_ok(&req);
    }
    assert_eq!(starts.len(), 1 + 4 * 2, "a list, then a head and an empty page each");
    for pair in starts.windows(2) {
        assert_eq!(pair[1] - pair[0], Duration::from_millis(2100));
    }

    // A slow answer: the next request goes as soon as it comes.
    let mut h = H::new();
    h.resident(0, [1]);
    h.start(true);
    let list = h.next_request();
    h.advance(7.0);
    h.answer_ok(&list);
    let head = h.next_request();
    assert_eq!(secs(head.at), 7.0);
}

#[test]
fn list_retry_rules() {
    // (status, delay of the next list after the answer)
    let cases: &[(u16, f64)] = &[
        (429, 120.0),
        (0, 5.0),
        (408, 5.0),
        (500, 5.0),
        (503, 5.0),
        (499, 3600.0),
        (404, 3600.0),
        (403, 3600.0),
        (200, 3600.0),
    ];
    for &(status, delay) in cases {
        let mut h = H::new();
        h.start(true);
        let list = h.next_request();
        h.advance(1.0);
        // A 2xx whose body is not a list is not retried either.
        let body = (status == 200).then(Llsd::new_map);
        h.answer(&list, status, body);
        let next = h.next_request();
        assert!(next.is_list(), "status {status}");
        assert_eq!(secs(next.at) - 1.0, delay, "status {status}");

        // While the hourly list is due, every turn resets the retry latch
        // (CS:2038-2042): a retryable failure is retried again and again,
        // as in the LL viewer, until a list goes through.
        h.answer(&next, status, None);
        let third = h.next_request();
        let again = if status == 429 {
            120.0
        } else if delay == 5.0 {
            5.0
        } else {
            3600.0
        };
        assert_eq!(secs(third.at - next.at), again, "second {status}");
    }

    // A relist between two hourly lists (here a region change) has one
    // retry, then waits for the next hour.
    let mut h = H::new();
    h.login();
    h.advance(100.0);
    h.m.set_context(Some(ctx(1)));
    let relist = h.next_request();
    assert!(relist.is_list() && relist.url == ctx(1).list_url);
    h.answer(&relist, 0, None);
    let retry = h.next_request();
    assert_eq!(secs(retry.at - relist.at), 5.0);
    h.answer(&retry, 503, None);
    let next = h.next_request();
    assert_eq!(secs(next.at - retry.at), 3600.0, "one retry only");

    // A valid list resets the retry, and the hour counts from its answer.
    let mut h = H::new();
    h.start(true);
    let list = h.next_request();
    h.answer(&list, 0, None);
    let retry = h.next_request();
    h.answer_ok(&retry);
    let hourly = h.next_request();
    assert_eq!(secs(hourly.at - retry.at), 3600.0);
    h.answer(&hourly, 502, None);
    let retry = h.next_request();
    assert_eq!(secs(retry.at - hourly.at), 5.0);
}

#[test]
fn history_page_retry_and_requeue_rules() {
    // 429: requeued behind the cooldown, with no limit.
    let mut h = H::new();
    let r = h.resident(0, 1..=150);
    h.login();
    let mut head = h.next_request();
    for _ in 0..3 {
        assert!(head.cursor().is_empty());
        h.answer(&head, 429, None);
        let again = h.next_request();
        assert_eq!(secs(again.at - head.at), 120.0);
        assert!(again.cursor().is_empty(), "the pass starts again from the head");
        assert!(h.m.work_active(r), "still queued: still working");
        head = again;
    }

    // A retryable failure: one retry after 5 s, from the head; the second
    // failure abandons the pass until a fresh trigger.
    let mut h = H::new();
    h.resident(0, 1..=150);
    h.login();
    let head = h.next_request();
    h.answer_ok(&head);
    let second = h.next_request();
    assert!(!second.cursor().is_empty(), "a continuation page");
    h.answer(&second, 500, None);
    let retry = h.next_request();
    assert_eq!(secs(retry.at - second.at), 5.0);
    assert!(retry.cursor().is_empty(), "a retry starts again from the head");
    h.answer(&retry, 0, None);
    assert!(!h.m.work_active(r));
    let next = h.next_request();
    assert!(next.is_list(), "abandoned: the next request is the hourly list");
    assert!(h.published().is_empty(), "nothing written");

    // Not retried: 499, 404, and a 2xx page that does not validate.
    for (status, body) in [(499, None), (404, None), (200, Some(Llsd::new_map()))] {
        let mut h = H::new();
        h.resident(0, 1..=5);
        h.login();
        let head = h.next_request();
        h.answer(&head, status, body);
        assert!(!h.m.work_active(r), "status {status}: no work shown");
        assert!(h.next_request().is_list(), "status {status}: abandoned");
    }
}

#[test]
fn region_change_relists_and_drops_the_response_in_flight() {
    let mut h = H::new();
    let r = h.resident(0, 1..=5);
    h.login();
    let head = h.next_request();
    assert_eq!(head.url, ctx(0).history_url);
    h.m.set_context(Some(ctx(1)));
    h.pump();
    // An answer with another id is not the one awaited.
    h.m.on_response(head.id + 1000, 200, Some(h.page_body(&head)));
    h.pump();
    assert!(h.http.is_empty());
    h.answer_ok(&head);
    assert!(h.published().is_empty(), "the answer from the old region is dropped");
    let list = h.next_request();
    assert!(list.is_list() && list.url == ctx(1).list_url, "the new region relists");
    h.answer_ok(&list);
    let head = h.next_request();
    assert_eq!(head.url, ctx(1).history_url);
    assert_eq!(h.resident_of(&head), r, "queued again by the new list: its pass was not complete");

    // Losing the capabilities while pacing keeps the resident queued.
    let mut h = H::new();
    let r = h.resident(0, 1..=150);
    h.login();
    let head = h.next_request();
    h.answer_ok(&head);
    h.m.set_context(None);
    h.advance(3.0);
    assert!(h.http.is_empty());
    assert_eq!(h.m.rt.queue, [r], "kept for the next context");
    assert!(!h.m.work_active(r));
    h.m.set_context(Some(ctx(0)));
    let list = h.next_request();
    assert!(list.is_list(), "a context change relists, even back to the same region");
}

#[test]
fn priority_preempts_the_active_pass() {
    let mut h = H::new();
    let (a, b, c) = (h.resident(0, 1..=250), h.resident(1, [1]), h.resident(2, [1]));
    h.login();
    let head = h.next_request();
    assert_eq!(h.resident_of(&head), a);
    h.answer_ok(&head);
    assert!(!h.m.head_preview(a).is_empty(), "the head page is shown before the archive");
    h.m.prioritize(c, false, &h.env);
    assert_eq!(h.m.rt.queue, [c, b]);
    let next = h.next_request();
    assert_eq!(h.resident_of(&next), c, "the priority goes first");
    assert_eq!(h.m.rt.queue, [b, a], "the preempted pass goes to the back");
    assert!(h.m.head_preview(a).is_empty(), "its preview went with the pass");
    assert!(h.m.work_active(a), "still queued: still working");
    h.answer_ok(&next);
    let rest: Vec<(Uuid, bool)> = h
        .serve_all(10.0)
        .iter()
        .map(|q| (h.resident_of(q), q.cursor().is_empty()))
        .collect();
    assert_eq!(rest[0], (c, false), "the priority's own empty page");
    assert_eq!(rest[1], (b, true));
    assert_eq!(rest[3], (a, true), "the preempted pass starts again from the head");
    assert_eq!(h.published().iter().find(|p| p.0 == a).map(|p| p.1.len()), Some(250));

    // A priority on the active resident once its first request started
    // records one follow-up pass, at the front.
    let mut h = H::new();
    let (a, b) = (h.resident(0, 1..=150), h.resident(1, [1]));
    h.login();
    let head = h.next_request();
    h.m.prioritize(a, true, &h.env);
    assert!(h.m.rt.residents[&a].forced_followup);
    h.answer_ok(&head);
    let mut next = h.next_request();
    while !next.cursor().is_empty() {
        h.answer_ok(&next);
        next = h.next_request();
    }
    assert_eq!(h.resident_of(&next), a, "the follow-up before b");
    assert_eq!(h.m.rt.queue, [b]);
}

#[test]
fn activity_refreshes_after_fifteen_quiet_seconds() {
    let mut h = H::new();
    let r = h.resident(0, [1]);
    h.login();
    h.serve_all(10.0);
    let t0 = h.t;
    h.m.note_activity(r, h.now(), &h.env);
    h.advance(10.0);
    h.m.note_activity(r, h.now(), &h.env);
    h.server.get_mut(&r).expect("a resident").push(2);
    assert!(
        h.next_request_within(Duration::from_secs(14)).is_none(),
        "each IM resets the quiet deadline"
    );
    let head = h.next_request();
    assert_eq!(secs(head.at - t0), 25.0);
    assert!(head.cursor().is_empty() && h.resident_of(&head) == r);
    h.answer_ok(&head);
    assert!(
        h.serve_all(30.0).is_empty(),
        "the archive's newest key ends the pass on its head page"
    );
    assert_eq!(h.published().last().map(|p| (p.1.len(), p.2)), Some((1, true)));

    // A muted resident's activity schedules nothing.
    h.env.muted.insert(r);
    h.m.note_activity(r, h.now(), &h.env);
    assert!(h.next_request_within(Duration::from_secs(100)).is_none());
}

#[test]
fn unknown_resident_is_confirmed_then_dropped() {
    let mut h = H::new();
    h.login();
    let stranger = res(77);
    h.env.names.insert(stranger, avatar("Some", "Stranger"));
    h.server.insert(stranger, vec![1]);
    h.m.note_activity(stranger, h.now(), &h.env);
    assert_eq!(h.m.rt.residents[&stranger].activity_discovery, Discovery::Pending);
    let list = h.next_request();
    assert!(list.is_list(), "a first contact relists at once");
    h.answer_ok(&list);
    assert_eq!(h.m.rt.residents[&stranger].activity_discovery, Discovery::Confirming);
    let confirm = h.next_request();
    assert!(confirm.is_list());
    assert_eq!(secs(confirm.at - list.at), 5.0, "the confirmation list after 5 s");
    h.answer_ok(&confirm);
    assert_eq!(h.m.rt.residents[&stranger].activity_discovery, Discovery::None);
    assert!(h.m.rt.queue.is_empty() && h.m.rt.priority_resident.is_none(), "dropped");
    // The quiet deadline of that first IM (15 s) starts one more bounded
    // discovery, as in the LL viewer; then nothing until the hourly list.
    let again = h.next_request();
    assert!(again.is_list() && secs(again.at) == 15.0, "the quiet deadline relists");
    h.answer_ok(&again);
    let confirm = h.next_request();
    assert!(confirm.is_list() && secs(confirm.at - again.at) == 5.0);
    h.answer_ok(&confirm);
    let next = h.next_request();
    assert!(next.is_list() && secs(next.at - confirm.at) == 3600.0, "no history request");

    // Listed by the confirmation: synced at once.
    let mut h = H::new();
    h.login();
    let stranger = res(78);
    h.env.names.insert(stranger, avatar("Other", "Stranger"));
    h.m.note_activity(stranger, h.now(), &h.env);
    let list = h.next_request();
    h.answer_ok(&list);
    h.server.insert(stranger, vec![1]);
    h.listed.push(stranger);
    let confirm = h.next_request();
    h.answer_ok(&confirm);
    let head = h.next_request();
    assert_eq!(h.resident_of(&head), stranger);
}

#[test]
fn pass_stops_at_the_archived_newest_key_and_appends_oldest_first() {
    let mut h = H::new();
    let r = h.resident(0, 1..=8);
    h.store.archives.insert(r, (1..=5).map(|n| row(r, n)).collect());
    h.login();
    let requests = h.serve_all(10.0);
    assert_eq!(requests.len(), 1, "one page reaches the archive");
    let head = h.log.iter().position(|a| matches!(a, Action::Http { body: Some(_), .. }));
    let prepared = h
        .log
        .iter()
        .position(|a| matches!(a, Action::PrepareArchive { resident, .. } if *resident == r));
    assert!(head < prepared, "the archive is scanned after the head page");
    let (who, rows, append) = &h.published()[0];
    assert_eq!(*who, r);
    assert!(*append);
    assert_eq!(rows.iter().map(|x| x.key).collect::<Vec<_>>(), [key(6), key(7), key(8)]);
    assert_eq!(h.m.archive_newest(r), Some(key(8)));
    assert_eq!(
        h.count(|a| matches!(a, Action::ConversationLogEntry { resident, newest_unix_secs: Some(s) } if *resident == r && *s == 8.0)),
        1
    );
    assert!(h.count(|a| matches!(a, Action::TranscriptCreated)) > 0);
    assert!(!h.m.work_active(r) && h.m.head_preview(r).is_empty());
}

#[test]
fn first_sync_without_an_archive_walks_to_the_empty_page() {
    let mut h = H::new();
    let r = h.resident(0, 1..=250);
    h.login();
    let requests = h.serve_all(10.0);
    let cursors: Vec<String> = requests.iter().map(Req::cursor).collect();
    assert_eq!(
        cursors,
        [String::new(), msg(151), msg(51), msg(1)],
        "newest to oldest, then the empty page"
    );
    let (_, rows, append) = &h.published()[0];
    assert!(!*append, "a new archive replaces");
    assert_eq!(rows.len(), 250);
    assert!(rows.windows(2).all(|w| w[0].key < w[1].key), "oldest first");
    assert_eq!(h.m.archive_serial(r), 1);
}

#[test]
fn deletion_cutoff_ends_the_paging() {
    let mut h = H::new();
    h.store.state = StateLoad::Safe {
        deleted_before_ticks: key(150).ticks,
        cleanup_pending: false,
    };
    h.resident(0, 1..=250);
    h.login();
    let requests = h.serve_all(10.0);
    assert_eq!(requests.len(), 2, "the page that reaches the cutoff is the last");
    let rows = &h.published()[0].1;
    assert_eq!((rows.len(), rows[0].key), (100, key(151)));
}

#[test]
fn pass_bounds_abort_with_nothing_written() {
    let mut h = H::new();
    let r = h.resident(0, 1..=(MAX_PASS_ROWS as u64 + 1));
    h.login();
    let requests = h.serve_all(10.0);
    assert_eq!(requests.len(), MAX_PASS_ROWS / 100 + 1);
    assert!(h.published().is_empty(), "rows bound: nothing written");
    assert!(
        h.m.rt.residents[&r].covered_token.is_empty(),
        "not covered: the next list tries again"
    );

    let mut h = H::new();
    h.big_rows = true;
    let row_bytes = direct_conversation_id(AGENT, res(0)).len() + 36 + 36 + 256 + 1024 + 1 + CREATED_AT.len();
    let fits = MAX_PASS_BYTES / row_bytes;
    h.resident(0, 1..=(fits as u64 + 1));
    h.login();
    let requests = h.serve_all(10.0);
    assert_eq!(requests.len(), (fits + 1).div_ceil(100));
    assert!(h.published().is_empty(), "bytes bound: nothing written");
}

#[test]
fn muted_residents_are_never_fetched() {
    let mut h = H::new();
    let (a, b, c) = (h.resident(0, [1]), h.resident(1, [1]), h.resident(2, [1]));
    h.env.muted.insert(b);
    h.env.muted_names.insert("Resident N2".into());
    h.login();
    let fetched: HashSet<Uuid> = h.serve_all(10.0).iter().map(|q| h.resident_of(q)).collect();
    assert_eq!(fetched, HashSet::from([a]));
    assert!(!h.m.work_active(b) && !h.m.work_active(c));
    h.m.prioritize(b, false, &h.env);
    assert!(!h.m.work_active(b), "no loading indicator for a muted resident");
    assert!(h.serve_all(10.0).is_empty());
}

#[test]
fn nothing_goes_out_while_a_gate_is_closed() {
    for (consent, mute_loaded, rollout) in [(true, false, true), (false, true, true), (true, true, false)] {
        let mut h = H::new();
        h.resident(0, [1]);
        h.m.set_gates(Gates {
            consent,
            mute_list_server_loaded: mute_loaded,
        });
        h.start(rollout);
        assert!(
            h.next_request_within(Duration::from_secs(8000)).is_none(),
            "{consent} {mute_loaded} {rollout}"
        );
        assert_eq!(h.m.service_presentation_allowed(), consent && rollout);
        if !mute_loaded {
            // The server's mute list arriving starts the sync.
            h.m.set_gates(Gates {
                consent,
                mute_list_server_loaded: true,
            });
            assert!(h.next_request_within(Duration::from_secs(1)).is_some_and(|q| q.is_list()));
        }
    }
}

#[test]
fn unsafe_state_suppresses_everything() {
    let mut h = H::new();
    let r = h.resident(0, [1]);
    h.store.state = StateLoad::Unsafe;
    h.start(true);
    assert!(h.next_request_within(Duration::from_secs(8000)).is_none());
    assert!(h.m.history_suppressed() && !h.m.service_presentation_allowed());
    assert_eq!(h.count(|a| matches!(a, Action::ListArchives { .. } | Action::StyleUpdate)), 0);
    assert!(h.m.local_history_exists(), "the Delete button stays available");
    assert!(h.m.begin_view_read(r, h.now(), &h.env).is_none());
    h.m.prioritize(r, false, &h.env);
    assert!(!h.m.work_active(r));
}

#[test]
fn delete_succeeds_and_history_comes_back_after_the_click() {
    let mut h = H::new();
    let r = h.resident(0, 1..=5);
    h.login();
    h.serve_all(10.0);
    assert!(h.m.local_history_exists());
    let click = key(3).ticks;
    assert!(h.m.request_delete(click));
    assert!(!h.m.request_delete(click), "one at a time");
    assert!(h.m.history_suppressed());
    h.log.clear();
    h.pump();
    let steps: Vec<&Action> = h
        .log
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::BeginDeleteState { .. }
                    | Action::ClearSessionsForDeletion
                    | Action::StyleUpdate
                    | Action::SweepTranscripts { .. }
                    | Action::WriteState { .. }
                    | Action::DeleteFinished { .. }
            )
        })
        .collect();
    assert_eq!(steps.len(), 6, "{steps:?}");
    assert!(matches!(steps[0], Action::BeginDeleteState { click_ticks, .. } if *click_ticks == click));
    assert!(matches!(steps[1], Action::ClearSessionsForDeletion));
    assert!(matches!(steps[2], Action::StyleUpdate));
    assert!(matches!(steps[3], Action::SweepTranscripts { .. }));
    assert!(matches!(steps[4], Action::WriteState { boundary, cleanup_pending: false, .. } if *boundary == click));
    assert!(matches!(steps[5], Action::DeleteFinished { success: true }));
    assert!(!h.m.history_suppressed());
    assert_eq!(h.m.rt.deleted_before_ticks, click);

    // The list again; only the messages after the click come back.
    h.store.published.clear();
    let requests = h.serve_all(10.0);
    assert!(requests[0].is_list());
    assert_eq!(h.resident_of(&requests[1]), r);
    let (_, rows, append) = &h.published()[0];
    assert_eq!(rows.iter().map(|x| x.key).collect::<Vec<_>>(), [key(4), key(5)]);
    assert!(!*append, "a new archive");
}

#[test]
fn delete_failure_keeps_history_hidden() {
    let mut h = H::new();
    h.resident(0, 1..=5);
    h.login();
    h.serve_all(10.0);
    h.store.sweep_ok = false;
    assert!(h.m.request_delete(key(9).ticks));
    h.pump();
    assert_eq!(h.count(|a| matches!(a, Action::DeleteFinished { success: false })), 1);
    assert!(h.m.history_suppressed(), "hidden until a delete succeeds");
    assert!(h.next_request_within(Duration::from_secs(8000)).is_none(), "no network");
    assert!(h.m.local_history_exists());
    // It may be tried again.
    h.store.sweep_ok = true;
    assert!(h.m.request_delete(key(10).ticks));
    h.pump();
    assert_eq!(h.count(|a| matches!(a, Action::DeleteFinished { success: true })), 1);
    assert!(!h.m.history_suppressed());

    // The pending state cannot be written: unsafe.
    let mut h = H::new();
    h.login();
    assert!(h.m.request_delete(key(9).ticks));
    let actions = h.m.poll(h.now(), &h.env);
    let Some(&Action::BeginDeleteState { ticket, .. }) = actions.iter().find(|a| matches!(a, Action::BeginDeleteState { .. })) else {
        panic!("the delete starts with its state: {actions:?}");
    };
    h.m.delete_state_written(ticket, None);
    h.pump();
    assert_eq!(h.m.rt.state_safety, StateSafety::Unsafe);
    assert!(h.m.history_suppressed());
    assert_eq!(h.count(|a| matches!(a, Action::DeleteFinished { success: false })), 1);
}

#[test]
fn an_interrupted_delete_resumes_at_the_next_login() {
    let mut h = H::new();
    let r = h.resident(0, 1..=5);
    let boundary = key(2).ticks;
    h.store.state = StateLoad::Safe {
        deleted_before_ticks: boundary,
        cleanup_pending: true,
    };
    h.store.archives.insert(r, (1..=2).map(|n| row(r, n)).collect());
    h.start(true);
    assert_eq!(
        h.count(|a| matches!(a, Action::BeginDeleteState { click_ticks, .. } if *click_ticks == boundary)),
        1
    );
    assert_eq!(h.count(|a| matches!(a, Action::DeleteFinished { success: true })), 1);
    assert!(h.store.archives.is_empty(), "swept");
    assert!(matches!(
        h.store.state,
        StateLoad::Safe {
            cleanup_pending: false,
            ..
        }
    ));
    let requests = h.serve_all(10.0);
    assert!(requests[0].is_list());
    assert_eq!(
        h.published()[0].1.iter().map(|x| x.key).collect::<Vec<_>>(),
        [key(3), key(4), key(5)]
    );
}

#[test]
fn name_lookup_times_out_and_is_retried_by_the_next_list() {
    let mut h = H::new();
    let r = h.resident(0, [1]);
    h.env.names.clear();
    h.login();
    assert_eq!(h.count(|a| matches!(a, Action::ResolveName(id) if *id == r)), 1);
    h.advance(29.0);
    assert!(matches!(h.m.rt.residents[&r].metadata, MetaState::Pending { .. }));
    h.advance(1.0);
    assert!(matches!(h.m.rt.residents[&r].metadata, MetaState::Failed), "30 s without a name");
    assert!(!h.m.work_active(r));
    // A late answer of the expired lookup is dropped.
    h.m.name_resolved(r, Some(avatar("Late", "Name")));
    assert!(h.m.resolved_name(r).is_none());

    let list = h.next_request();
    assert!(list.is_list(), "no history request without a name");
    h.answer_ok(&list);
    assert_eq!(h.count(|a| matches!(a, Action::ResolveName(id) if *id == r)), 2, "asked again");
    h.m.name_resolved(r, Some(avatar("Resident", "N0")));
    let head = h.next_request();
    assert_eq!(h.resident_of(&head), r, "queued again once named");
}

#[test]
fn outgoing_capture_takes_the_newest_known_key() {
    let mut h = H::new();
    let r = h.resident(0, 1..=150);
    h.store.archives.insert(r, (1..=5).map(|n| row(r, n)).collect());
    h.login();
    assert_eq!(
        h.m.outgoing_after_msg_id(res(9), TimeUuidKey::default()),
        "",
        "nothing known: empty but present"
    );
    let head = h.next_request();
    h.answer_ok(&head);
    // The head page (150..51) is newer than the archive and the view.
    assert_eq!(h.m.outgoing_after_msg_id(r, key(7)), msg(150));
    h.serve_all(10.0);
    assert_eq!(
        h.m.outgoing_after_msg_id(r, TimeUuidKey::default()),
        msg(150),
        "the archive's newest"
    );
    assert_eq!(h.m.outgoing_after_msg_id(r, key(200)), msg(200), "the view's");
}

#[test]
fn stop_drops_every_awaited_reply() {
    let mut h = H::new();
    h.resident(0, [1]);
    h.start(true);
    let list = h.next_request();
    h.m.stop();
    assert!(h.m.history_suppressed());
    h.m.on_response(list.id, 200, Some(h.list_body()));
    assert!(h.m.poll(h.now(), &h.env).is_empty());
    assert!(h.m.next_wake(h.now()).is_none());
    // A new login starts over, with new tickets.
    h.start(true);
    let again = h.next_request();
    assert!(again.is_list() && again.id != list.id);
}

#[test]
fn click_ticks_is_the_last_tick_of_its_microsecond() {
    assert_eq!(click_ticks(0), Some(UUID_EPOCH + 9));
    assert_eq!(click_ticks(1), Some(UUID_EPOCH + 19));
    assert_eq!(click_ticks(-1), None);
    assert_eq!(click_ticks(i64::MAX), None);
}
