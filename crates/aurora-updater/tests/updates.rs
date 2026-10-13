//! Disposable packages and a real local HTTP server exercise release filtering,
//! byte integrity, cancellation, ZIP validation and transactional rollback.

use aurora_updater::{
    Cancel, Failure,
    feed::{Asset, Release, Source},
    install, media_pack, package,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("aurora-updater-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).expect("test directory");
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        assert!(self.0.parent() == Some(std::env::temp_dir().as_path()));
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(routes: HashMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
        listener.set_nonblocking(true).expect("nonblocking");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                if let Ok((mut socket, _)) = listener.accept() {
                    // Windows accepts can inherit the listener's nonblocking
                    // mode; wait for request bytes rather than closing early.
                    socket.set_nonblocking(false).expect("blocking connection");
                    socket.set_read_timeout(Some(Duration::from_secs(2))).expect("timeout");
                    let mut data = Vec::new();
                    let mut buffer = [0_u8; 1024];
                    while data.len() < 8192 && !data.ends_with(b"\r\n\r\n") {
                        match socket.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => data.extend_from_slice(&buffer[..n]),
                        }
                    }
                    if !data.is_empty() {
                        let path = std::str::from_utf8(&data)
                            .ok()
                            .and_then(|s| s.split_whitespace().nth(1))
                            .unwrap_or("");
                        let (status, body) = if let Some(body) = routes.get(path) {
                            ("200 OK", body.as_slice())
                        } else {
                            ("404 Not Found", b"missing".as_slice())
                        };
                        let mut response =
                            format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                        response.extend_from_slice(body);
                        let _ = socket.write_all(&response);
                        let _ = socket.shutdown(std::net::Shutdown::Write);
                        // Drain the peer before dropping the socket: closing with
                        // unread bytes can reset the connection on Windows.
                        while matches!(socket.read(&mut buffer), Ok(n) if n > 0) {}
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        Self {
            base,
            stop,
            thread: Some(thread),
        }
    }
    fn source(&self) -> Source {
        Source::local(&format!("{}/releases", self.base)).expect("test feed")
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("HTTP thread");
        }
    }
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}
fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("ZIP member");
        writer.write_all(value).expect("ZIP data");
    }
    writer.finish().expect("ZIP").into_inner()
}
fn valid_zip() -> Vec<u8> {
    zip(&[
        ("Aurora-Viewer-v0.3.0-windows-x64/aurora-viewer.exe", b"new viewer"),
        ("Aurora-Viewer-v0.3.0-windows-x64/assets/character/avatar.llm", b"avatar"),
        ("Aurora-Viewer-v0.3.0-windows-x64/LICENSE", b"license"),
        (
            "Aurora-Viewer-v0.3.0-windows-x64/aurora-install.json",
            br#"{"schema":1,"version":"0.3.0"}"#,
        ),
    ])
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn release(server: &Server, size: usize) -> Release {
    Release {
        tag: "v0.3.0".into(),
        version: "0.3.0".into(),
        notes: "Changes".into(),
        url: format!("{}/release", server.base),
        required: false,
        maintenance: Default::default(),
        archive: Some(Asset {
            name: "package.zip".into(),
            size: size as u64,
            browser_download_url: format!("{}/package.zip", server.base),
        }),
        checksum: Some(Asset {
            name: "package.zip.sha256".into(),
            size: 80,
            browser_download_url: format!("{}/hash", server.base),
        }),
    }
}

#[test]
fn verifies_and_extracts_real_download() {
    let bytes = valid_zip();
    let hash = format!("{}  package.zip\n", digest(&bytes));
    let server = Server::new(HashMap::from([
        ("/package.zip".into(), bytes.clone()),
        ("/hash".into(), hash.into_bytes()),
    ]));
    let temp = Temp::new();
    let root = run(package::download(
        &server.source(),
        &release(&server, bytes.len()),
        &temp.0,
        &Cancel::default(),
        |_| {},
    ))
    .expect("verified download");
    assert_eq!(fs::read(root.join("aurora-viewer.exe")).expect("viewer"), b"new viewer");
}

#[test]
fn rejects_hash_mismatch_and_truncated_transfer_before_extraction() {
    let bytes = valid_zip();
    for (hash, size) in [("0".repeat(64), bytes.len()), (digest(&bytes), bytes.len() + 10)] {
        let server = Server::new(HashMap::from([
            ("/package.zip".into(), bytes.clone()),
            ("/hash".into(), format!("{hash} package.zip").into_bytes()),
        ]));
        let temp = Temp::new();
        let outcome = run(package::download(
            &server.source(),
            &release(&server, size),
            &temp.0,
            &Cancel::default(),
            |_| {},
        ));
        assert!(matches!(outcome, Err(Failure::Integrity)), "download outcome: {outcome:?}");
        assert!(!temp.0.join("prepared").exists());
    }
}

#[test]
fn cancellation_never_extracts_or_changes_installation() {
    let bytes = valid_zip();
    let server = Server::new(HashMap::from([
        ("/package.zip".into(), bytes.clone()),
        ("/hash".into(), format!("{} package.zip", digest(&bytes)).into_bytes()),
    ]));
    let temp = Temp::new();
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(
        run(package::download(
            &server.source(),
            &release(&server, bytes.len()),
            &temp.0,
            &cancel,
            |_| {}
        ))
        .is_err()
    );
    assert!(!temp.0.join("prepared").exists());
}

#[test]
fn invalid_zip_paths_and_case_aliases_do_not_escape_staging() {
    for entries in [
        vec![("../escape.txt", b"bad".as_slice())],
        vec![
            ("Aurora-Viewer-v0.3.0-windows-x64/FILE", b"a".as_slice()),
            ("Aurora-Viewer-v0.3.0-windows-x64/file", b"b".as_slice()),
        ],
    ] {
        let temp = Temp::new();
        let archive = temp.0.join("test.zip");
        fs::write(&archive, zip(&entries)).expect("ZIP file");
        assert!(package::extract(&archive, &temp.0.join("prepared"), "v0.3.0", &Cancel::default()).is_err());
        assert!(!temp.0.join("escape.txt").exists());
        assert!(!temp.0.join("prepared").exists());
    }
}

fn installation(path: &Path, version: &[u8]) {
    fs::create_dir_all(path.join("assets")).expect("assets");
    fs::write(path.join("aurora-viewer.exe"), version).expect("viewer");
    fs::write(path.join("LICENSE"), b"license").expect("license");
}
#[test]
fn new_viewer_failure_restores_old_files_and_preserves_profile() {
    let temp = Temp::new();
    let install = temp.0.join("install");
    installation(&install, b"old");
    fs::write(temp.0.join("settings.json"), b"preferences").expect("profile");
    let work = install::Workspace::new(&install).expect("workspace");
    installation(&work.root.join("prepared"), b"new");
    assert!(
        install::apply(&install, &work.root, || {
            assert_eq!(fs::read(install.join("aurora-viewer.exe"))?, b"new");
            assert_eq!(fs::read(work.root.join("backup/aurora-viewer.exe"))?, b"old");
            anyhow::bail!("new process exited without a frame")
        })
        .is_err()
    );
    assert_eq!(fs::read(install.join("aurora-viewer.exe")).expect("restored"), b"old");
    assert!(
        work.root.join("failed/aurora-viewer.exe").is_file(),
        "the new package was actually launched then rolled back"
    );
    assert_eq!(fs::read(temp.0.join("settings.json")).expect("profile intact"), b"preferences");
}

#[test]
fn success_removes_backup_only_after_launch_and_refuses_foreign_files_or_concurrency() {
    let temp = Temp::new();
    let path = temp.0.join("install");
    installation(&path, b"old");
    fs::write(path.join("personal-document.txt"), b"mine").expect("foreign file");
    assert!(install::Workspace::new(&path).is_err());
    fs::remove_file(path.join("personal-document.txt")).expect("test file");
    let work = install::Workspace::new(&path).expect("workspace");
    assert!(install::Workspace::new(&path).is_err());
    installation(&work.root.join("prepared"), b"new");
    install::apply(&path, &work.root, || {
        assert!(work.root.join("backup/aurora-viewer.exe").is_file());
        Ok(())
    })
    .expect("swap committed");
    assert_eq!(fs::read(path.join("aurora-viewer.exe")).expect("new viewer"), b"new");
    assert!(!work.root.join("backup").exists());
}

const PACK_ROUTE: &str = "/download/media-plugins-2/Aurora-Media-Plugins-2-windows-x64.zip";

fn pack_zip(with_cef: bool) -> Vec<u8> {
    let mut entries: Vec<(&str, &[u8])> = vec![
        (
            "Aurora-Media-Plugins-2-windows-x64/aurora-media-plugins.json",
            br#"{"schema":1,"version":"2"}"#,
        ),
        ("Aurora-Media-Plugins-2-windows-x64/llplugin/SLPlugin.exe", b"launcher"),
        ("Aurora-Media-Plugins-2-windows-x64/llplugin/media_plugin_libvlc.dll", b"vlc"),
        ("Aurora-Media-Plugins-2-windows-x64/llplugin/locales/fr.pak", b"fr"),
    ];
    if with_cef {
        entries.push(("Aurora-Media-Plugins-2-windows-x64/llplugin/media_plugin_cef.dll", b"cef"));
    }
    zip(&entries)
}

fn pack(bytes: &[u8], sha256: String) -> media_pack::Pack {
    media_pack::Pack {
        version: "2",
        size: bytes.len() as u64,
        sha256: Box::leak(sha256.into_boxed_str()),
    }
}

/// Folders of `root` whose name starts with a dot (staging, trash).
fn hidden(root: &Path) -> usize {
    fs::read_dir(root)
        .expect("root")
        .filter(|e| {
            let e = e.as_ref().expect("entry");
            e.path().is_dir() && e.file_name().to_string_lossy().starts_with('.')
        })
        .count()
}

#[test]
fn media_pack_installs_once_and_replaces_what_older_viewers_left() {
    let bytes = pack_zip(true);
    let pack = pack(&bytes, digest(&bytes));
    let server = Server::new(HashMap::from([(PACK_ROUTE.into(), bytes)]));
    let temp = Temp::new();
    let root = temp.0.join("media-plugins");
    fs::create_dir_all(root.join("1/llplugin")).expect("older pack");
    fs::write(root.join("1/llplugin/SLPlugin.exe"), b"old").expect("older pack");
    fs::create_dir_all(root.join(".staging-interrupted")).expect("leftover");
    fs::write(root.join(".staging-interrupted/download.zip"), b"partial").expect("leftover");
    assert!(media_pack::installed(&root, &pack).is_none());
    let dir = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {})).expect("installed pack");
    assert_eq!(dir, root.join("2"));
    assert_eq!(media_pack::installed(&root, &pack), Some(root.join("2")));
    assert_eq!(fs::read(dir.join("llplugin/SLPlugin.exe")).expect("launcher"), b"launcher");
    assert_eq!(fs::read(dir.join("llplugin/locales/fr.pak")).expect("resource"), b"fr");
    assert!(!root.join("1").exists(), "the pack of an older viewer is removed");
    // the staging folder is deleted on its own thread
    for _ in 0..100 {
        if hidden(&root) == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(hidden(&root), 0);
    // an installed pack asks nothing of the network
    drop(server);
    let offline = Source::local("http://127.0.0.1:9/releases").expect("feed");
    assert_eq!(
        run(media_pack::install(&offline, &pack, &root, &Cancel::default(), |_| {})).expect("already installed"),
        root.join("2")
    );
}

