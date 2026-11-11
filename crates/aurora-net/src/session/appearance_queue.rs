//! One UpdateAvatarAppearance request at a time, always for the newest outfit
//! version. Port of LLAppearanceMgr::requestServerAppearanceUpdate and
//! serverAppearanceUpdateCoro (indra/newview/llappearancemgr.cpp, originally
//! LGPL 2.1): mOutstandingAppearanceBakeRequest and mRerequestAppearanceBake
//! hold what comes during a request, the coroutine reads the current version
//! when it sends, and mLastUpdateRequestCOFVersion ends older ones. Sent side
//! by side during a burst of outfit changes, these requests reach the server
//! out of order, and many come back 503.

use parking_lot::Mutex;
use std::future::Future;

/// What one request needs: the outfit version, and the capability of the
/// region at the time of asking (a teleport can happen during a request).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Request {
    pub cof_version: i32,
    pub url: String,
}

#[derive(Default)]
pub(super) struct AppearanceQueue {
    in_flight: bool,
    wanted: Option<Request>,
    /// The version last sent, forgotten on a failure: the server's `expected`
    /// can ask for a lower one, and a failed version can be asked again.
    last_sent: Option<i32>,
}

impl AppearanceQueue {
    fn already_sent(&self, cof_version: i32) -> bool {
        self.last_sent.is_some_and(|sent| sent >= cof_version)
    }

    /// The request to send now; None when one is running (the newest is held
    /// for its end) or when this version was already sent.
    pub fn request(&mut self, request: Request) -> Option<Request> {
        if self.already_sent(request.cof_version) {
            return None;
        }
        if self.in_flight {
            if self.wanted.as_ref().is_none_or(|held| held.cof_version <= request.cof_version) {
                self.wanted = Some(request);
            }
            return None;
        }
        self.in_flight = true;
        self.last_sent = Some(request.cof_version);
        Some(request)
    }

    /// The running request ended: the held one to send next, if any.
    fn finished(&mut self, success: bool) -> Option<Request> {
        if !success {
            self.last_sent = None;
        }
        let next = self.wanted.take().filter(|held| !self.already_sent(held.cof_version));
        self.in_flight = next.is_some();
        if let Some(next) = &next {
            self.last_sent = Some(next.cof_version);
        }
        next
    }
}

/// Send `first`, then what was held meanwhile, one after the other. `post`
/// answers whether the request succeeded.
pub(super) async fn drain<F, Fut>(queue: &Mutex<AppearanceQueue>, first: Request, mut post: F)
where
    F: FnMut(Request) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut next = Some(first);
    while let Some(request) = next {
        let success = post(request).await;
        next = queue.lock().finished(success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, Ordering};

    fn asked(cof_version: i32) -> Request {
        Request {
            cof_version,
            url: format!("region-of-{cof_version}"),
        }
    }

    #[test]
    fn a_burst_sends_one_request_then_only_the_newest() {
        let mut queue = AppearanceQueue::default();
        assert_eq!(queue.request(asked(10)), Some(asked(10)));
        for version in [11, 13, 12] {
            assert_eq!(queue.request(asked(version)), None, "held while one is running");
        }
        assert_eq!(queue.finished(true), Some(asked(13)), "the newest, with its own region");
        assert_eq!(queue.finished(true), None);
        assert_eq!(queue.request(asked(14)), Some(asked(14)), "free again");
    }

    #[test]
    fn a_version_already_sent_is_not_sent_again() {
        let mut queue = AppearanceQueue::default();
        assert!(queue.request(asked(10)).is_some());
        assert_eq!(queue.request(asked(10)), None);
        assert_eq!(queue.finished(true), None);
        assert_eq!(queue.request(asked(10)), None, "the same version");
        assert_eq!(queue.request(asked(9)), None, "an older one");
    }

    /// The server's `expected` retry can be lower than what was sent.
    #[test]
    fn a_failure_lets_a_version_be_asked_again() {
        let mut queue = AppearanceQueue::default();
        assert!(queue.request(asked(10)).is_some());
        assert_eq!(queue.finished(false), None);
        assert_eq!(queue.request(asked(9)), Some(asked(9)));
        assert_eq!(queue.request(asked(10)), None, "running");
        assert_eq!(queue.finished(false), Some(asked(10)), "held through a failure");
        assert_eq!(queue.finished(true), None);
    }

    #[tokio::test]
    async fn requests_never_overlap_and_the_queue_frees_itself() {
        let queue = Arc::new(Mutex::new(AppearanceQueue::default()));
        let running = Arc::new(AtomicI32::new(0));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let first = queue.lock().request(asked(1)).expect("the first goes out");
        let worker = {
            let (queue, running, sent) = (queue.clone(), running.clone(), sent.clone());
            tokio::spawn(async move {
                let (running, sent, inner) = (&running, &sent, queue.clone());
                drain(&queue, first, |request| {
                    let inner = inner.clone();
                    async move {
                        assert_eq!(running.fetch_add(1, Ordering::SeqCst), 0, "one at a time");
                        sent.lock().push(request.cof_version);
                        if request.cof_version == 1 {
                            assert_eq!(inner.lock().request(asked(2)), None);
                            assert_eq!(inner.lock().request(asked(3)), None);
                        }
                        tokio::task::yield_now().await;
                        running.fetch_sub(1, Ordering::SeqCst);
                        true
                    }
                })
                .await;
            })
        };
        worker.await.expect("worker");
        assert_eq!(*sent.lock(), [1, 3]);
        assert_eq!(queue.lock().request(asked(4)), Some(asked(4)));
    }
}
