//! Minimal blocking HTTP/1.1 client for internet radio.
//!
//! A dedicated client is used instead of a general-purpose one because
//! SHOUTcast v1 servers answer with a non-HTTP status line (`ICY 200 OK`),
//! which strict HTTP parsers reject. Supports http/https, redirects and
//! chunked transfer encoding; every blocking read honours a stop flag.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use url::{Host, Url};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Socket read timeout; bounds how long a stop request can take to be seen.
const READ_POLL: Duration = Duration::from_millis(250);
/// No audio bytes for this long means the stream is dead.
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 5;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const USER_AGENT: &str = concat!("AuroraViewer/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, thiserror::Error)]
pub(crate) enum HttpError {
    #[error("invalid stream URL: {0}")]
    Url(String),
    #[error("unsupported URL scheme `{0}`")]
    Scheme(String),
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("malformed HTTP response")]
    Malformed,
    #[error("HTTP status {0}")]
    Status(u16),
    #[error("too many redirects")]
    Redirects,
    #[error("stopped")]
    Stopped,
}

/// Body + interesting headers of a successful response.
pub(crate) struct HttpResponse {
    pub content_type: Option<String>,
    pub metaint: Option<usize>,
    pub body: Box<dyn Read + Send + Sync>,
}

/// Parsed status line + headers.
#[derive(Debug, PartialEq)]
pub(crate) struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Parses `HTTP/1.x 200 OK` or `ICY 200 OK`.
pub(crate) fn parse_status_line(line: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    let proto = parts.next()?;
    if !(proto.starts_with("HTTP/") || proto == "ICY") {
        return None;
    }
    let code = parts.next()?;
    if code.len() != 3 {
        return None;
    }
    code.parse().ok()
}

/// Parses response header lines (status line first, without the blank line).
pub(crate) fn parse_head(lines: &[String]) -> Option<Head> {
    let (first, rest) = lines.split_first()?;
    let status = parse_status_line(first)?;
    let headers = rest
        .iter()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    Some(Head { status, headers })
}

enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

/// Retries socket read timeouts while checking the stop flag, and fails if
/// the stream stalls.
struct StopAware<R> {
    inner: R,
    stop: Arc<AtomicBool>,
    last_data: Instant,
}

impl<R: Read> Read for StopAware<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(io::Error::other("stopped"));
            }
            match self.inner.read(buf) {
                Ok(n) => {
                    self.last_data = Instant::now();
                    return Ok(n);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                    ) =>
                {
                    if self.last_data.elapsed() > STALL_TIMEOUT {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "stream stalled"));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Decoder for `Transfer-Encoding: chunked`.
pub(crate) struct ChunkedReader<R> {
    inner: R,
    remaining: usize,
    done: bool,
}

impl<R: BufRead> ChunkedReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            remaining: 0,
            done: false,
        }
    }

    fn read_line(&mut self) -> io::Result<String> {
        let mut line = Vec::new();
        let n = (&mut self.inner).take(4096).read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(String::from_utf8_lossy(&line).trim().to_string())
    }
}

impl<R: BufRead> Read for ChunkedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut line = self.read_line()?;
            if line.is_empty() {
                // CRLF terminating the previous chunk.
                line = self.read_line()?;
            }
            let size_str = line.split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size_str, 16).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
            if size == 0 {
                self.done = true;
                return Ok(0);
            }
            self.remaining = size;
        }
        let n = buf.len().min(self.remaining);
        let got = self.inner.read(&mut buf[..n])?;
        if got == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= got;
        Ok(got)
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, HttpError> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
            // Explicit provider: the workspace may enable several rustls
            // crypto backends, which makes the implicit default ambiguous.
            let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()))
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(HttpError::Tls)
}

fn connect(url: &Url) -> Result<Conn, HttpError> {
    let https = match url.scheme() {
        "http" | "icy" => false,
        "https" => true,
        other => return Err(HttpError::Scheme(other.to_string())),
    };
    let port = url.port_or_known_default().unwrap_or(if https { 443 } else { 80 });
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(Host::Domain(d)) => (d, port)
            .to_socket_addrs()
            .map_err(|e| HttpError::Connect(e.to_string()))?
            .collect(),
        Some(Host::Ipv4(ip)) => vec![SocketAddr::from((ip, port))],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::from((ip, port))],
        None => return Err(HttpError::Url("missing host".into())),
    };
    let mut last_err = String::from("no address");
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let tcp = tcp.ok_or(HttpError::Connect(last_err))?;
    tcp.set_read_timeout(Some(READ_POLL))?;
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let _ = tcp.set_nodelay(true);
    if !https {
        return Ok(Conn::Plain(tcp));
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let name = rustls::pki_types::ServerName::try_from(host).map_err(|e| HttpError::Tls(e.to_string()))?;
    let conn = rustls::ClientConnection::new(tls_config()?, name).map_err(|e| HttpError::Tls(e.to_string()))?;
    Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}

fn request_target(url: &Url) -> String {
    let mut t = url.path().to_string();
    if t.is_empty() {
        t.push('/');
    }
    if let Some(q) = url.query() {
        t.push('?');
        t.push_str(q);
    }
    t
}

