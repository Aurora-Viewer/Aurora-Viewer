//! Disposable packages and a real local HTTP server exercise release filtering,
//! byte integrity, cancellation, ZIP validation and transactional rollback.

use aurora_updater::{
    Cancel, Failure,
    feed::{Archive, Channel, Release, Source},
    install, media_pack, package,
    signature::{self, Entry, Manifest},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
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
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
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
        let routes = Arc::new(Mutex::new(routes));
        let served = routes.clone();
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
                        let body = served.lock().expect("routes").get(path).cloned();
                        let (status, body) = match &body {
                            Some(body) => ("200 OK", body.as_slice()),
                            None => ("404 Not Found", b"missing".as_slice()),
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
            routes,
            stop,
            thread: Some(thread),
        }
    }
    fn source(&self) -> Source {
        Source::local(&format!("{}/releases", self.base)).expect("test feed")
    }
    /// Serves `body` at `path`, in place of what was there.
    fn add(&self, path: &str, body: impl Into<Vec<u8>>) {
        self.routes.lock().expect("routes").insert(path.into(), body.into());
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

/// A release whose package has the size and SHA-256 of `bytes`, as its
/// signed manifest would name them.
fn release(server: &Server, size: usize, bytes: &[u8]) -> Release {
    Release {
        tag: "v0.3.0".into(),
        version: "0.3.0".into(),
        notes: "Changes".into(),
        url: format!("{}/release", server.base),
        required: false,
        maintenance: Default::default(),
        archive: Some(Archive {
            name: "package.zip".into(),
            url: format!("{}/package.zip", server.base),
            size: size as u64,
            sha256: Sha256::digest(bytes).into(),
        }),
    }
}

#[test]
fn verifies_and_extracts_real_download() {
    let bytes = valid_zip();
    let server = Server::new(HashMap::from([("/package.zip".into(), bytes.clone())]));
    let temp = Temp::new();
    let root = run(package::download(
        &server.source(),
        &release(&server, bytes.len(), &bytes),
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
    for (named, size) in [(&b"another package"[..], bytes.len()), (&bytes[..], bytes.len() + 10)] {
        let server = Server::new(HashMap::from([("/package.zip".into(), bytes.clone())]));
        let temp = Temp::new();
        let outcome = run(package::download(
            &server.source(),
            &release(&server, size, named),
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
    let server = Server::new(HashMap::from([("/package.zip".into(), bytes.clone())]));
    let temp = Temp::new();
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(
        run(package::download(
            &server.source(),
            &release(&server, bytes.len(), &bytes),
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

/// A media pack path written with the names of the Windows pack, in the
/// names of this system's pack.
fn os(path: &str) -> String {
    if cfg!(target_os = "linux") {
        path.replace("windows-x64", "linux-x64")
            .replace("SLPlugin.exe", "SLPlugin")
            .replace("media_plugin_libvlc.dll", "libmedia_plugin_gstreamer.so")
            .replace("media_plugin_cef.dll", "libmedia_plugin_cef.so")
    } else if cfg!(target_os = "macos") {
        // SLPlugin.app, its plugins in its Contents/Frameworks
        path.replace("windows-x64", "macos-universal")
            .replace("llplugin/SLPlugin.exe", "llplugin/SLPlugin.app/Contents/MacOS/SLPlugin")
            .replace(
                "llplugin/media_plugin_libvlc.dll",
                "llplugin/SLPlugin.app/Contents/Frameworks/media_plugin_libvlc.dylib",
            )
            .replace(
                "llplugin/media_plugin_cef.dll",
                "llplugin/SLPlugin.app/Contents/Frameworks/media_plugin_cef.dylib",
            )
    } else {
        path.to_string()
    }
}

fn pack_route() -> String {
    os("/download/media-plugins-2/Aurora-Media-Plugins-2-windows-x64.zip")
}

fn pack_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let named: Vec<(String, &[u8])> = entries.iter().map(|(name, value)| (os(name), *value)).collect();
    zip(&named.iter().map(|(name, value)| (name.as_str(), *value)).collect::<Vec<_>>())
}

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
    pack_archive(&entries)
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
    let server = Server::new(HashMap::from([(pack_route(), bytes)]));
    let temp = Temp::new();
    let root = temp.0.join("media-plugins");
    let older = root.join(os("1/llplugin/SLPlugin.exe"));
    fs::create_dir_all(older.parent().expect("folder")).expect("older pack");
    fs::write(older, b"old").expect("older pack");
    fs::create_dir_all(root.join(".staging-interrupted")).expect("leftover");
    fs::write(root.join(".staging-interrupted/download.zip"), b"partial").expect("leftover");
    assert!(media_pack::installed(&root, &pack).is_none());
    let dir = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {})).expect("installed pack");
    assert_eq!(dir, root.join("2"));
    assert_eq!(media_pack::installed(&root, &pack), Some(root.join("2")));
    assert_eq!(fs::read(dir.join(os("llplugin/SLPlugin.exe"))).expect("launcher"), b"launcher");
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
    // the same version built again is another archive: it replaces the folder
    let rebuilt = pack_archive(&[
        (
            "Aurora-Media-Plugins-2-windows-x64/aurora-media-plugins.json",
            br#"{"schema":1,"version":"2"}"#,
        ),
        ("Aurora-Media-Plugins-2-windows-x64/llplugin/SLPlugin.exe", b"launcher built again"),
        ("Aurora-Media-Plugins-2-windows-x64/llplugin/media_plugin_libvlc.dll", b"vlc"),
        ("Aurora-Media-Plugins-2-windows-x64/llplugin/media_plugin_cef.dll", b"cef"),
    ]);
    let rebuilt_pack = self::pack(&rebuilt, digest(&rebuilt));
    assert!(media_pack::installed(&root, &rebuilt_pack).is_none());
    let server = Server::new(HashMap::from([(pack_route(), rebuilt)]));
    run(media_pack::install(
        &server.source(),
        &rebuilt_pack,
        &root,
        &Cancel::default(),
        |_| {},
    ))
    .expect("rebuilt pack");
    assert_eq!(media_pack::installed(&root, &rebuilt_pack), Some(root.join("2")));
    assert!(media_pack::installed(&root, &pack).is_none());
    assert_eq!(
        fs::read(root.join(os("2/llplugin/SLPlugin.exe"))).expect("launcher"),
        b"launcher built again"
    );
    assert!(!root.join("2/llplugin/locales").exists(), "nothing of the replaced folder is left");
}

#[cfg(unix)]
#[test]
fn media_pack_programs_stay_executable_without_setuid() {
    use std::os::unix::fs::PermissionsExt;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let plain = zip::write::SimpleFileOptions::default();
    for (name, value, options) in [
        (
            "Aurora-Media-Plugins-2-windows-x64/aurora-media-plugins.json",
            &br#"{"schema":1,"version":"2"}"#[..],
            plain,
        ),
        (
            "Aurora-Media-Plugins-2-windows-x64/llplugin/SLPlugin.exe",
            b"launcher",
            plain.unix_permissions(0o755),
        ),
        (
            "Aurora-Media-Plugins-2-windows-x64/llplugin/media_plugin_libvlc.dll",
            b"video",
            plain,
        ),
        ("Aurora-Media-Plugins-2-windows-x64/llplugin/media_plugin_cef.dll", b"cef", plain),
        (
            "Aurora-Media-Plugins-2-windows-x64/llplugin/chrome-sandbox",
            b"sandbox",
            plain.unix_permissions(0o4755),
        ),
    ] {
        writer.start_file(os(name), options).expect("ZIP member");
        writer.write_all(value).expect("ZIP data");
    }
    let bytes = writer.finish().expect("ZIP").into_inner();
    let pack = pack(&bytes, digest(&bytes));
    let server = Server::new(HashMap::from([(pack_route(), bytes)]));
    let temp = Temp::new();
    let root = temp.0.join("media-plugins");
    let dir = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {})).expect("installed pack");
    let mode = |name: &str| fs::metadata(dir.join(os(name))).expect("installed file").permissions().mode() & 0o7777;
    assert_eq!(mode("llplugin/SLPlugin.exe"), 0o755);
    assert_eq!(mode("llplugin/chrome-sandbox"), 0o755, "never setuid from an archive");
    assert_eq!(mode("llplugin/media_plugin_cef.dll") & 0o111, 0);
}

/// A viewer still running holds a shared lock on `.in-use-<version>`: its pack
/// is neither removed nor replaced under it, on Linux and macOS as well,
/// where nothing refuses to rename a folder in use.
#[test]
fn media_pack_of_a_running_viewer_stays_whole() {
    let bytes = pack_zip(true);
    let pack = pack(&bytes, digest(&bytes));
    let temp = Temp::new();
    let root = temp.0.join("media-plugins");
    let older = root.join(os("1/llplugin/SLPlugin.exe"));
    fs::create_dir_all(older.parent().expect("folder")).expect("older pack");
    fs::write(&older, b"old").expect("older pack");
    let damaged = root.join(os("2/llplugin/SLPlugin.exe"));
    fs::create_dir_all(damaged.parent().expect("folder")).expect("damaged pack");
    fs::write(&damaged, b"damaged").expect("damaged pack");
    let in_use = |version: &str| {
        let file = fs::File::create(root.join(format!(".in-use-{version}"))).expect("lock file");
        file.try_lock_shared().expect("running viewer");
        file
    };
    let (older_viewer, same_viewer) = (in_use("1"), in_use("2"));
    let server = Server::new(HashMap::from([(pack_route(), bytes)]));
    let outcome = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {}));
    assert!(matches!(outcome, Err(Failure::Package)), "install outcome: {outcome:?}");
    assert_eq!(fs::read(&damaged).expect("kept"), b"damaged", "a pack in use is not replaced");
    drop(same_viewer);
    let dir = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {})).expect("installed pack");
    assert_eq!(fs::read(dir.join(os("llplugin/SLPlugin.exe"))).expect("launcher"), b"launcher");
    assert_eq!(fs::read(&older).expect("kept"), b"old", "an older viewer keeps its pack");
    drop(older_viewer);
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
        let server = Server::new(HashMap::from([(pack_route(), served)]));
        let temp = Temp::new();
        let root = temp.0.join("media-plugins");
        let outcome = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {}));
        assert!(
            matches!(
                (&outcome, expected),
                (Err(Failure::Integrity), Failure::Integrity) | (Err(Failure::Package), Failure::Package)
            ),
            "install outcome: {outcome:?}"
        );
        assert!(media_pack::installed(&root, &pack).is_none());
        assert!(!root.join("2").exists());
    }
    // a second viewer does not download while the first one installs
    let pack = pack(&bytes, digest(&bytes));
    let server = Server::new(HashMap::from([(pack_route(), bytes)]));
    let temp = Temp::new();
    let root = temp.0.join("media-plugins");
    fs::create_dir_all(&root).expect("root");
    let lock = fs::File::create(root.join(".lock")).expect("lock file");
    lock.try_lock().expect("first viewer");
    let outcome = run(media_pack::install(&server.source(), &pack, &root, &Cancel::default(), |_| {}));
    assert!(matches!(outcome, Err(Failure::Busy)), "install outcome: {outcome:?}");
    assert_eq!(hidden(&root), 0);
}

