//! The web profile of the demo (and of the tests): a local server that
//! answers what my.secondlife.com and its storage answer to a photo post —
//! the session cookie for the OpenID cookie, `snapshots/s3_upload_config`,
//! the form's POST with its 303, and the address it leads to. It checks
//! what Firestorm sends (cookies, the fields and their order) and keeps
//! each post for the log and the tests.

use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// The cookies of the demo: what the login's OpenID gives, what the
/// profile gives back.
pub const OPENID: &str = "demo-openid=1";
const AUTH: &str = "_my_session=demo-auth";
/// Fields of the storage form, in buildPostData's order.
pub const FIELDS: [&str; 8] = [
    "key",
    "AWSAccessKeyId",
    "acl",
    "Content-Type",
    "policy",
    "signature",
    "success_action_redirect",
    "file",
];
const MAX_BODY: usize = 64 * 1024 * 1024;

/// A photo the server received.
#[derive(Debug, Clone, Default)]
pub struct Posted {
    pub caption: String,
    pub add_location: bool,
    /// The form's field names, in their order.
    pub fields: Vec<String>,
    pub png: Vec<u8>,
    /// The Cookie headers of the form's POST.
    pub cookies: Vec<String>,
    /// The 303's address was asked.
    pub finished: bool,
}

pub struct Server {
    /// The profile's address ("http://127.0.0.1:<port>/").
    pub url: String,
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the tests"))]
    pub posts: Arc<Mutex<Vec<Posted>>>,
}

/// Start the server; `fail`: the upload configuration is refused (500).
pub fn start(fail: bool) -> std::io::Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/", listener.local_addr()?);
    let posts: Arc<Mutex<Vec<Posted>>> = Arc::default();
    let (base, kept) = (url.clone(), posts.clone());
    std::thread::Builder::new().name("demo-profile".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (base, kept) = (base.clone(), kept.clone());
            let _ = std::thread::Builder::new()
                .name("demo-profile-connection".into())
                .spawn(move || serve(stream, &base, &kept, fail));
        }
    })?;
    Ok(Server { url, posts })
}

struct Request {
    method: String,
    target: String,
    cookies: Vec<String>,
    content_type: String,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut input = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    input.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?.to_owned(), parts.next()?.to_owned());
    let (mut length, mut cookies, mut content_type) = (0usize, Vec::new(), String::new());
    loop {
        let mut header = String::new();
        if input.read_line(&mut header).ok()? == 0 || header.trim().is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok()?,
            "cookie" => cookies.push(value),
            "content-type" => content_type = value,
            _ => {}
        }
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = vec![0; length];
    input.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        target,
        cookies,
        content_type,
        body,
    })
}

fn serve(mut stream: TcpStream, base: &str, posts: &Mutex<Vec<Posted>>, fail: bool) {
    let Some(r) = read_request(&stream) else {
        return;
    };
    let has = |cookie: &str| r.cookies.iter().any(|c| c.contains(cookie));
    let (path, query) = r.target.split_once('?').unwrap_or((r.target.as_str(), ""));
    let reply = match (r.method.as_str(), path) {
        // getOpenIDCookieCoro: the profile with the OpenID cookie
        ("GET", "/") if has(OPENID) => ("200 OK", format!("Set-Cookie: {AUTH}; path=/; HttpOnly\r\n"), b"profile".to_vec()),
        ("GET", "/snapshots/s3_upload_config") if fail => ("500 Internal Server Error", String::new(), Vec::new()),
        ("GET", "/snapshots/s3_upload_config") if has(OPENID) && has(AUTH) => {
            let value = |name: &str| {
                query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
                    .map(unescape)
                    .unwrap_or_default()
            };
            posts.lock().push(Posted {
                caption: value("caption"),
                add_location: value("add_loc") == "1",
                ..Default::default()
            });
            let config = serde_json::json!({
                "url": format!("{base}s3"),
                "data": {
                    "key": "snapshots/demo/${filename}",
                    "AWSAccessKeyId": "DEMO",
                    "acl": "public-read",
                    "Content-Type": "image/png",
                    "policy": "demo-policy",
                    "signature": "demo-signature",
                    "success_action_redirect": format!("{base}snapshots/done"),
                }
            });
            (
                "200 OK",
                "Content-Type: application/json\r\n".into(),
                config.to_string().into_bytes(),
            )
        }
        // the storage: the form, answered by a 303 to success_action_redirect
        ("POST", "/s3") if r.content_type.contains("boundary=") => {
            let (fields, png) = parse_form(&r.body, &r.content_type);
            let ok = fields == FIELDS && !png.is_empty();
            if let Some(last) = posts.lock().last_mut() {
                last.fields = fields;
                last.png = png;
                last.cookies = r.cookies.clone();
            }
            if ok {
                ("303 See Other", format!("Location: {base}snapshots/done?key=demo\r\n"), Vec::new())
            } else {
                ("400 Bad Request", String::new(), Vec::new())
            }
        }
        ("GET", "/snapshots/done") if has(AUTH) => {
            if let Some(last) = posts.lock().last_mut() {
                last.finished = true;
                log::info!(
                    "demo profile: photo posted ({} KB PNG, caption \"{}\", location {})",
                    last.png.len() >> 10,
                    last.caption,
                    last.add_location
                );
            }
            ("200 OK", String::new(), b"posted".to_vec())
        }
        _ => ("403 Forbidden", String::new(), Vec::new()),
    };
    let (status, headers, body) = reply;
    let head = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
}

/// The field names of a multipart form and its file's bytes.
fn parse_form(body: &[u8], content_type: &str) -> (Vec<String>, Vec<u8>) {
    let Some(boundary) = content_type.split("boundary=").nth(1) else {
        return (Vec::new(), Vec::new());
    };
    let delimiter = format!("--{}", boundary.trim());
    let mut fields = Vec::new();
    let mut file = Vec::new();
    let mut rest = body;
    while let Some(start) = find(rest, delimiter.as_bytes()) {
        rest = &rest[start + delimiter.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        let Some(head_end) = find(rest, b"\r\n\r\n") else {
            break;
        };
        let head = String::from_utf8_lossy(&rest[..head_end]).into_owned();
        let content = &rest[head_end + 4..];
        let end = find(content, delimiter.as_bytes()).unwrap_or(content.len());
        let value = content[..end].strip_suffix(b"\r\n").unwrap_or(&content[..end]);
        if let Some(name) = head.split("name=\"").nth(1).and_then(|n| n.split('"').next()) {
            if name == "file" {
                file = value.to_vec();
            }
            fields.push(name.to_owned());
        }
        rest = content;
    }
    (fields, file)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
