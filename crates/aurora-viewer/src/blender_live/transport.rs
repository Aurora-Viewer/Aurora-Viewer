use super::*;
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

/// The add-on's greeting, followed by the session's secret (`token.rs`).
const MAGIC: &[u8; 8] = b"AURBLD2\n";
/// The greeting of add-ons that send no secret: refused, with a reply that
/// makes them say "Incompatible Aurora protocol" instead of retrying silently.
const OLD_MAGIC: &[u8; 8] = b"AURBLD1\n";

pub(super) fn start(
    port: u16,
    secret: String,
    supported: HashSet<String>,
    rest_axes: std::collections::HashMap<String, [f32; 16]>,
    ctx: egui::Context,
    pool: Option<Arc<aurora_render::StagingPool>>,
) -> io::Result<Arc<Shared>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    start_listener(listener, secret, supported, rest_axes, ctx, pool)
}

pub(super) fn start_listener(
    listener: TcpListener,
    secret: String,
    supported: HashSet<String>,
    rest_axes: std::collections::HashMap<String, [f32; 16]>,
    ctx: egui::Context,
    pool: Option<Arc<aurora_render::StagingPool>>,
) -> io::Result<Arc<Shared>> {
    listener.set_nonblocking(true)?;
    let shared = Arc::new(Shared::default());
    let worker = shared.clone();
    std::thread::Builder::new().name("blender-live".into()).spawn(move || {
        while !worker.stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Err(error) = serve(&mut stream, &secret, &worker, &supported, &rest_axes, &ctx, pool.as_deref())
                        && !worker.stop.load(Ordering::Acquire)
                        && !matches!(
                            error.kind(),
                            io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                        )
                    {
                        worker.status.lock().error = Some(error.to_string());
                    }
                    worker.status.lock().connected = false;
                    worker.status.lock().shape_capable = false;
                    worker.status.lock().shape_apply_capable = false;
                    worker.status.lock().import_capable = false;
                    worker.import_request.lock().take();
                    worker.import_expected.lock().take();
                    worker.shape.lock().take();
                    worker.shape_ack.lock().take();
                    worker.shape_cancel.lock().take();
                    ctx.request_repaint();
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(30)),
                Err(e) => {
                    worker.status.lock().error = Some(e.to_string());
                    ctx.request_repaint();
                    break;
                }
            }
        }
    })?;
    Ok(shared)
}