/// A release key of the tests only.
struct Key {
    pkcs8: Vec<u8>,
    public: [u8; 32],
}
impl Key {
    fn new() -> Self {
        let pkcs8 = signature::generate().expect("test key");
        let public = signature::public_key(&pkcs8).expect("public key");
        Self { pkcs8, public }
    }
}

/// A release of the test feed: its files served under `/<tag>/`, listed
/// with a manifest of `version` signed with `key`, and the manifest's
/// signature. `extra` adds or replaces fields of the release.
fn publish(server: &Server, key: &Key, tag: &str, version: &str, files: &[(&str, &[u8])], extra: serde_json::Value) -> serde_json::Value {
    let manifest = Manifest::new(version, files.iter().map(|(name, bytes)| Entry::of(name, bytes)).collect()).to_bytes();
    let sig = signature::sign(&manifest, &key.pkcs8).expect("signature");
    let mut assets = Vec::new();
    for (name, bytes) in files
        .iter()
        .map(|(n, b)| (*n, b.to_vec()))
        .chain([(signature::MANIFEST, manifest), (signature::SIGNATURE, sig.into_bytes())])
    {
        let path = format!("/{tag}/{name}");
        assets.push(serde_json::json!({"name": name, "size": bytes.len(), "browser_download_url": format!("{}{path}", server.base)}));
        server.add(&path, bytes);
    }
    let mut release = serde_json::json!({
        "tag_name": tag,
        "prerelease": tag.contains('-'),
        "html_url": format!("{}/{tag}", server.base),
        "assets": assets,
    });
    if let (Some(release), Some(extra)) = (release.as_object_mut(), extra.as_object()) {
        release.extend(extra.clone());
    }
    release
}

