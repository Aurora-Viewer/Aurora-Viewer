//! Session-scoped AIS item reads. Coalesce callers before spawning work;
//! sixteen reads across the session, rather than sixteen per command.
//! Folder/tree reads retain their separate budget and interactive slot.

use crate::inventory::ais::ItemReads;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use uuid::Uuid;

const ACTIVE_READS: usize = 16;
type Key = (Uuid, Uuid);

#[derive(Clone)]
struct Read {
    key: Key,
    cap: Arc<str>,
    generation: u64,
}

struct Entry {
    cap: Arc<str>,
    generation: u64,
    due: Instant,
    ordinary: bool,
    attachment: bool,
    abort: Option<tokio::task::AbortHandle>,
    reading_generation: Option<u64>,
}

#[derive(Default)]
struct Queue {
    entries: HashMap<Key, Entry>,
    waiting: VecDeque<Key>,
    failures: HashMap<Key, (u8, Instant, Arc<str>)>,
    overloaded_until: Option<(Arc<str>, Instant)>,
    next_generation: u64,
    suspended: bool,
}

impl Queue {
    fn enqueue(&mut self, owner: Uuid, items: Vec<Uuid>, cap: Arc<str>, attachment: bool) {
        let now = Instant::now();
        for item in items {
            let key = (owner, item);
            let generation = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1);
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.ordinary |= !attachment;
                if attachment
                    && !entry.attachment
                    && let Some(index) = self.waiting.iter().position(|waiting| *waiting == key)
                {
                    self.waiting.remove(index);
                    self.waiting.push_front(key);
                }
                entry.attachment |= attachment;
                if entry.cap != cap {
                    entry.cap = cap.clone();
                    entry.generation = generation;
                    entry.due = now;
                }
                continue;
            }
            self.entries.insert(
                key,
                Entry {
                    cap: cap.clone(),
                    generation,
                    due: self
                        .failures
                        .get(&key)
                        .filter(|(_, _, failed_cap)| failed_cap == &cap)
                        .map_or(now, |(_, due, _)| *due),
                    ordinary: !attachment,
                    attachment,
                    abort: None,
                    reading_generation: None,
                },
            );
            if attachment {
                self.waiting.push_front(key);
            } else {
                self.waiting.push_back(key);
            }
        }
    }

    fn retarget(&mut self, cap: Option<Arc<str>>) {
        if self.suspended && cap.is_none() {
            return;
        }
        let now = Instant::now();
        for entry in self.entries.values_mut() {
            if cap.as_ref().is_some_and(|new| *new == entry.cap) && !self.suspended {
                continue;
            }
            if let Some(new) = &cap {
                entry.cap = new.clone();
            }
            entry.generation = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1);
            entry.due = now;
            if let Some(abort) = entry.abort.take() {
                abort.abort();
            }
        }
        self.suspended = cap.is_none();
    }

    fn start(&mut self) -> Option<Read> {
        if self.suspended {
            return None;
        }
        let now = Instant::now();
        let index = self.waiting.iter().position(|key| {
            self.entries
                .get(key)
                .is_some_and(|entry| self.due(entry) <= now && entry.reading_generation.is_none())
        })?;
        let key = self.waiting.remove(index)?;
        let entry = self.entries.get_mut(&key)?;
        entry.reading_generation = Some(entry.generation);
        Some(Read {
            key,
            cap: entry.cap.clone(),
            generation: entry.generation,
        })
    }

    fn finish(&mut self, read: &Read, failed: bool) -> Option<(bool, bool)> {
        let entry = self.entries.get_mut(&read.key)?;
        if entry.reading_generation != Some(read.generation) {
            return None;
        }
        if read.generation != entry.generation {
            // A region change supplied a newer capability while the old read
            // was running. Its 404/error must not mark the item missing.
            self.waiting.push_front(read.key);
            entry.abort = None;
            entry.reading_generation = None;
            return None;
        }
        let entry = self.entries.remove(&read.key)?;
        if failed {
            let (attempts, due, cap) = self.failures.entry(read.key).or_insert((0, Instant::now(), read.cap.clone()));
            if cap != &read.cap {
                *attempts = 0;
                *cap = read.cap.clone();
            }
            *attempts = attempts.saturating_add(1);
            *due = Instant::now()
                + Duration::from_secs(match *attempts {
                    1 => 2,
                    2 => 10,
                    _ => 60,
                });
        } else {
            self.failures.remove(&read.key);
        }
        Some((entry.ordinary, entry.attachment))
    }

    fn delay(&self) -> Duration {
        if self.suspended {
            return Duration::from_secs(86400);
        }
        self.waiting
            .iter()
            .filter_map(|key| self.entries.get(key))
            .map(|entry| self.due(entry))
            .min()
            .map_or(Duration::from_secs(86400), |due| due.saturating_duration_since(Instant::now()))
    }

    fn due(&self, entry: &Entry) -> Instant {
        self.overloaded_until
            .as_ref()
            .filter(|(cap, _)| *cap == entry.cap)
            .map_or(entry.due, |(_, until)| entry.due.max(*until))
    }

    fn pause_overloaded(&mut self, cap: Arc<str>, delay: Duration) {
        let due = Instant::now() + delay;
        let pause = self.overloaded_until.get_or_insert_with(|| (cap.clone(), due));
        if pause.0 == cap {
            pause.1 = pause.1.max(due);
        } else {
            *pause = (cap, due);
        }
    }
}