#[test]
fn media_pack_is_refused_when_altered_incomplete_or_being_installed() {
    let bytes = pack_zip(true);
    let incomplete = pack_zip(false);
    for (served, pack, expected) in [
        (bytes.clone(), pack(&bytes, "0".repeat(64)), Failure::Integrity),
        (
            bytes.clone(),
            media_pack::Pack {
                size: bytes.len() as u64 + 1,
                ..pack(&bytes, digest(&bytes))
            },
            Failure::Integrity,
        ),
        (incomplete.clone(), pack(&incomplete, digest(&incomplete)), Failure::Package),
    ] {
        let server = Server::new(HashMap::from([(PACK_ROUTE.into(), served)]));
        let temp = Temp::new();
        let root = temp.0.join("media-plugins");
        let outcome = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {}));
        assert!(
            matches!((&outcome, expected), (Err(Failure::Integrity), Failure::Integrity) | (Err(Failure::Package), Failure::Package)),
            "install outcome: {outcome:?}"
        );
        assert!(media_pack::installed(&root, &pack).is_none());
        assert!(!root.join("2").exists());
    }
    // a second viewer does not download while the first one installs
    let pack = pack(&bytes, digest(&bytes));
    let server = Server::new(HashMap::from([(PACK_ROUTE.into(), bytes)]));
    let temp = Temp::new();
    let root = temp.0.join("media-plugins");
    fs::create_dir_all(&root).expect("root");
    let lock = fs::File::create(root.join(".lock")).expect("lock file");
    lock.try_lock().expect("first viewer");
    let outcome = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {}));
    assert!(matches!(outcome, Err(Failure::Busy)), "install outcome: {outcome:?}");
    assert_eq!(hidden(&root), 0);
}

