//! Disposable packages and a real local HTTP server exercise release filtering,
//! the release signatures, byte integrity, cancellation, archive validation
//! (ZIP and tar.gz) and transactional rollback.

use aurora_updater::{
    Cancel, Failure,
    feed::{Asset, Channel, Release, Source},
    install, media_pack, package, signature,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
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
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Each request's path, and whether it asked if the list changed.
    seen: Arc<Mutex<Vec<(String, bool)>>>,
}
/// The ETag of the release list (`/releases`): asked again with it, the
/// server answers "304 Not Modified".
const LIST_ETAG: &str = "\"list-1\"";
fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    (listener, base)
}
impl Server {
    fn new(routes: HashMap<String, Vec<u8>>) -> Self {
        let (listener, base) = bind();
        Self::start(listener, base, routes)
    }
    fn start(listener: TcpListener, base: String, routes: HashMap<String, Vec<u8>>) -> Self {
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
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
                        let request = String::from_utf8_lossy(&data);
                        let path = request.split_whitespace().nth(1).unwrap_or("");
                        let asks = request.to_ascii_lowercase().contains(&format!("if-none-match: {LIST_ETAG}"));
                        record.lock().expect("requests").push((path.to_owned(), asks));
                        let list = path == "/releases";
                        let (status, body) = match routes.get(path) {
                            Some(_) if list && asks => ("304 Not Modified", b"".as_slice()),
                            Some(body) => ("200 OK", body.as_slice()),
                            None => ("404 Not Found", b"missing".as_slice()),
                        };
                        let etag = if list { format!("ETag: {LIST_ETAG}\r\n") } else { String::new() };
                        let mut response = format!(
                            "HTTP/1.1 {status}\r\n{etag}Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
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
            seen,
        }
    }
    fn source(&self) -> Source {
        Source::local(&format!("{}/releases", self.base)).expect("test feed")
    }
    /// The feed, trusting the releases signed by `signer`.
    fn trusting(&self, signer: &Ed25519KeyPair) -> Source {
        self.source().trusting(key(signer)).expect("loopback feed")
    }
    /// How many times `path` was asked for.
    fn asked(&self, path: &str) -> usize {
        self.seen.lock().expect("requests").iter().filter(|(p, _)| p == path).count()
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

/// One member of a tar.gz fixture: a file, a folder, or a link of the kinds
/// a package must never hold.
struct Member {
    name: String,
    data: Vec<u8>,
    mode: u32,
    kind: tar::EntryType,
    link: String,
}

impl Member {
    fn file(name: impl Into<String>, data: impl Into<Vec<u8>>, mode: u32) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
            mode,
            kind: tar::EntryType::Regular,
            link: String::new(),
        }
    }
    fn dir(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data: Vec::new(),
            mode: 0o755,
            kind: tar::EntryType::Directory,
            link: String::new(),
        }
    }
    fn symlink(name: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data: Vec::new(),
            mode: 0o777,
            kind: tar::EntryType::Symlink,
            link: target.into(),
        }
    }
    fn hardlink(name: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data: Vec::new(),
            mode: 0o644,
            kind: tar::EntryType::Link,
            link: target.into(),
        }
    }
}

fn tar_gz(members: &[Member]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for member in members {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(member.kind);
        header.set_size(member.data.len() as u64);
        header.set_mode(member.mode);
        if !member.link.is_empty() {
            header.set_link_name(&member.link).expect("link name");
        }
        // tar-rs refuses to write `..` or an absolute name: write them as a
        // hostile archive would, then seal the header
        assert!(member.name.len() <= 100, "test member name");
        header.as_mut_bytes()[..100].fill(0);
        header.as_mut_bytes()[..member.name.len()].copy_from_slice(member.name.as_bytes());
        header.set_cksum();
        builder.append(&header, member.data.as_slice()).expect("tar member");
    }
    builder.into_inner().expect("tar").finish().expect("gzip")
}