fn host_header(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    }
}

fn read_head<R: BufRead>(r: &mut R) -> Result<Head, HttpError> {
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let n = r.read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(HttpError::Malformed);
        }
        total += n;
        if total > MAX_HEADER_BYTES {
            return Err(HttpError::Malformed);
        }
        let text = String::from_utf8_lossy(&line).trim_end().to_string();
        if text.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(text);
    }
    parse_head(&lines).ok_or(HttpError::Malformed)
}

/// Opens `url` for streaming, following redirects. Sends `Icy-MetaData: 1`.
pub(crate) fn open(url: &str, stop: &Arc<AtomicBool>) -> Result<HttpResponse, HttpError> {
    let mut url = Url::parse(url.trim()).map_err(|e| HttpError::Url(e.to_string()))?;
    for _ in 0..=MAX_REDIRECTS {
        if stop.load(Ordering::Relaxed) {
            return Err(HttpError::Stopped);
        }
        let mut conn = connect(&url)?;
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: */*\r\nIcy-MetaData: 1\r\nConnection: close\r\n\r\n",
            request_target(&url),
            host_header(&url),
            USER_AGENT
        );
        conn.write_all(req.as_bytes())?;
        conn.flush()?;
        let mut reader = BufReader::with_capacity(
            16 * 1024,
            StopAware {
                inner: conn,
                stop: stop.clone(),
                last_data: Instant::now(),
            },
        );
        let head = read_head(&mut reader)?;
        match head.status {
            200..=299 => {
                let content_type = head
                    .header("content-type")
                    .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
                let metaint = head.header("icy-metaint").and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0);
                let chunked = head
                    .header("transfer-encoding")
                    .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
                let body: Box<dyn Read + Send + Sync> = if chunked {
                    Box::new(ChunkedReader::new(reader))
                } else {
                    Box::new(reader)
                };
                return Ok(HttpResponse {
                    content_type,
                    metaint,
                    body,
                });
            }
            301 | 302 | 303 | 307 | 308 => {
                let loc = head.header("location").ok_or(HttpError::Malformed)?;
                url = url.join(loc).map_err(|e| HttpError::Url(e.to_string()))?;
                log::debug!("stream redirected to {url}");
            }
            other => return Err(HttpError::Status(other)),
        }
    }
    Err(HttpError::Redirects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        assert_eq!(parse_status_line("ICY 200 OK"), Some(200));
        assert_eq!(parse_status_line("HTTP/1.0 200 OK"), Some(200));
        assert_eq!(parse_status_line("HTTP/1.1 302 Found"), Some(302));
        assert_eq!(parse_status_line("HTTP/1.1 404"), Some(404));
        assert_eq!(parse_status_line("FOO 200 OK"), None);
        assert_eq!(parse_status_line("HTTP/1.1 2000 OK"), None);
        assert_eq!(parse_status_line(""), None);
    }

    #[test]
    fn heads() {
        let lines: Vec<String> = [
            "ICY 200 OK",
            "icy-name: Test Radio",
            "Content-Type: audio/mpeg",
            "icy-metaint:16000",
            "garbage line",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let head = parse_head(&lines).unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-type"), Some("audio/mpeg"));
        assert_eq!(head.header("ICY-METAINT"), Some("16000"));
        assert_eq!(head.header("icy-name"), Some("Test Radio"));
        assert_eq!(head.headers.len(), 3);
    }

    #[test]
    fn read_head_from_bytes() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: audio/ogg\r\n\r\nBODY";
        let mut r = BufReader::new(&raw[..]);
        let head = read_head(&mut r).unwrap();
        assert_eq!(head.status, 200);
        let mut body = String::new();
        r.read_to_string(&mut body).unwrap();
        assert_eq!(body, "BODY");
    }

    #[test]
    fn chunked_decoding() {
        let raw = b"5\r\nhello\r\n7;ext=1\r\n, world\r\n0\r\n\r\n";
        let mut r = ChunkedReader::new(BufReader::new(&raw[..]));
        let mut out = String::new();
        r.read_to_string(&mut out).unwrap();
        assert_eq!(out, "hello, world");

        let bad = b"zz\r\nhello\r\n";
        let mut r = ChunkedReader::new(BufReader::new(&bad[..]));
        let mut out = Vec::new();
        assert!(r.read_to_end(&mut out).is_err());
    }

    #[test]
    fn request_parts() {
        let u = Url::parse("http://radio.example:8000/stream?x=1").unwrap();
        assert_eq!(request_target(&u), "/stream?x=1");
        assert_eq!(host_header(&u), "radio.example:8000");
        let u = Url::parse("https://radio.example").unwrap();
        assert_eq!(request_target(&u), "/");
        assert_eq!(host_header(&u), "radio.example");
    }

    #[test]
    fn rejects_unknown_scheme() {
        let stop = Arc::new(AtomicBool::new(false));
        assert!(matches!(open("ftp://example/x", &stop), Err(HttpError::Scheme(_))));
        assert!(matches!(open("not a url", &stop), Err(HttpError::Url(_))));
    }
}