fn read(stream: &mut TcpStream, mut target: &mut [u8], shared: &Shared, idle: Duration) -> io::Result<()> {
    let mut last = Instant::now();
    while !target.is_empty() {
        if shared.stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        match stream.read(target) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                target = &mut target[n..];
                last = Instant::now();
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) && last.elapsed() < idle => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn serve(
    stream: &mut TcpStream,
    secret: &str,
    shared: &Shared,
    supported: &HashSet<String>,
    rest_axes: &std::collections::HashMap<String, [f32; 16]>,
    ctx: &egui::Context,
    pool: Option<&aurora_render::StagingPool>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(150)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_nodelay(true)?;
    let mut magic = [0; 8];
    read(stream, &mut magic, shared, Duration::from_secs(3))?;
    if &magic == OLD_MAGIC {
        let _ = writeln!(stream, "{}", serde_json::json!({ "protocol": protocol::VERSION + 1 }));
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            aurora_i18n::tr!("blender-live-addon-outdated").to_string(),
        ));
    }
    if &magic != MAGIC {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut received = [0; token::LEN];
    read(stream, &mut received, shared, Duration::from_secs(3))?;
    if !token::matches(&received, secret) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            aurora_i18n::tr!("livemesh-token-refused").to_string(),
        ));
    }
    let mut joints: Vec<_> = supported.iter().collect();
    joints.sort();
    let hello = serde_json::json!({ "protocol": protocol::VERSION, "joints": joints, "rest_axes": rest_axes, "max_bytes": protocol::MAX_BYTES, "max_decoded_bytes": protocol::MAX_DECODED_BYTES, "features": ["pbr-materials", "source-ids", "shape-transfer", "shape-apply", "import-lods", "static-meshes", "object-links", "compressed-snapshots", "packed-geometry", "object-warnings", "compressed-lods"] });
    writeln!(stream, "{hello}")?;
    {
        let mut status = shared.status.lock();
        status.connected = true;
        status.shape_capable = false;
        status.shape_apply_capable = false;
        status.import_capable = false;
        status.error = None;
    }
    ctx.request_repaint();
    let mut previous = None;
    let mut images = std::collections::HashMap::new();
    while !shared.stop.load(Ordering::Acquire) {
        let mut len = [0; 4];
        // A zero-length heartbeat lets a quiet Blender retain the connection.
        read(stream, &mut len, shared, Duration::from_secs(10))?;
        let len = u32::from_le_bytes(len) as usize;
        if len == 0 {
            reply(stream, shared, serde_json::json!({"ok": true}))?;
            continue;
        }
        if len > protocol::MAX_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut bytes = vec![0; len];
        read(stream, &mut bytes, shared, Duration::from_secs(5))?;
        if bytes.starts_with(b"LODS") {
            let expected = *shared.import_expected.lock();
            // The import that waits gets its result, unless another took
            // its place meanwhile.
            let finish = |expected: super::import::Request, result: Result<super::import::Bundle, String>| {
                let mut pending = shared.import_expected.lock();
                if *pending == Some(expected) {
                    pending.take();
                    *shared.import_result.lock() = Some((expected.id, result));
                }
            };
            // "compressed-lods": the prepared LODs may come deflated, as
            // snapshots do; four LOD levels of JSON outgrow MAX_BYTES first.
            // Inflated only while an import waits: with none pending, a
            // frame is dropped without spending up to MAX_DECODED_BYTES on
            // it. One for an abandoned request that comes while another
            // import waits is inflated, then dropped on its id.
            let mut answer = serde_json::json!({"ok": true});
            if let Some(expected) = expected {
                match protocol::lods_body(&bytes[4..]) {
                    // (truncated, or over MAX_DECODED_BYTES once inflated):
                    // the import fails with the cause instead of waiting out
                    // its time, and the plug-in is told, as for a snapshot
                    Err(error) => {
                        answer = serde_json::json!({"ok": false, "error": error});
                        finish(expected, Err(error));
                    }
                    Ok(body) if serde_json::from_slice::<super::import::Request>(&body).is_ok_and(|request| request == expected) => {
                        finish(expected, super::import::decode(&body, expected, supported));
                    }
                    Ok(_) => {}
                }
            }
            reply(stream, shared, answer)?;
            ctx.request_repaint();
            continue;
        }
        if len <= 4096 && control(&bytes, shared) {
            reply(stream, shared, serde_json::json!({"ok": true}))?;
            ctx.request_repaint();
            continue;
        }
        let result = protocol::prepare_staged(&bytes, supported, pool, &images).and_then(|prepared| {
            if previous.is_some_and(|old| prepared.revision <= old) {
                return Err(aurora_i18n::tr!("blender-live-invalid-revision").into());
            }
            Ok(prepared)
        });
        let reply = match result {
            Ok(prepared) => {
                let revision = prepared.revision;
                previous = Some(revision);
                images = prepared.images.iter().map(|image| (image.id, image.clone())).collect();
                *shared.latest.lock() = Some(prepared);
                let mut status = shared.status.lock();
                status.received += 1;
                status.error = None;
                serde_json::json!({ "ok": true, "revision": revision })
            }
            Err(error) => {
                let retry = error == protocol::STAGING_BUSY;
                let error = if retry {
                    aurora_i18n::tr!("blender-live-material-busy").to_owned()
                } else {
                    error
                };
                shared.status.lock().error = Some(error.clone());
                serde_json::json!({ "ok": false, "error": error, "retry": retry })
            }
        };
        self::reply(stream, shared, reply)?;
        ctx.request_repaint();
    }
    Ok(())
}