pub(super) struct Reader {
    queue: Arc<Mutex<Queue>>,
    wake: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
    events: crate::EventSender,
}

impl Reader {
    /// Keep in-flight consumers across a crossing, but prevent the old
    /// capability's response from classifying an original on the new seed.
    pub(super) fn retarget(&self, cap: Option<String>, seed_resolved: bool) {
        let mut queue = self.queue.lock();
        let unavailable = cap.is_none() && seed_resolved;
        queue.retarget(cap.map(Arc::from));
        if unavailable {
            queue.waiting.clear();
            for ((_, id), entry) in queue.entries.drain() {
                super::send_item_reads(
                    &self.events,
                    ItemReads {
                        failed: vec![id],
                        ..Default::default()
                    },
                    entry.attachment && !entry.ordinary,
                    entry.attachment && entry.ordinary,
                );
            }
        }
        drop(queue);
        self.wake.notify_one();
    }

    /// Only the model acknowledges accepted records, after checking folder
    /// versions. Cancel ordinary duplicate GETs, preserving the attachment
    /// metadata reads' special parent-repair semantics.
    pub(super) fn known(&self, owner: Uuid, items: &[Uuid]) {
        {
            let mut queue = self.queue.lock();
            for item in items {
                let key = (owner, *item);
                queue.failures.remove(&key);
                if queue.entries.get(&key).is_some_and(|entry| entry.attachment) {
                    continue;
                }
                if let Some(entry) = queue.entries.remove(&key)
                    && let Some(abort) = entry.abort
                {
                    abort.abort();
                }
            }
            let Queue { entries, waiting, .. } = &mut *queue;
            waiting.retain(|key| entries.contains_key(key));
        }
        self.wake.notify_one();
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // Dropping the worker drops its JoinSet too: old-account reads may
        // not deliver events into the next login's inventory.
        self.task.abort();
    }
}

impl Reader {
    pub(super) fn new(http: reqwest::Client, events: crate::EventSender) -> Self {
        Self::with_fetch(events, move |owner, item, cap| {
            let http = http.clone();
            async move { crate::inventory::ais::read_one_item(&http, &cap, owner, item).await }
        })
    }