fn valid_tar_gz() -> Vec<u8> {
    tar_gz(&[
        Member::dir("Aurora-Viewer-v0.3.0-linux-x64/"),
        Member::file("Aurora-Viewer-v0.3.0-linux-x64/aurora-viewer", b"new viewer", 0o755),
        Member::dir("Aurora-Viewer-v0.3.0-linux-x64/assets/"),
        Member::file("Aurora-Viewer-v0.3.0-linux-x64/assets/character/avatar.llm", b"avatar", 0o644),
        Member::file("Aurora-Viewer-v0.3.0-linux-x64/LICENSE", b"license", 0o644),
        Member::file(
            "Aurora-Viewer-v0.3.0-linux-x64/aurora-install.json",
            br#"{"schema":1,"version":"0.3.0"}"#,
            0o644,
        ),
    ])
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A release offering the archive `name`, served at `path`, whose signed
/// `.sha256` gave `sha256` (`Source::check`).
fn release(server: &Server, size: usize, name: &str, path: &str, sha256: [u8; 32]) -> Release {
    Release {
        tag: "v0.3.0".into(),
        version: "0.3.0".into(),
        notes: "Changes".into(),
        url: format!("{}/release", server.base),
        required: false,
        minimum: None,
        grace: aurora_updater::feed::DEFAULT_GRACE,
        maintenance: Default::default(),
        archive: Some(Asset {
            name: name.into(),
            size: size as u64,
            browser_download_url: format!("{}{path}", server.base),
        }),
        sha256: Some(sha256),
    }
}

#[test]
fn verifies_and_extracts_real_download() {
    let bytes = valid_zip();
    let server = Server::new(HashMap::from([("/package.zip".into(), bytes.clone())]));
    let temp = Temp::new();
    let root = run(package::download(
        &server.source(),
        &release(&server, bytes.len(), "package.zip", "/package.zip", sha256(&bytes)),
        &temp.0,
        &Cancel::default(),
        |_| {},
    ))
    .expect("verified download");
    assert_eq!(fs::read(root.join("aurora-viewer.exe")).expect("viewer"), b"new viewer");
}

#[test]
fn verifies_and_extracts_a_linux_tarball() {
    let bytes = valid_tar_gz();
    let name = "Aurora-Viewer-v0.3.0-linux-x64.tar.gz";
    let server = Server::new(HashMap::from([("/package.tar.gz".into(), bytes.clone())]));
    let temp = Temp::new();
    let root = run(package::download(
        &server.source(),
        &release(&server, bytes.len(), name, "/package.tar.gz", sha256(&bytes)),
        &temp.0,
        &Cancel::default(),
        |_| {},
    ))
    .expect("verified download");
    assert_eq!(fs::read(root.join("aurora-viewer")).expect("viewer"), b"new viewer");
    assert_eq!(fs::read(root.join("assets/character/avatar.llm")).expect("asset"), b"avatar");
}

#[test]
fn rejects_hash_mismatch_and_truncated_transfer_before_extraction() {
    let bytes = valid_zip();
    for (hash, size) in [([0; 32], bytes.len()), (sha256(&bytes), bytes.len() + 10)] {
        let server = Server::new(HashMap::from([("/package.zip".into(), bytes.clone())]));
        let temp = Temp::new();
        let outcome = run(package::download(
            &server.source(),
            &release(&server, size, "package.zip", "/package.zip", hash),
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
            &release(&server, bytes.len(), "package.zip", "/package.zip", sha256(&bytes)),
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

#[test]
fn tarballs_with_links_traversal_or_case_aliases_never_escape_staging() {
    let root = "Aurora-Viewer-v0.3.0-linux-x64";
    for members in [
        // a valid entry first: nothing is extracted before the last is known
        vec![
            Member::file(format!("{root}/LICENSE"), b"license", 0o644),
            Member::file("../escape.txt", b"bad", 0o644),
        ],
        vec![Member::symlink(format!("{root}/passwd"), "/etc/passwd")],
        vec![Member::hardlink(format!("{root}/hard"), "LICENSE")],
        vec![
            Member::file(format!("{root}/FILE"), b"a", 0o644),
            Member::file(format!("{root}/file"), b"b", 0o644),
        ],
        // the same entry twice
        vec![
            Member::file(format!("{root}/LICENSE"), b"a", 0o644),
            Member::file(format!("{root}/LICENSE"), b"b", 0o644),
        ],
        // a package wrapped in another folder
        vec![Member::file(format!("other/{root}/LICENSE"), b"a", 0o644)],
        vec![],
    ] {
        let temp = Temp::new();
        let archive = temp.0.join("test.tar.gz");
        fs::write(&archive, tar_gz(&members)).expect("tar.gz file");
        assert!(package::extract(&archive, &temp.0.join("prepared"), "v0.3.0", &Cancel::default()).is_err());
        assert!(!temp.0.join("escape.txt").exists());
        assert!(!temp.0.join("prepared").exists());
    }
}

#[cfg(unix)]
#[test]
fn a_linux_tarball_keeps_its_executable_bit_and_never_setuid() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let archive = temp.0.join("package.tar.gz");
    fs::write(&archive, valid_tar_gz()).expect("tar.gz file");
    let root = package::extract(&archive, &temp.0.join("prepared"), "v0.3.0", &Cancel::default()).expect("extracted");
    let mode = |path: &str| fs::metadata(root.join(path)).expect("file").permissions().mode() & 0o7777;
    assert_eq!(fs::read(root.join("aurora-viewer")).expect("viewer"), b"new viewer");
    assert_eq!(mode("aurora-viewer"), 0o755);
    assert_eq!(mode("LICENSE"), 0o644);
}

fn installation(path: &Path, version: &[u8]) {
    fs::create_dir_all(path.join("assets")).expect("assets");
    fs::write(path.join(exe()), version).expect("viewer");
    fs::write(path.join("assets/dictionary.dic"), version).expect("asset");
    fs::write(path.join("LICENSE"), b"license").expect("license");
    fs::write(path.join("aurora-install.json"), version).expect("manifest");
    // the package kept the executable bit, and so does the swap
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path.join(exe()), fs::Permissions::from_mode(0o755)).expect("executable");
    }
}

/// What a user may keep beside the viewer: a file, a folder with a file.
fn personal_files(path: &Path) {
    fs::write(path.join("personal-document.txt"), b"mine").expect("personal file");
    fs::create_dir_all(path.join("skins/mine")).expect("personal folder");
    fs::write(path.join("skins/mine/theme.json"), b"my skin").expect("personal file in a folder");
}
fn assert_personal_files(path: &Path) {
    assert_eq!(fs::read(path.join("personal-document.txt")).expect("personal file kept"), b"mine");
    assert_eq!(
        fs::read(path.join("skins/mine/theme.json")).expect("personal folder kept"),
        b"my skin"
    );
}

#[test]
fn an_update_keeps_the_users_own_files_beside_the_viewer() {
    let temp = Temp::new();
    let path = temp.0.join("install");
    installation(&path, b"old");
    personal_files(&path);
    let work = install::Workspace::new(&path).expect("a folder with personal files updates");
    installation(&work.root.join("prepared"), b"new");
    install::apply(&path, &work.root, || {
        assert_personal_files(&path);
        assert!(!work.root.join("backup/personal-document.txt").exists(), "only the package moves");
        Ok(())
    })
    .expect("swap committed");
    assert_eq!(fs::read(path.join(exe())).expect("new viewer"), b"new");
    assert_eq!(fs::read(path.join("assets/dictionary.dic")).expect("new assets"), b"new");
    assert_personal_files(&path);
    assert!(!work.root.join("backup").exists());
}

#[test]
fn a_rollback_keeps_the_users_own_files_too() {
    let temp = Temp::new();
    let path = temp.0.join("install");
    installation(&path, b"old");
    personal_files(&path);
    let work = install::Workspace::new(&path).expect("workspace");
    installation(&work.root.join("prepared"), b"new");
    assert!(install::apply(&path, &work.root, || anyhow::bail!("no frame")).is_err());
    assert_eq!(fs::read(path.join(exe())).expect("restored"), b"old");
    assert_eq!(fs::read(path.join("assets/dictionary.dic")).expect("restored assets"), b"old");
    assert_personal_files(&path);
    assert!(!work.root.join("backup").exists(), "an empty backup tells the restoration worked");
    assert!(!work.root.join("failed/personal-document.txt").exists());
}

/// The viewer copied alone onto a Desktop that also holds the user's own
/// `assets`, `LICENSE` and `README.md`: only the viewer is replaced.
#[test]
fn a_viewer_copied_alone_updates_only_itself() {
    let temp = Temp::new();
    let desktop = temp.0.join("Desktop");
    fs::create_dir_all(desktop.join("assets")).expect("the user's assets");
    fs::write(desktop.join("assets/photo.png"), b"mine").expect("the user's file");
    fs::write(desktop.join("LICENSE"), b"my licence").expect("the user's licence");
    fs::write(desktop.join("README.md"), b"my notes").expect("the user's readme");
    fs::write(desktop.join(exe()), b"old").expect("viewer");
    // a viewer copied alone does run, so it kept its executable bit
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(desktop.join(exe()), fs::Permissions::from_mode(0o755)).expect("executable");
    }
    personal_files(&desktop);
    let work = install::Workspace::new(&desktop).expect("a viewer alone updates");
    installation(&work.root.join("prepared"), b"new");
    fs::write(work.root.join("prepared/README.md"), b"package readme").expect("package readme");
    install::apply(&desktop, &work.root, || Ok(())).expect("swap committed");
    assert_eq!(fs::read(desktop.join(exe())).expect("new viewer"), b"new");
    assert_eq!(fs::read(desktop.join("assets/photo.png")).expect("assets kept"), b"mine");
    assert!(!desktop.join("assets/dictionary.dic").exists(), "no package asset added");
    assert_eq!(fs::read(desktop.join("LICENSE")).expect("licence kept"), b"my licence");
    assert_eq!(fs::read(desktop.join("README.md")).expect("readme kept"), b"my notes");
    assert!(!desktop.join("aurora-install.json").exists(), "no package file added");
    assert_personal_files(&desktop);
}

#[test]
fn a_folder_without_the_viewer_or_an_update_with_foreign_files_is_refused() {
    let temp = Temp::new();
    let empty = temp.0.join("empty");
    fs::create_dir_all(&empty).expect("folder");
    assert!(install::Workspace::check_install(&empty).is_err());
    let path = temp.0.join("install");
    installation(&path, b"old");
    let work = install::Workspace::new(&path).expect("workspace");
    installation(&work.root.join("prepared"), b"new");
    fs::write(work.root.join("prepared/extra.dll"), b"x").expect("foreign file in the update");
    assert!(install::apply(&path, &work.root, || Ok(())).is_err());
    assert_eq!(fs::read(path.join(exe())).expect("untouched"), b"old");
}

/// The viewer's executable, as this system's package names it.
fn exe() -> &'static str {
    if cfg!(windows) { "aurora-viewer.exe" } else { "aurora-viewer" }
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
            assert_eq!(fs::read(install.join(exe()))?, b"new");
            assert_eq!(fs::read(work.root.join("backup").join(exe()))?, b"old");
            anyhow::bail!("new process exited without a frame")
        })
        .is_err()
    );
    assert_eq!(fs::read(install.join(exe())).expect("restored"), b"old");
    assert!(
        work.root.join("failed").join(exe()).is_file(),
        "the new package was actually launched then rolled back"
    );
    assert_eq!(fs::read(temp.0.join("settings.json")).expect("profile intact"), b"preferences");
}