fn feed(server: &Server, releases: &[serde_json::Value]) {
    server.add("/releases", serde_json::Value::from(releases.to_vec()).to_string());
}

const POLICY: &[u8] = br#"{"schema":1,"version":"0.3.0","minimum_supported_version":"0.2.1"}"#;

#[test]
fn a_signed_release_is_offered_with_its_policy_and_installs() {
    let (server, key, zip) = (Server::new(HashMap::new()), Key::new(), valid_zip());
    let files = [
        ("Aurora-Viewer-v0.3.0-windows-x64.zip", &zip[..]),
        ("aurora-update-policy.json", POLICY),
    ];
    feed(
        &server,
        &[
            serde_json::json!({"tag_name":"assets-1","html_url":format!("{}/assets", server.base)}),
            serde_json::json!({"tag_name":"v1.0.0-beta.1","prerelease":true,"html_url":format!("{}/beta", server.base)}),
            serde_json::json!({"tag_name":"v9.0.0","draft":true,"html_url":format!("{}/draft", server.base)}),
            publish(&server, &key, "v0.3.0", "0.3.0", &files, serde_json::json!({"body": "Real notes"})),
            serde_json::json!({"tag_name":"v0.2.0","html_url":format!("{}/old", server.base)}),
        ],
    );
    let source = server.source().trust(key.public).expect("test key");
    let release = run(source.check("0.2.0+8.gabc123", Channel::Stable))
        .expect("check")
        .expect("release");
    assert_eq!(release.version, "0.3.0");
    assert_eq!(release.notes, "Real notes");
    assert!(release.required, "the signed policy applies");
    let temp = Temp::new();
    let root = run(package::download(&source, &release, &temp.0, &Cancel::default(), |_| {})).expect("installed");
    assert_eq!(fs::read(root.join("aurora-viewer.exe")).expect("viewer"), b"new viewer");
    // The compiled keys alone do not know the test key.
    let refusal = run(server.source().check("0.2.0", Channel::Stable)).expect_err("unknown key");
    assert!(signature::is_refused(&refusal), "{refusal:#}");
    assert!(Source::default().trust(key.public).is_err(), "no test key on the published feed");
}

