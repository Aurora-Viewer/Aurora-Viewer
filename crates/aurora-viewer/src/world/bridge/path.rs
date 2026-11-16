//! The pathfinding console's path test through the bridge
//! (`Query::StaticPath`, llGetStaticPath in query.lsl). The Legacy Viewer
//! computes the path itself with Havok (LLPathfindingPathTool::computeFinalPath);
//! Aurora asks the region. One request in flight, at most two a second, and
//! the newest asked path waits in place of an older one; a region change or a
//! new bridge session drops them (the points are region coordinates).
use aurora_net::{
    NetCommand, RegionHandle,
    bridge::{
        Endpoint,
        query::{Query, Reply, StaticPath},
    },
};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The rate limit: two requests a second.
pub const MIN_GAP: Duration = Duration::from_millis(500);
const TIMEOUT: Duration = Duration::from_secs(5);

/// What a path request came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Path(StaticPath),
    /// The bridge could not answer (its message).
    Failed(String),
}

#[derive(Default)]
pub struct PathQueue {
    region: Option<RegionHandle>,
    // In-memory epoch only, like the other bridge services.
    session: Option<Uuid>,
    waiting: Option<Query>,
    flight: Option<(Uuid, Instant)>,
    last_sent: Option<Instant>,
}

impl PathQueue {
    /// A new path to ask; it replaces one still waiting.
    pub fn ask(&mut self, query: Query) {
        if query.valid() {
            self.waiting = Some(query);
        }
    }

    /// Clear path, or the console closed: nothing more is shown.
    pub fn cancel(&mut self) {
        self.waiting = None;
        self.flight = None;
    }

    /// A request waits or is on its way.
    pub fn busy(&self) -> bool {
        self.waiting.is_some() || self.flight.is_some()
    }

    /// Follow the agent's region and the bridge session: moving from one
    /// known region (or session) to another drops what was asked, and the
    /// caller forgets the points (true). The first region or session to be
    /// known is not a change.
    pub fn sync(&mut self, region: Option<RegionHandle>, endpoint: Option<&Endpoint>) -> bool {
        let session = endpoint.map(|e| e.token);
        let region_changed = self.region.is_some() && self.region != region;
        let session_changed = self.session.is_some() && session.is_some() && self.session != session;
        if region.is_some() {
            self.region = region;
        }
        if session.is_some() {
            self.session = session;
        }
        if region_changed || session_changed {
            self.cancel();
            return true;
        }
        false
    }

    /// The request to send now, if any, and a timed-out request's outcome.
    pub fn update(&mut self, endpoint: Option<&Endpoint>, now: Instant) -> (Option<NetCommand>, Option<Outcome>) {
        if let Some((_, at)) = self.flight {
            if now.duration_since(at) < TIMEOUT {
                return (None, None);
            }
            self.flight = None;
            return (None, Some(Outcome::Failed(aurora_i18n::tr!("bridge-query-timeout").into())));
        }
        if self.last_sent.is_some_and(|at| now.duration_since(at) < MIN_GAP) {
            return (None, None);
        }
        let Some(endpoint) = endpoint else { return (None, None) };
        let Some(query) = self.waiting.take() else { return (None, None) };
        let request = Uuid::new_v4();
        self.flight = Some((request, now));
        self.last_sent = Some(now);
        (
            Some(NetCommand::BridgeQuery {
                request,
                endpoint: endpoint.clone(),
                query,
            }),
            None,
        )
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.flight.is_some_and(|(r, _)| r == request)
    }

    /// The bridge's answer to the request in flight; None for another one.
    pub fn reply(&mut self, request: Uuid, result: Result<Reply, String>) -> Option<Outcome> {
        if !self.owns(request) {
            return None;
        }
        self.flight = None;
        Some(match result {
            Ok(Reply::StaticPath(path)) => Outcome::Path(path),
            Ok(_) => Outcome::Failed(aurora_i18n::tr!("bridge-request-response").into()),
            Err(error) => Outcome::Failed(error),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> Endpoint {
        Endpoint {
            url: "https://sim.example/cap/1".into(),
            token: Uuid::from_u128(1),
        }
    }

    fn query(x: f32) -> Query {
        Query::StaticPath {
            start: [x, 10.0, 20.0],
            end: [50.0, 50.0, 20.0],
            radius: 0.5,
            character: 4,
        }
    }

    fn sent(command: Option<NetCommand>) -> (Uuid, Query) {
        match command {
            Some(NetCommand::BridgeQuery { request, query, .. }) => (request, query),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn one_request_at_a_time_two_a_second_the_newest_waiting() {
        let mut q = PathQueue::default();
        let e = endpoint();
        let t0 = Instant::now();
        q.ask(query(1.0));
        let (request, first) = sent(q.update(Some(&e), t0).0);
        assert_eq!(first, query(1.0));
        q.ask(query(2.0));
        q.ask(query(3.0));
        assert!(q.update(Some(&e), t0).0.is_none(), "one in flight");
        let path = StaticPath {
            status: 0,
            points: vec![[1.0, 2.0, 3.0]],
        };
        assert_eq!(q.reply(request, Ok(Reply::StaticPath(path.clone()))), Some(Outcome::Path(path)));
        assert!(q.update(Some(&e), t0 + Duration::from_millis(100)).0.is_none(), "two a second");
        let (_, newest) = sent(q.update(Some(&e), t0 + MIN_GAP).0);
        assert_eq!(newest, query(3.0), "the newest replaced the older one");
    }

    #[test]
    fn nothing_leaves_without_a_bridge_and_a_silent_one_times_out() {
        let mut q = PathQueue::default();
        let t0 = Instant::now();
        q.ask(query(1.0));
        assert!(q.update(None, t0).0.is_none());
        assert!(q.busy(), "still waiting for the bridge");
        let (request, _) = sent(q.update(Some(&endpoint()), t0).0);
        assert!(q.owns(request));
        let (command, outcome) = q.update(Some(&endpoint()), t0 + TIMEOUT);
        assert!(command.is_none() && matches!(outcome, Some(Outcome::Failed(_))));
        assert_eq!(q.reply(request, Err("late".into())), None, "a late answer is dropped");
    }

    #[test]
    fn a_region_change_or_a_new_session_drops_the_request() {
        let mut q = PathQueue::default();
        let e = endpoint();
        q.sync(Some(1), Some(&e));
        q.ask(query(1.0));
        assert!(!q.sync(Some(1), Some(&e)));
        assert!(q.sync(Some(2), Some(&e)), "another region");
        assert!(!q.busy());
        q.ask(query(1.0));
        let other = Endpoint {
            token: Uuid::from_u128(2),
            ..endpoint()
        };
        assert!(q.sync(Some(2), Some(&other)), "another bridge session");
        q.ask(query(f32::NAN));
        assert!(!q.busy(), "an invalid path is never asked");
    }

    #[test]
    fn the_first_known_region_and_session_are_not_changes() {
        let mut q = PathQueue::default();
        q.ask(query(1.0));
        assert!(!q.sync(None, None));
        assert!(!q.sync(Some(1), None), "the region becomes known");
        assert!(!q.sync(Some(1), Some(&endpoint())), "the bridge becomes ready");
        assert!(!q.sync(Some(1), None), "the bridge goes quiet for a moment");
        assert!(q.busy(), "what was asked stays");
    }
}
