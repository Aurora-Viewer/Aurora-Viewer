//! A local asset server for the demo (`net=<ms>` of AURORA_DEMO_STREAM):
//! it stands for the grid's asset hosts, so that the whole download path
//! runs without a grid — the fetcher with its queue and its connections,
//! the byte ranges, the replies that continue what is held — at the round
//! trip of a distant simulator. Each request is answered after `delay`,
//! whatever the number of requests under way, as on a link where latency
//! and not throughput is the limit.
//!
//! It serves textures, and with `start_assets` meshes and animations too
//! (AURORA_DEMO_ARRIVAL). It speaks the little of HTTP/1.1 the fetcher needs: `GET` with an
//! optional `Range: bytes=a-b` or `a-`, answered by a 200, a 206 with its
//! `Content-Range`, a 416 past the end or a 404, on connections kept open.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A request head is not read past this (nothing the fetcher sends comes
/// close).
const MAX_HEAD: usize = 16 * 1024;

/// The file of an asset: its kind (`texture`, `mesh`, `animatn`, the name
/// of the request's `<kind>_id=` parameter) and its id. None: a 404.
pub type Resolve = Arc<dyn Fn(&str, &Uuid) -> Option<PathBuf> + Send + Sync>;

/// The link the server stands for: each reply leaves `delay` after its
/// request, and with a bandwidth the bodies of all the connections share
/// it, one after the other, as on a link held by its throughput. Counts the
/// bytes sent, per kind of asset.
pub struct Link {
    delay: Duration,
    bytes_per_second: Option<f64>,
    /// When the shared pipe is free again.
    free_at: Mutex<Instant>,
    sent: [AtomicU64; 4],
}

impl Link {
    /// `mbps`: megabits a second (None: no limit but the round trip).
    pub fn new(delay: Duration, mbps: Option<f64>) -> Arc<Link> {
        Arc::new(Link {
            delay,
            bytes_per_second: mbps.filter(|m| *m > 0.0).map(|m| m * 1e6 / 8.0),
            free_at: Mutex::new(Instant::now()),
            sent: Default::default(),
        })
    }

    /// Bytes of bodies sent for a kind (`texture`, `mesh`, `animatn`).
    pub fn sent(&self, kind: &str) -> u64 {
        KINDS
            .iter()
            .position(|k| *k == kind)
            .map_or(0, |i| self.sent[i].load(Ordering::Relaxed))
    }

    /// Wait for the body's turn through the pipe, and its time in it.
    fn pass(&self, kind: Option<&str>, len: usize) {
        if let Some(i) = kind.and_then(|kind| KINDS.iter().position(|k| *k == kind)) {
            self.sent[i].fetch_add(len as u64, Ordering::Relaxed);
        }
        let Some(rate) = self.bytes_per_second else {
            return;
        };
        let done = {
            let mut free_at = self.free_at.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let start = (*free_at).max(Instant::now());
            *free_at = start + Duration::from_secs_f64(len as f64 / rate);
            *free_at
        };
        std::thread::sleep(done.saturating_duration_since(Instant::now()));
    }
}

/// Starts the server for `files`: texture `id` is the file `index(id)`
/// (cycled). Returns its base URL, the `ViewerAsset` capability of the
/// demo's region.
pub fn start(files: Arc<Vec<PathBuf>>, delay: Duration, index: fn(&Uuid) -> Option<u32>) -> std::io::Result<String> {
    start_assets(
        Arc::new(move |kind, id| {
            let i = index(id).filter(|_| kind == "texture")?;
            files.get(i as usize % files.len().max(1)).cloned()
        }),
        Link::new(delay, None),
    )
}

/// Starts the server for any kind of asset, found by `resolve`.
pub fn start_assets(resolve: Resolve, link: Arc<Link>) -> std::io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/", listener.local_addr()?);
    std::thread::Builder::new().name("demo-assets".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (resolve, link) = (resolve.clone(), link.clone());
            let spawned = std::thread::Builder::new()
                .name("demo-assets-connection".into())
                .spawn(move || serve(stream, &resolve, &link));
            if let Err(e) = spawned {
                log::warn!("demo asset server: connection not served ({e})");
            }
        }
    })?;
    Ok(url)
}

/// One connection: requests answered one after the other until it closes.
fn serve(stream: TcpStream, resolve: &Resolve, link: &Link) {
    let _ = stream.set_nodelay(true);
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let mut input = BufReader::new(stream);
    loop {
        let mut head = String::new();
        loop {
            let before = head.len();
            match input.read_line(&mut head) {
                Ok(0) | Err(_) => return,
                Ok(_) if head.len() > MAX_HEAD => return,
                // (the empty line that ends the head)
                Ok(_) if head.len() - before <= 2 => break,
                Ok(_) => {}
            }
        }
        std::thread::sleep(link.delay);
        let asked = parse_request(&head);
        let file = asked.and_then(|(kind, id, range)| Some((std::fs::read(resolve(kind, &id)?).ok()?, range)));
        let (status, headers, body) = match &file {
            Some((data, range)) => reply(data, *range),
            None => ("404 Not Found", String::new(), &[][..]),
        };
        link.pass(asked.map(|(kind, _, _)| kind), body.len());
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: image/x-j2c\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{headers}\r\n",
            body.len()
        );
        if out.write_all(head.as_bytes()).and_then(|()| out.write_all(body)).is_err() {
            return;
        }
    }
}

/// A byte range as asked: first byte, and the last one when the range is
/// not open.
type Range = (u64, Option<u64>);

/// Kinds of asset served, as the `<kind>_id=` parameters of the
/// ViewerAsset capability name them.
const KINDS: [&str; 4] = ["texture", "mesh", "animatn", "material"];