#[test]
fn unsigned_altered_or_mismatched_releases_are_refused() {
    let zip = valid_zip();
    let name = "Aurora-Viewer-v0.3.0-windows-x64.zip";
    let files = [(name, &zip[..]), ("aurora-update-policy.json", POLICY)];
    let without = |release: &mut serde_json::Value, asset: &str| {
        release["assets"].as_array_mut().expect("assets").retain(|a| a["name"] != asset);
    };
    type Tamper = Box<dyn Fn(&Server, &Key, &mut serde_json::Value)>;
    let other = zip.clone();
    let size = zip.len();
    let cases: Vec<(&str, Tamper)> = vec![
        ("no signature", Box::new(move |_, _, r| without(r, signature::SIGNATURE))),
        ("no manifest", Box::new(move |_, _, r| without(r, signature::MANIFEST))),
        (
            "wrong key",
            Box::new(move |s, _, r| {
                *r = publish(
                    s,
                    &Key::new(),
                    "v0.3.0",
                    "0.3.0",
                    &[(name, &other), ("aurora-update-policy.json", POLICY)],
                    serde_json::json!({}),
                )
            }),
        ),
        (
            "manifest of another version",
            Box::new(|s, k, r| {
                *r = publish(
                    s,
                    k,
                    "v0.3.0",
                    "0.2.9",
                    &[("aurora-update-policy.json", POLICY)],
                    serde_json::json!({}),
                )
            }),
        ),
        (
            "altered signature",
            Box::new(|s, _, _| s.add(&format!("/v0.3.0/{}", signature::SIGNATURE), "00".repeat(64))),
        ),
        (
            "altered policy",
            Box::new(|s, _, _| {
                s.add(
                    "/v0.3.0/aurora-update-policy.json",
                    &br#"{"schema":1,"version":"0.3.0","minimum_supported_version":"0.3.0"}"#[..],
                )
            }),
        ),
        (
            "policy taken away",
            Box::new(move |_, _, r| without(r, "aurora-update-policy.json")),
        ),
        (
            "package of another size",
            Box::new(move |_, _, r| {
                for asset in r["assets"].as_array_mut().expect("assets") {
                    if asset["name"] == name {
                        asset["size"] = serde_json::json!(size + 1);
                    }
                }
            }),
        ),
    ];
    for (case, tamper) in cases {
        let server = Server::new(HashMap::new());
        let key = Key::new();
        let mut release = publish(&server, &key, "v0.3.0", "0.3.0", &files, serde_json::json!({}));
        tamper(&server, &key, &mut release);
        feed(&server, &[release]);
        let source = server.source().trust(key.public).expect("test key");
        let refusal = run(source.check("0.2.0", Channel::Stable)).expect_err(case);
        assert!(signature::is_refused(&refusal), "{case}: {refusal:#}");
    }

    // A package replaced by another of the same size: offered, then refused
    // at download, before anything is extracted.
    let server = Server::new(HashMap::new());
    let key = Key::new();
    feed(&server, &[publish(&server, &key, "v0.3.0", "0.3.0", &files, serde_json::json!({}))]);
    let mut altered = zip.clone();
    let last = altered.len() - 1;
    altered[last] ^= 1;
    server.add(&format!("/v0.3.0/{name}"), altered);
    let source = server.source().trust(key.public).expect("test key");
    let release = run(source.check("0.2.0", Channel::Stable)).expect("check").expect("release");
    let temp = Temp::new();
    let outcome = run(package::download(&source, &release, &temp.0, &Cancel::default(), |_| {}));
    assert!(matches!(outcome, Err(Failure::Integrity)), "download outcome: {outcome:?}");
    assert!(!temp.0.join("prepared").exists());
}