#[test]
fn success_removes_backup_only_after_launch_and_refuses_concurrency() {
    let temp = Temp::new();
    let path = temp.0.join("install");
    installation(&path, b"old");
    let work = install::Workspace::new(&path).expect("workspace");
    assert!(install::Workspace::new(&path).is_err());
    installation(&work.root.join("prepared"), b"new");
    install::apply(&path, &work.root, || {
        assert!(work.root.join("backup").join(exe()).is_file());
        Ok(())
    })
    .expect("swap committed");
    assert_eq!(fs::read(path.join(exe())).expect("new viewer"), b"new");
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

/// A key the test releases are signed with (`seed` tells them apart).
fn signer(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("seed")
}

fn key(signer: &Ed25519KeyPair) -> signature::Key {
    signer.public_key().as_ref().try_into().expect("32 bytes")
}

/// The file whose signed `.sha256` vouches for a release on this system
/// (`feed::signed_name`).
fn vouching(tag: &str) -> String {
    if cfg!(windows) {
        format!("Aurora-Viewer-{tag}-windows-x64.zip")
    } else if cfg!(target_os = "macos") {
        format!("Aurora-Viewer-{tag}-macos-arm64.dmg")
    } else {
        format!("Aurora-Viewer-{tag}-linux-x64.tar.gz")
    }
}

fn package_bytes(tag: &str) -> Vec<u8> {
    format!("package {tag}").into_bytes()
}

/// The `.sha256` file the Release workflow writes for `name`.
fn sidecar(tag: &str, name: &str) -> Vec<u8> {
    format!("{}  {name}\n", digest(&package_bytes(tag))).into_bytes()
}

/// A release list on loopback, with its files served: each release
/// published as the Release workflow publishes it, signed or not.
struct Feed {
    listener: TcpListener,
    base: String,
    list: Vec<serde_json::Value>,
    routes: HashMap<String, Vec<u8>>,
}

impl Feed {
    fn new() -> Self {
        let (listener, base) = bind();
        Self {
            listener,
            base,
            list: Vec::new(),
            routes: HashMap::new(),
        }
    }

    fn path(tag: &str, name: &str) -> String {
        format!("/download/{tag}/{name}")
    }

    fn entry(&mut self, tag: &str) -> &mut serde_json::Value {
        self.list.iter_mut().find(|r| r["tag_name"] == tag).expect("release")
    }

    /// The file `name` of the release `tag`.
    fn add(&mut self, tag: &str, name: &str, bytes: Vec<u8>) {
        let path = Self::path(tag, name);
        let url = format!("{}{path}", self.base);
        let asset = serde_json::json!({"name": name, "size": bytes.len(), "browser_download_url": url});
        self.entry(tag)["assets"].as_array_mut().expect("assets").push(asset);
        self.routes.insert(path, bytes);
    }

    /// The release `tag`: this system's package, its `.sha256` and the
    /// update `policy`, signed by `signer` (or not signed).
    fn release(&mut self, tag: &str, signer: Option<&Ed25519KeyPair>, policy: Option<&str>) -> &mut Self {
        let release = serde_json::json!({
            "tag_name": tag,
            "prerelease": tag.contains('-'),
            "html_url": format!("{}/release/{tag}", self.base),
            "body": format!("Notes of {tag}"),
            "assets": [],
        });
        self.list.push(release);
        let name = vouching(tag);
        self.add(tag, &name, package_bytes(tag));
        let sha = sidecar(tag, &name);
        if let Some(signer) = signer {
            self.add(tag, &format!("{name}.sha256.sig"), signer.sign(&sha).as_ref().to_vec());
        }
        self.add(tag, &format!("{name}.sha256"), sha);
        if let Some(policy) = policy {
            let policy = policy.as_bytes().to_vec();
            if let Some(signer) = signer {
                self.add(tag, "aurora-update-policy.json.sig", signer.sign(&policy).as_ref().to_vec());
            }
            self.add(tag, "aurora-update-policy.json", policy);
        }
        self
    }

    /// The file `name` of `tag` taken out of the release.
    fn remove(&mut self, tag: &str, name: &str) {
        self.entry(tag)["assets"]
            .as_array_mut()
            .expect("assets")
            .retain(|a| a["name"] != name);
    }

    /// The file `name` of `tag` replaced by hand, its listing kept.
    fn replace(&mut self, tag: &str, name: &str, bytes: Vec<u8>) {
        self.routes.insert(Self::path(tag, name), bytes);
    }

    fn serve(mut self) -> Server {
        let list = serde_json::Value::from(self.list).to_string().into_bytes();
        self.routes.insert("/releases".into(), list);
        Server::start(self.listener, self.base, self.routes)
    }
}

fn policy(version: &str, extra: &str) -> String {
    format!("{{\"schema\":1,\"version\":\"{version}\"{extra}}}")
}

/// The version `server` offers above `current`, trusting `signer`.
fn offered(server: &Server, signer: &Ed25519KeyPair, current: &str, channel: Channel) -> Option<String> {
    run(server.trusting(signer).check(current, channel))
        .expect("check")
        .map(|r| r.version)
}

#[test]
fn release_filter_ignores_assets_drafts_prereleases_and_retrieves_minimum_version() {
    let signer = signer(1);
    let mut feed = Feed::new();
    let base = feed.base.clone();
    feed.list
        .push(serde_json::json!({"tag_name":"assets-1","html_url":format!("{base}/assets")}));
    feed.release("v1.0.0-beta.1", Some(&signer), None)
        .release("v9.0.0", Some(&signer), None)
        .release(
            "v0.3.0",
            Some(&signer),
            Some(&policy("0.3.0", ",\"minimum_supported_version\":\"0.2.1\"")),
        )
        .release("v0.2.0", Some(&signer), None);
    feed.entry("v9.0.0")["draft"] = true.into();
    let server = feed.serve();
    let result = run(server.trusting(&signer).check("0.2.0+8.gabc123", Channel::Stable))
        .expect("check")
        .expect("release");
    assert_eq!(result.version, "0.3.0");
    assert_eq!(result.notes, "Notes of v0.3.0");
    assert!(result.required);
    assert_eq!(result.minimum.as_deref(), Some("0.2.1"));
    assert_eq!(result.grace, aurora_updater::feed::DEFAULT_GRACE);
    // the download must match the hash the signed file gives
    if aurora_updater::installs_here() {
        assert_eq!(result.sha256, Some(sha256(&package_bytes("v0.3.0"))));
        assert_eq!(result.archive.map(|a| a.name), Some(vouching("v0.3.0")));
    } else {
        assert_eq!((result.sha256, result.archive.is_none()), (None, true));
    }
}

#[test]
fn the_policy_sets_the_grace_period_below_a_new_minimum() {
    let check = |grace: &str| {
        let signer = signer(1);
        let mut feed = Feed::new();
        let policy = policy("0.3.0", &format!(",\"minimum_supported_version\":\"0.3.0\"{grace}"));
        feed.release("v0.3.0", Some(&signer), Some(&policy));
        let server = feed.serve();
        let result = run(server.trusting(&signer).check("0.2.0", Channel::Stable));
        result.map(|r| r.map(|r| (r.required, r.grace)))
    };
    let hours = |h: u64| std::time::Duration::from_secs(h * 3600);
    // a release that cannot wait blocks at once
    assert_eq!(check(",\"grace_hours\":0").expect("check"), Some((true, hours(0))));
    assert_eq!(check(",\"grace_hours\":24").expect("check"), Some((true, hours(24))));
    // a policy written before grace periods existed gets the default one
    assert_eq!(check("").expect("check"), Some((true, aurora_updater::feed::DEFAULT_GRACE)));
    // more than three days is refused, like any invalid policy
    assert!(check(",\"grace_hours\":100").is_err());
}

#[test]
fn the_beta_channel_takes_betas_and_moves_on_to_the_stable_of_the_same_number() {
    let check = |current: &str, channel: Channel, stable_out: bool| {
        let signer = signer(1);
        let mut feed = Feed::new();
        if stable_out {
            feed.release("v0.7.7", Some(&signer), None);
        } else {
            feed.release("v0.8.0-beta.20261026.1", Some(&signer), None);
        }
        for tag in [
            "v0.7.7-rc.1",
            "v0.9.0",
            "v0.7.7-beta.20261025.2",
            "v0.7.7-beta.20261025.1",
            "v0.7.6",
        ] {
            feed.release(tag, Some(&signer), None);
        }
        // mislabelled on GitHub: a stable number marked pre-release, a beta not
        feed.entry("v0.9.0")["prerelease"] = true.into();
        feed.entry("v0.7.7-beta.20261025.2")["prerelease"] = false.into();
        let server = feed.serve();
        offered(&server, &signer, current, channel)
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

#[test]
fn a_periodic_check_asks_whether_the_list_changed_and_keeps_it_on_not_modified() {
    let signer = signer(1);
    let mut feed = Feed::new();
    feed.release("v0.4.0-beta.20261106.1", Some(&signer), None)
        .release("v0.3.0", Some(&signer), None);
    let server = feed.serve();
    let source = server.trusting(&signer);
    let version = |channel| run(source.check("0.2.0", channel)).expect("check").map(|r| r.version);
    assert_eq!(version(Channel::Stable).as_deref(), Some("0.3.0"));
    // unchanged: the kept list answers, for either channel
    assert_eq!(version(Channel::Stable).as_deref(), Some("0.3.0"));
    assert_eq!(version(Channel::Beta).as_deref(), Some("0.4.0-beta.20261106.1"));
    let lists: Vec<bool> = server
        .seen
        .lock()
        .expect("requests")
        .iter()
        .filter(|(path, _)| path == "/releases")
        .map(|(_, asks)| *asks)
        .collect();
    assert_eq!(lists, [false, true, true]);
    // the signatures of a release still the newest are not downloaded again
    let sig = Feed::path("v0.3.0", &format!("{}.sha256.sig", vouching("v0.3.0")));
    assert_eq!(server.asked(&sig), 1);
}

#[test]
fn only_a_release_signed_with_a_trusted_key_is_offered() {
    let genuine = signer(1);
    let other = signer(2);
    let name = vouching("v0.3.0");
    let check = |change: &dyn Fn(&mut Feed)| {
        let mut feed = Feed::new();
        feed.release("v0.3.0", Some(&genuine), Some(&policy("0.3.0", "")))
            .release("v0.2.5", Some(&genuine), None);
        change(&mut feed);
        let server = feed.serve();
        offered(&server, &genuine, "0.2.0", Channel::Stable)
    };
    assert_eq!(check(&|_| {}).as_deref(), Some("0.3.0"));
    // made by hand on GitHub: no signature, or one by another key; the
    // genuine release below it is offered instead
    assert_eq!(
        check(&|f| f.remove("v0.3.0", &format!("{name}.sha256.sig"))).as_deref(),
        Some("0.2.5")
    );
    let forged = other.sign(&sidecar("v0.3.0", &name)).as_ref().to_vec();
    assert_eq!(
        check(&|f| f.replace("v0.3.0", &format!("{name}.sha256.sig"), forged.clone())).as_deref(),
        Some("0.2.5")
    );
    // the package and its `.sha256` replaced, the old signature kept
    assert_eq!(
        check(&|f| f.replace(
            "v0.3.0",
            &format!("{name}.sha256"),
            format!("{}  {name}\n", "0".repeat(64)).into_bytes()
        ))
        .as_deref(),
        Some("0.2.5")
    );
    // a genuine `.sha256` and its signature copied from an older release:
    // it names the older package
    let older = vouching("v0.2.5");
    let replayed = |f: &mut Feed| {
        f.replace("v0.3.0", &format!("{name}.sha256"), sidecar("v0.2.5", &older));
        f.replace(
            "v0.3.0",
            &format!("{name}.sha256.sig"),
            genuine.sign(&sidecar("v0.2.5", &older)).as_ref().to_vec(),
        );
    };
    assert_eq!(check(&replayed).as_deref(), Some("0.2.5"));
    // the update policy without its signature, or changed after it
    assert_eq!(
        check(&|f| f.remove("v0.3.0", "aurora-update-policy.json.sig")).as_deref(),
        Some("0.2.5")
    );
    assert_eq!(
        check(&|f| f.replace(
            "v0.3.0",
            "aurora-update-policy.json",
            policy("0.3.0", ",\"settings_reset_before\":\"0.3.0\"").into_bytes()
        ))
        .as_deref(),
        Some("0.2.5")
    );
    // nothing signed at all: nothing offered
    assert_eq!(
        check(&|f| {
            for tag in ["v0.3.0", "v0.2.5"] {
                let name = vouching(tag);
                f.remove(tag, &format!("{name}.sha256.sig"));
            }
        }),
        None
    );
}

#[test]
fn a_feed_trusts_no_key_unless_a_test_names_its_own() {
    let signer = signer(1);
    let mut feed = Feed::new();
    feed.release("v0.3.0", Some(&signer), None);
    let server = feed.serve();
    // a loopback feed with no key, or another key: everything is refused
    assert_eq!(
        run(server.source().check("0.2.0", Channel::Stable))
            .expect("check")
            .map(|r| r.version),
        None
    );
    assert_eq!(offered(&server, &self::signer(2), "0.2.0", Channel::Stable), None);
    assert_eq!(offered(&server, &signer, "0.2.0", Channel::Stable).as_deref(), Some("0.3.0"));
    // GitHub's feed takes the release keys only
    assert!(Source::default().trusting(key(&signer)).is_err());
}

#[test]
fn a_few_releases_made_by_hand_are_looked_past_and_a_missing_file_is_an_error() {
    let signer = signer(1);
    let check = |unsigned: &[&str], missing: bool| {
        let mut feed = Feed::new();
        for tag in unsigned {
            feed.release(tag, None, None);
        }
        feed.release("v0.2.5", Some(&signer), None);
        if missing {
            // listed, but its download fails: tried again later, not skipped
            let name = vouching("v0.2.5");
            feed.routes.remove(&Feed::path("v0.2.5", &format!("{name}.sha256.sig")));
        }
        let server = feed.serve();
        run(server.trusting(&signer).check("0.2.0", Channel::Stable)).map(|r| r.map(|r| r.version))
    };
    assert_eq!(check(&["v0.3.1", "v0.3.0"], false).expect("check").as_deref(), Some("0.2.5"));
    // only the three newest are looked at
    assert_eq!(check(&["v0.3.2", "v0.3.1", "v0.3.0"], false).expect("check"), None);
    assert!(check(&[], true).is_err());
}