#[test]
fn release_filter_ignores_assets_drafts_prereleases_and_retrieves_minimum_version() {
    // Bind first, then construct absolute fixture links without relying on GitHub.
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let body = serde_json::json!([
        {"tag_name":"assets-1","html_url":format!("{base}/assets")},
        {"tag_name":"v1.0.0-beta.1","prerelease":true,"html_url":format!("{base}/beta")},
        {"tag_name":"v9.0.0","draft":true,"html_url":format!("{base}/draft")},
        {"tag_name":"v0.3.0","html_url":format!("{base}/release"),"body":"Real notes","assets":[{"name":"aurora-update-policy.json","size":100,"browser_download_url":format!("{base}/policy")}]},
        {"tag_name":"v0.2.0","html_url":format!("{base}/old")}
    ]).to_string();
    let thread = std::thread::spawn(move || {
        for data in [
            body,
            "{\"schema\":1,\"version\":\"0.3.0\",\"minimum_supported_version\":\"0.2.1\"}".into(),
        ] {
            let (mut socket, _) = listener.accept().expect("request");
            let mut request = [0; 4096];
            let _ = socket.read(&mut request);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",
                data.len()
            )
            .expect("response");
        }
    });
    let result = run(Source::local(&format!("{base}/releases")).expect("feed").check("0.2.0+8.gabc123"))
        .expect("check")
        .expect("release");
    thread.join().expect("server");
    assert_eq!(result.version, "0.3.0");
    assert_eq!(result.notes, "Real notes");
    assert!(result.required);
}
