//! Prioritised, concurrency-limited HTTP fetcher for assets (textures,
//! meshes, materials). Results are delivered on a crossbeam channel so the
//! render thread can poll them without blocking.

use crate::stats::NetStats;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};

#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// Caller-defined key, echoed in the result.
    pub key: u64,
    pub url: String,
    /// Inclusive byte range.
    pub range: Option<(u64, u64)>,
    /// Higher is more urgent.
    pub priority: f32,
    pub accept: &'static str,
}

#[derive(Debug)]
pub struct FetchResult {
    pub key: u64,
    pub status: u16,
    /// Whole resource was returned (no further ranges available).
    pub complete: bool,
    pub data: Result<Vec<u8>, String>,
}

enum Msg {
    Request(FetchRequest),
    Cancel(u64),
}

struct Queued(FetchRequest, u64);

impl PartialEq for Queued {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == CmpOrdering::Equal
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, o: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(o))
    }
}
impl Ord for Queued {
    fn cmp(&self, o: &Self) -> CmpOrdering {
        self.0.priority.total_cmp(&o.0.priority).then_with(|| o.1.cmp(&self.1))
    }
}

#[derive(Clone)]
pub struct Fetcher {
    tx: mpsc::UnboundedSender<Msg>,
}

impl Fetcher {
    pub fn new(
        rt: &tokio::runtime::Handle,
        http: reqwest::Client,
        max_concurrent: usize,
        stats: Arc<NetStats>,
    ) -> (Fetcher, crossbeam_channel::Receiver<FetchResult>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (res_tx, res_rx) = crossbeam_channel::unbounded();
        rt.spawn(dispatcher(rx, res_tx, http, max_concurrent, stats));
        (Fetcher { tx }, res_rx)
    }

    /// Queue (or re-prioritise) a request.
    pub fn request(&self, r: FetchRequest) {
        let _ = self.tx.send(Msg::Request(r));
    }

    pub fn cancel(&self, key: u64) {
        let _ = self.tx.send(Msg::Cancel(key));
    }
}

async fn dispatcher(
    mut rx: mpsc::UnboundedReceiver<Msg>,
    res_tx: crossbeam_channel::Sender<FetchResult>,
    http: reqwest::Client,
    max_concurrent: usize,
    stats: Arc<NetStats>,
) {
    let sem = Arc::new(Semaphore::new(max_concurrent));
    let mut heap: BinaryHeap<Queued> = BinaryHeap::new();
    // key -> generation of the latest request; stale heap entries are skipped
    let mut live: HashMap<u64, u64> = HashMap::new();
    let mut generation: u64 = 0;
    loop {
        // Drain incoming messages without blocking when we have work.
        loop {
            let msg = if heap.is_empty() {
                match rx.recv().await {
                    Some(m) => m,
                    None => return,
                }
            } else {
                match rx.try_recv() {
                    Ok(m) => m,
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => return,
                }
            };
            match msg {
                Msg::Request(r) => {
                    generation += 1;
                    live.insert(r.key, generation);
                    heap.push(Queued(r, generation));
                }
                Msg::Cancel(k) => {
                    live.remove(&k);
                }
            }
        }
        stats.http_queued.store(live.len() as u32, Ordering::Relaxed);

        // Wait for a free slot, but keep accepting messages meanwhile.
        let permit = tokio::select! {
            p = sem.clone().acquire_owned() => match p { Ok(p) => p, Err(_) => return },
            m = rx.recv() => {
                match m {
                    Some(Msg::Request(r)) => { generation += 1; live.insert(r.key, generation); heap.push(Queued(r, generation)); }
                    Some(Msg::Cancel(k)) => { live.remove(&k); }
                    None => return,
                }
                continue;
            }
        };
        let next = loop {
            match heap.pop() {
                Some(Queued(r, g)) => {
                    if live.get(&r.key) == Some(&g) {
                        live.remove(&r.key);
                        break Some(r);
                    }
                }
                None => break None,
            }
        };
        let Some(req) = next else {
            drop(permit);
            continue;
        };
        let http = http.clone();
        let res_tx = res_tx.clone();
        let stats = stats.clone();
        tokio::spawn(async move {
            stats.http_in_flight.fetch_add(1, Ordering::Relaxed);
            let result = do_fetch(&http, &req).await;
            stats.http_in_flight.fetch_sub(1, Ordering::Relaxed);
            match &result.data {
                Ok(d) => {
                    stats.http_bytes.fetch_add(d.len() as u64, Ordering::Relaxed);
                    stats.http_completed.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    stats.http_failed.fetch_add(1, Ordering::Relaxed);
                }
            }
            let _ = res_tx.send(result);
            drop(permit);
        });
    }
}

async fn do_fetch(http: &reqwest::Client, req: &FetchRequest) -> FetchResult {
    let mut last_err = String::new();
    for attempt in 0..3u32 {
        let mut rb = http
            .get(&req.url)
            .header("Accept", req.accept)
            .header("Accept-Encoding", "identity")
            .timeout(Duration::from_secs(60));
        if let Some((a, b)) = req.range {
            rb = rb.header("Range", format!("bytes={a}-{b}"));
        }
        match rb.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                // 503 = throttled: back off and retry.
                if status == 503 || status == 429 {
                    tokio::time::sleep(Duration::from_millis(250 * (attempt as u64 + 1))).await;
                    last_err = format!("HTTP {status}");
                    continue;
                }
                if status == 416 {
                    // Requested range beyond the end: resource is complete.
                    return FetchResult {
                        key: req.key,
                        status,
                        complete: true,
                        data: Ok(Vec::new()),
                    };
                }
                if !(200..300).contains(&status) {
                    return FetchResult {
                        key: req.key,
                        status,
                        complete: true,
                        data: Err(format!("HTTP {status}")),
                    };
                }
                let total = resp
                    .headers()
                    .get("Content-Range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.rsplit('/').next())
                    .and_then(|s| s.parse::<u64>().ok());
                match resp.bytes().await {
                    Ok(b) => {
                        let complete = match (status, req.range, total) {
                            (200, _, _) => true,
                            (_, Some((a, _)), Some(t)) => a + b.len() as u64 >= t,
                            (_, Some((a, end)), None) => (b.len() as u64) < end - a + 1,
                            _ => true,
                        };
                        return FetchResult {
                            key: req.key,
                            status,
                            complete,
                            data: Ok(b.to_vec()),
                        };
                    }
                    Err(e) => last_err = e.to_string(),
                }
            }
            Err(e) => {
                last_err = e.to_string();
                tokio::time::sleep(Duration::from_millis(200 * (attempt as u64 + 1))).await;
            }
        }
    }
    FetchResult {
        key: req.key,
        status: 0,
        complete: true,
        data: Err(last_err),
    }
}