    fn with_fetch<F, Fut>(events: crate::EventSender, fetch: F) -> Self
    where
        F: Fn(Uuid, Uuid, Arc<str>) -> Fut + Send + 'static,
        Fut: Future<Output = ItemReads> + Send + 'static,
    {
        let queue = Arc::new(Mutex::new(Queue::default()));
        let wake = Arc::new(Notify::new());
        let worker_queue = queue.clone();
        let worker_wake = wake.clone();
        let worker_events = events.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            let mut active = HashMap::new();
            loop {
                while tasks.len() < ACTIVE_READS {
                    let Some(read) = worker_queue.lock().start() else { break };
                    let future = fetch(read.key.0, read.key.1, read.cap.clone());
                    let handle = tasks.spawn(future);
                    if let Some(entry) = worker_queue
                        .lock()
                        .entries
                        .get_mut(&read.key)
                        .filter(|entry| entry.generation == read.generation && entry.reading_generation == Some(read.generation))
                    {
                        entry.abort = Some(handle.clone());
                    } else {
                        handle.abort();
                    }
                    active.insert(handle.id(), read);
                }
                let delay = if tasks.len() < ACTIVE_READS {
                    worker_queue.lock().delay()
                } else {
                    Duration::from_secs(86400)
                };
                tokio::select! {
                    _ = worker_wake.notified() => {},
                    _ = tokio::time::sleep(delay) => {},
                    result = tasks.join_next_with_id(), if !tasks.is_empty() => {
                        let Some(result) = result else { continue };
                        let (task_id, result) = match result {
                            Ok((task_id, reads)) => (task_id, Some(reads)),
                            Err(error) => (error.id(), None),
                        };
                        let Some(read) = active.remove(&task_id) else { continue };
                        let reads = result.unwrap_or_else(|| ItemReads { failed: vec![read.key.1], ..Default::default() });
                        // Serialize delivery with a capability switch.
                        let mut queue = worker_queue.lock();
                        if let Some((ordinary, attachment)) = queue.finish(&read, !reads.failed.is_empty()) {
                            if let Some(delay) = reads.overload_pause {
                                queue.pause_overloaded(read.cap, delay);
                            }
                            super::send_item_reads(&worker_events, reads, attachment && !ordinary, attachment && ordinary);
                        }
                    },
                }
            }
        });
        Self { queue, wake, task, events }
    }

    pub(super) fn enqueue(&self, owner: Uuid, items: Vec<Uuid>, cap: String, attachment: bool) {
        self.queue.lock().enqueue(owner, items, Arc::from(cap), attachment);
        self.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn duplicate_callers_share_one_read_and_keep_both_delivery_kinds() {
        let mut queue = Queue::default();
        let owner = Uuid::from_u128(1);
        let item = Uuid::from_u128(2);
        for _ in 0..100 {
            queue.enqueue(owner, vec![item], Arc::from("synthetic-cap"), false);
        }
        queue.enqueue(owner, vec![item], Arc::from("synthetic-cap"), true);
        let read = queue.start().unwrap();
        assert!(queue.start().is_none());
        assert_eq!(queue.finish(&read, false), Some((true, true)));
        assert!(queue.entries.is_empty());
    }

    #[test]
    fn queued_ordinary_read_is_promoted_by_an_attachment_consumer() {
        let mut queue = Queue::default();
        let owner = Uuid::new_v4();
        let first = Uuid::new_v4();
        let attachment = Uuid::new_v4();
        queue.enqueue(owner, vec![first, attachment], Arc::from("synthetic-cap"), false);
        queue.enqueue(owner, vec![attachment], Arc::from("synthetic-cap"), true);
        assert_eq!(queue.start().unwrap().key, (owner, attachment));
        assert_eq!(queue.start().unwrap().key, (owner, first));
        assert!(queue.start().is_none());
    }

    #[test]
    fn obsolete_capability_results_are_discarded_and_read_on_the_new_capability() {
        let mut queue = Queue::default();
        queue.enqueue(Uuid::nil(), vec![Uuid::from_u128(2)], Arc::from("old"), false);
        let old = queue.start().unwrap();
        queue.enqueue(Uuid::nil(), vec![Uuid::from_u128(2)], Arc::from("new"), true);
        assert_eq!(queue.finish(&old, true), None);
        let new = queue.start().unwrap();
        assert_eq!(&*new.cap, "new");
        assert_eq!(queue.finish(&new, false), Some((true, true)));
    }

    #[test]
    fn a_region_switch_invalidates_an_active_read_without_a_second_consumer_command() {
        let mut queue = Queue::default();
        let owner = Uuid::nil();
        let item = Uuid::from_u128(2);
        queue.enqueue(owner, vec![item], Arc::from("old"), false);
        queue.enqueue(owner, vec![item], Arc::from("old"), true);
        let old = queue.start().unwrap();
        queue.retarget(None);
        assert!(queue.finish(&old, false).is_none(), "an old 404 must not reach consumers");
        assert!(queue.start().is_none(), "wait for the new seed");
        queue.retarget(Some(Arc::from("new")));
        let new = queue.start().unwrap();
        assert_eq!(&*new.cap, "new");
        assert_eq!(queue.finish(&new, false), Some((true, true)));
    }

    #[test]
    fn an_overload_pauses_new_item_reads_on_that_capability_only() {
        let mut queue = Queue::default();
        let owner = Uuid::nil();
        let first = Uuid::from_u128(1);
        let next = Uuid::from_u128(2);
        let old: Arc<str> = Arc::from("old");
        queue.enqueue(owner, vec![first, next], old.clone(), false);
        let active = queue.start().unwrap();
        assert_eq!(queue.finish(&active, true), Some((true, false)));
        queue.pause_overloaded(old, Duration::from_secs(27));
        assert!(queue.start().is_none(), "a fresh item must also honor the server's pause");
        assert!(queue.delay() > Duration::from_secs(20));
        queue.retarget(Some(Arc::from("new")));
        assert_eq!(queue.start().unwrap().key, (owner, next));
    }

    #[tokio::test]
    async fn a_shared_success_is_delivered_once_with_attachment_parent_context() {
        let (events, received) = crossbeam_channel::unbounded();
        let reader = Reader::with_fetch(crate::EventSender(events), |_, id, _| async move {
            ItemReads {
                items: vec![crate::inventory::InvItem { id, ..Default::default() }],
                ..Default::default()
            }
        });
        let item = Uuid::from_u128(2);
        reader.enqueue(Uuid::nil(), vec![item], "synthetic-cap".into(), false);
        reader.enqueue(Uuid::nil(), vec![item], "synthetic-cap".into(), true);
        let mut response = None;
        for _ in 0..100 {
            if let Ok((_, event)) = received.try_recv() {
                response = Some(event);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            matches!(response, Some(crate::NetEvent::AttachmentInventoryItems { items, coalesced: true })
            if items.len() == 1 && items[0].id == item)
        );
        assert!(received.is_empty());
    }

    #[tokio::test]
    async fn region_without_ais_releases_a_shared_read_as_temporary_failure() {
        let (events, received) = crossbeam_channel::unbounded();
        let reader = Reader::with_fetch(crate::EventSender(events), |_, _, _| std::future::pending::<ItemReads>());
        let item = Uuid::from_u128(2);
        reader.enqueue(Uuid::nil(), vec![item], "synthetic-cap".into(), false);
        reader.enqueue(Uuid::nil(), vec![item], "synthetic-cap".into(), true);
        reader.retarget(None, true);
        assert!(matches!(received.try_recv(),
            Ok((_, crate::NetEvent::InventoryItemsFailed { items, attachment: false, coalesced: true }))
            if items == [item]));
        assert!(received.is_empty());
    }

    #[test]
    fn repeated_producers_cannot_turn_a_failed_item_into_an_immediate_retry_loop() {
        let mut queue = Queue::default();
        let item = Uuid::from_u128(2);
        queue.enqueue(Uuid::nil(), vec![item], Arc::from("synthetic-cap"), false);
        let read = queue.start().unwrap();
        queue.finish(&read, true);
        for _ in 0..100 {
            queue.enqueue(Uuid::nil(), vec![item], Arc::from("synthetic-cap"), false);
        }
        assert!(queue.start().is_none());
        assert_eq!(queue.waiting.len(), 1);
        assert!(queue.delay() > Duration::from_secs(1));
        queue.entries.get_mut(&(Uuid::nil(), item)).unwrap().due = Instant::now();
        assert!(queue.start().is_some());
    }

    #[tokio::test]
    async fn independent_commands_share_the_active_budget_and_do_not_spawn_duplicate_reads() {
        let (events, _received) = crossbeam_channel::unbounded();
        let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let release_fetch = release.clone();
        let reader = Reader::with_fetch(crate::EventSender(events), move |_, item, _| {
            let started = started.clone();
            let release = release_fetch.clone();
            async move {
                started.send(item).unwrap();
                release.acquire().await.unwrap().forget();
                ItemReads::default()
            }
        });
        let ids: Vec<_> = (1..=32).map(Uuid::from_u128).collect();
        for _ in 0..100 {
            reader.enqueue(Uuid::nil(), ids.clone(), "synthetic-cap".into(), false);
        }
        let mut first = std::collections::HashSet::new();
        for _ in 0..ACTIVE_READS {
            first.insert(tokio::time::timeout(Duration::from_secs(2), starts.recv()).await.unwrap().unwrap());
        }
        assert_eq!(first.len(), ACTIVE_READS);
        assert!(tokio::time::timeout(Duration::from_millis(30), starts.recv()).await.is_err());
        release.add_permits(ACTIVE_READS);
        for _ in 0..ACTIVE_READS {
            assert!(first.insert(tokio::time::timeout(Duration::from_secs(2), starts.recv()).await.unwrap().unwrap()));
        }
        assert_eq!(first.len(), 32);
        assert!(tokio::time::timeout(Duration::from_millis(30), starts.recv()).await.is_err());
        release.add_permits(ACTIVE_READS);
    }

    #[tokio::test]
    async fn ending_a_session_cancels_its_pending_item_reads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Active(Arc<AtomicUsize>);
        impl Drop for Active {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        let worker_count = count.clone();
        let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
        let (events, received) = crossbeam_channel::unbounded();
        let reader = Reader::with_fetch(crate::EventSender(events), move |_, _, _| {
            let count = worker_count.clone();
            let started = started.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                let _active = Active(count);
                started.send(()).unwrap();
                std::future::pending::<ItemReads>().await
            }
        });
        reader.enqueue(Uuid::nil(), vec![Uuid::from_u128(2)], "synthetic-cap".into(), false);
        tokio::time::timeout(Duration::from_secs(2), starts.recv()).await.unwrap().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(reader);
        tokio::time::timeout(Duration::from_secs(2), async {
            while count.load(Ordering::SeqCst) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(received.try_recv().is_err());
    }

    #[tokio::test]
    async fn accepted_bulk_items_cancel_duplicates_but_keep_attachment_and_newer_reads() {
        let (events, received) = crossbeam_channel::unbounded();
        let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let release_fetch = release.clone();
        let reader = Reader::with_fetch(crate::EventSender(events), move |_, item, _| {
            let started = started.clone();
            let release = release_fetch.clone();
            async move {
                started.send(item).unwrap();
                release.acquire().await.unwrap().forget();
                ItemReads {
                    failed: vec![item],
                    ..Default::default()
                }
            }
        });
        let ids: Vec<_> = (1..=32).map(Uuid::from_u128).collect();
        reader.enqueue(Uuid::nil(), ids.clone(), "synthetic-cap".into(), false);
        reader.enqueue(Uuid::nil(), vec![ids[0]], "synthetic-cap".into(), true);
        for _ in 0..ACTIVE_READS {
            tokio::time::timeout(Duration::from_secs(2), starts.recv()).await.unwrap().unwrap();
        }
        reader.known(Uuid::nil(), &ids);
        assert_eq!(
            reader.queue.lock().entries.len(),
            1,
            "attachment metadata keeps its special repair read"
        );
        // Reuse a canceled key before the old cancellation callback runs.
        // Its result must not remove or fail this new generation.
        reader.enqueue(Uuid::nil(), vec![ids[1]], "synthetic-cap".into(), false);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), starts.recv()).await.unwrap().unwrap(),
            ids[1]
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), starts.recv()).await.is_err(),
            "the other known items are never started"
        );
        release.add_permits(2);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !reader.queue.lock().entries.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let failed: std::collections::HashSet<_> = received
            .try_iter()
            .flat_map(|(_, event)| match event {
                crate::NetEvent::InventoryItemsFailed { items, .. } => items,
                _ => panic!("only the two intentionally failed reads should answer"),
            })
            .collect();
        assert_eq!(failed, [ids[0], ids[1]].into_iter().collect());
    }
}
