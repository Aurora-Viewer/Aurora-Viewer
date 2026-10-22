use super::*;
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

const MAGIC: &[u8; 8] = b"AURBLD1\n";

pub(super) fn start(
    port: u16,
    supported: HashSet<String>,
    rest_axes: std::collections::HashMap<String, [f32; 16]>,
    ctx: egui::Context,
    pool: Option<Arc<aurora_render::StagingPool>>,
) -> io::Result<Arc<Shared>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    start_listener(listener, supported, rest_axes, ctx, pool)
}

pub(super) fn start_listener(
    listener: TcpListener,
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
                    if let Err(error) = serve(&mut stream, &worker, &supported, &rest_axes, &ctx, pool.as_deref())
                        && !worker.stop.load(Ordering::Acquire)
                        && !matches!(
                            error.kind(),
                            io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                        )
                    {
                        worker.status.lock().error = Some(error.to_string());
                    }
                    worker.status.lock().connected = false;
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
    if &magic != MAGIC {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut joints: Vec<_> = supported.iter().collect();
    joints.sort();
    let hello = serde_json::json!({ "protocol": protocol::VERSION, "joints": joints, "rest_axes": rest_axes, "max_bytes": protocol::MAX_BYTES, "features": ["pbr-materials"] });
    writeln!(stream, "{hello}")?;
    {
        let mut status = shared.status.lock();
        status.connected = true;
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
            writeln!(stream, "{{\"ok\":true}}")?;
            continue;
        }
        if len > protocol::MAX_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut bytes = vec![0; len];
        read(stream, &mut bytes, shared, Duration::from_secs(5))?;
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
        writeln!(stream, "{reply}")?;
        ctx.request_repaint();
    }
    Ok(())
}