fn control(bytes: &[u8], shared: &Shared) -> bool {
    #[derive(serde::Deserialize)]
    struct Control {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        features: Vec<String>,
        id: Option<uuid::Uuid>,
        error: Option<String>,
    }
    let Ok(control) = serde_json::from_slice::<Control>(bytes) else {
        return false;
    };
    match control.kind.as_str() {
        "features" => {
            let mut status = shared.status.lock();
            status.shape_capable = control.features.iter().any(|f| f == "shape-transfer");
            status.shape_apply_capable = status.shape_capable && control.features.iter().any(|f| f == "shape-apply");
            status.import_capable = control.features.iter().any(|f| f == "import-lods");
        }
        "shape-result" => {
            if let Some(id) = control.id {
                *shared.shape_ack.lock() = Some((id, control.error.map(|message| message.chars().take(512).collect())));
            }
        }
        _ => return false,
    }
    true
}

fn reply(stream: &mut TcpStream, shared: &Shared, mut reply: serde_json::Value) -> io::Result<()> {
    let status = shared.status.lock();
    let capable = status.shape_capable;
    let apply_capable = status.shape_apply_capable;
    let import_capable = status.import_capable;
    drop(status);
    if import_capable && let Some(request) = shared.import_request.lock().take() {
        reply["prepare_import"] = serde_json::json!(request);
    }
    let shape = if capable {
        let mut queued = shared.shape.lock();
        if queued.as_ref().is_some_and(|shape| shape.data["action"] == "apply") && !apply_capable {
            None
        } else {
            queued.take()
        }
    } else {
        None
    };
    if let Some(shape) = shape {
        reply["shape"] = (*shape.data).clone();
        reply["shape"]["id"] = serde_json::json!(shape.id);
    }
    if capable && let Some(id) = shared.shape_cancel.lock().take() {
        reply["shape_cancel"] = serde_json::json!(id);
    }
    writeln!(stream, "{reply}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    fn server(secret: &str) -> (Arc<Shared>, u16) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
        let supported = ["mPelvis".to_owned()].into_iter().collect();
        let shared = start_listener(
            listener,
            secret.to_owned(),
            supported,
            Default::default(),
            egui::Context::default(),
            None,
        )
        .expect("start");
        (shared, port)
    }

    /// The viewer's first line after `greeting`; empty when it closes without one.
    fn first_line(port: u16, greeting: &[u8]) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        stream.write_all(greeting).expect("greeting");
        let mut line = String::new();
        // A reset counts as a close: the viewer may drop unread bytes.
        let _ = io::BufReader::new(stream).read_line(&mut line);
        line
    }

    /// All the viewer sends after `greeting` until it closes the connection.
    /// The worker records its error before it drops the stream, so once this
    /// returns the status holds it (reading only the reply raced with it).
    fn refused(port: u16, greeting: &[u8]) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        stream.write_all(greeting).expect("greeting");
        let mut reply = Vec::new();
        // A reset counts as a close: the viewer may drop unread bytes.
        let _ = stream.read_to_end(&mut reply);
        String::from_utf8_lossy(&reply).into_owned()
    }

    #[test]
    fn only_an_add_on_with_the_session_secret_is_greeted() {
        let secret = token::generate();
        let (shared, port) = server(&secret);

        let hello = first_line(port, &[MAGIC.as_slice(), secret.as_bytes()].concat());
        assert!(hello.contains("\"joints\":[\"mPelvis\"]"), "{hello}");

        let other = token::generate();
        assert_eq!(refused(port, &[MAGIC.as_slice(), other.as_bytes()].concat()), "");
        assert_eq!(
            shared.status.lock().error.as_deref(),
            Some(aurora_i18n::tr!("livemesh-token-refused").as_ref())
        );

        assert_eq!(refused(port, b"GET / HTTP/1.1\r\n\r\n"), "");

        let old = refused(port, OLD_MAGIC);
        let reply: serde_json::Value = serde_json::from_str(&old).expect("JSON reply");
        assert_eq!(
            reply["protocol"],
            protocol::VERSION + 1,
            "an old add-on reads an incompatible protocol"
        );
        assert!(reply.get("joints").is_none());
        assert_eq!(
            shared.status.lock().error.as_deref(),
            Some(aurora_i18n::tr!("blender-live-addon-outdated").as_ref())
        );

        let hello = first_line(port, &[MAGIC.as_slice(), secret.as_bytes()].concat());
        assert!(hello.contains("\"joints\""), "the secret still works after refusals: {hello}");
        shared.stop.store(true, Ordering::Release);
    }
}