/// The asset asked for (kind, id) and its range, from the head of a request
/// (`GET /?texture_id=<id> HTTP/1.1` and its header lines).
fn parse_request(head: &str) -> Option<(&'static str, Uuid, Option<Range>)> {
    let mut lines = head.lines();
    let target = lines.next()?.strip_prefix("GET ")?.split(' ').next()?;
    let (kind, id) = target.split(['?', '&']).find_map(|p| {
        let (name, value) = p.split_once('=')?;
        let kind = KINDS.into_iter().find(|k| name.strip_suffix("_id") == Some(*k))?;
        Some((kind, value))
    })?;
    let id = Uuid::parse_str(id).ok()?;
    let range = lines.find_map(|l| {
        let (name, value) = l.split_once(':')?;
        if !name.eq_ignore_ascii_case("range") {
            return None;
        }
        let (first, last) = value.trim().strip_prefix("bytes=")?.split_once('-')?;
        Some((first.parse().ok()?, last.parse().ok()))
    });
    Some((kind, id, range))
}

/// Status, extra header lines and body for `data` asked with `range`.
fn reply(data: &[u8], range: Option<Range>) -> (&'static str, String, &[u8]) {
    let total = data.len() as u64;
    let Some((first, last)) = range else {
        return ("200 OK", String::new(), data);
    };
    if first >= total {
        return ("416 Range Not Satisfiable", format!("Content-Range: bytes */{total}\r\n"), &[]);
    }
    let last = last.map_or(total - 1, |l| l.min(total - 1));
    (
        "206 Partial Content",
        format!("Content-Range: bytes {first}-{last}/{total}\r\n"),
        &data[first as usize..=last as usize],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn requests_name_a_texture_and_a_range() {
        let id = Uuid::from_u128(7);
        let head = format!("GET /?texture_id={id} HTTP/1.1\r\nhost: x\r\nRange: bytes=0-16383\r\n\r\n");
        assert_eq!(parse_request(&head), Some(("texture", id, Some((0, Some(16383))))));
        let head = format!("GET /?texture_id={id} HTTP/1.1\r\nrange: bytes=599-\r\n\r\n");
        assert_eq!(parse_request(&head), Some(("texture", id, Some((599, None)))));
        let head = format!("GET /?texture_id={id} HTTP/1.1\r\n\r\n");
        assert_eq!(parse_request(&head), Some(("texture", id, None)));
        let head = format!("GET /?mesh_id={id} HTTP/1.1\r\n\r\n");
        assert_eq!(parse_request(&head), Some(("mesh", id, None)));
        let head = format!("GET /?animatn_id={id} HTTP/1.1\r\n\r\n");
        assert_eq!(parse_request(&head), Some(("animatn", id, None)));
        assert_eq!(parse_request("GET /?mesh_id=1 HTTP/1.1\r\n\r\n"), None);
        let head = format!("GET /?material_id={id} HTTP/1.1\r\n\r\n");
        assert_eq!(parse_request(&head), Some(("material", id, None)));
        assert_eq!(parse_request(&format!("GET /?sound_id={id} HTTP/1.1\r\n\r\n")), None);
        assert_eq!(parse_request("POST / HTTP/1.1\r\n\r\n"), None);
    }

    #[test]
    fn ranges_are_cut_to_the_file() {
        let data = [1u8, 2, 3, 4, 5];
        assert_eq!(reply(&data, None), ("200 OK", String::new(), &data[..]));
        let (status, headers, body) = reply(&data, Some((1, Some(2))));
        assert_eq!(
            (status, headers.as_str(), body),
            ("206 Partial Content", "Content-Range: bytes 1-2/5\r\n", &[2u8, 3][..])
        );
        // longer than the file, or open: up to its end
        assert_eq!(reply(&data, Some((3, Some(99)))).2, &[4, 5]);
        assert_eq!(reply(&data, Some((4, None))).1, "Content-Range: bytes 4-4/5\r\n");
        // past the end
        let (status, headers, body) = reply(&data, Some((5, None)));
        assert_eq!(
            (status, headers.as_str(), body.len()),
            ("416 Range Not Satisfiable", "Content-Range: bytes */5\r\n", 0)
        );
    }

    #[test]
    fn a_connection_serves_several_requests() {
        let dir = std::env::temp_dir().join(format!("aurora-demo-assets-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join("a.j2c");
        std::fs::write(&path, b"0123456789").expect("file");
        let url = start(Arc::new(vec![path]), Duration::ZERO, |id| Some(id.as_u128() as u32)).expect("server");
        let addr = url.trim_start_matches("http://").trim_end_matches('/');
        let mut stream = TcpStream::connect(addr).expect("connection");
        let mut ask = |range: &str| {
            let request = format!("GET /?texture_id={} HTTP/1.1\r\nRange: bytes={range}\r\n\r\n", Uuid::from_u128(3));
            stream.write_all(request.as_bytes()).expect("request");
            let mut answer = Vec::new();
            let mut byte = [0u8; 1];
            while !answer.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).expect("head");
                answer.push(byte[0]);
            }
            let head = String::from_utf8(answer).expect("text");
            let length: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("Content-Length: "))
                .and_then(|v| v.parse().ok())
                .expect("length");
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).expect("body");
            (head, body)
        };
        let (head, body) = ask("0-3");
        assert!(head.starts_with("HTTP/1.1 206"), "{head}");
        assert_eq!(body, b"0123");
        let (head, body) = ask("3-");
        assert!(head.contains("Content-Range: bytes 3-9/10"), "{head}");
        assert_eq!(body, b"3456789");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