#[test]
fn the_beta_channel_takes_betas_and_moves_on_to_the_stable_of_the_same_number() {
    let key = Key::new();
    let check = |current: &str, channel: Channel, stable_out: bool| {
        let server = Server::new(HashMap::new());
        let release = |tag: &str, extra: serde_json::Value| publish(&server, &key, tag, tag.trim_start_matches('v'), &[], extra);
        let mut list = vec![
            release("v0.8.0-beta.20261026.1", serde_json::json!({})),
            release("v0.7.7-rc.1", serde_json::json!({})),
            release("v0.9.0", serde_json::json!({"prerelease": true})),
            release("v0.7.7-beta.20261025.2", serde_json::json!({"prerelease": false})),
            release("v0.7.7-beta.20261025.1", serde_json::json!({})),
            release("v0.7.6", serde_json::json!({})),
        ];
        if stable_out {
            list[0] = release("v0.7.7", serde_json::json!({}));
        }
        feed(&server, &list);
        let source = server.source().trust(key.public).expect("test key");
        run(source.check(current, channel)).expect("check").map(|r| r.version)
    };
    // Stable never sees a beta, nor a stable number GitHub marks pre-release.
    assert_eq!(check("0.7.5", Channel::Stable, false).as_deref(), Some("0.7.6"));
    // Beta takes the newest beta; an rc or a mislabelled release is skipped.
    assert_eq!(check("0.7.6", Channel::Beta, false).as_deref(), Some("0.8.0-beta.20261026.1"));
    // Once the stable of that number is out, the beta tester moves on to it.
    assert_eq!(check("0.7.7-beta.20261025.1", Channel::Beta, true).as_deref(), Some("0.7.7"));
    // Back on Stable, a beta waits for the next stable instead of a downgrade.
    assert_eq!(check("0.7.7-beta.20261025.1", Channel::Stable, false), None);
}
