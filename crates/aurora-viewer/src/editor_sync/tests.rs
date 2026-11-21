use super::*;
use aurora_editor_sync::subscriptions::script_id;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("test runtime")
}

/// A port nothing listens on (taken and released at once).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port")
}

fn hello() -> Hello {
    Hello {
        viewer_name: "Aurora Viewer".into(),
        viewer_version: "0.0.0".into(),
        agent_id: Uuid::from_u128(9),
        agent_name: "Test".into(),
        syntax_id: Uuid::nil(),
        commands: false,
    }
}

fn target(n: u128) -> ScriptTarget {
    ScriptTarget {
        object: Uuid::from_u128(100),
        item: Uuid::from_u128(n),
    }
}

fn frame<'a>(enabled: bool, port: u16, scripts: Vec<Synced<'a>>) -> Frame<'a> {
    Frame {
        enabled,
        port,
        forward_debug: true,
        logged_in: true,
        scripts,
        compiled: Vec::new(),
        hello: hello(),
        now: Instant::now(),
    }
}

fn synced(n: u128, can_edit: bool) -> Synced<'static> {
    Synced {
        target: target(n),
        name: "door",
        lua: false,
        can_edit,
    }
}

#[test]
fn the_server_starts_with_the_first_script_sent_and_stops_with_the_setting() {
    let rt = runtime();
    let port = free_port();
    let mut sync = EditorSync::default();
    let mut chat = ChatTap::default();
    let root = |_| None;
    // on, but nothing sent to the external editor yet (ensureServerRunning
    // is called by openInExternalEditor)
    assert!(sync.update(rt.handle(), frame(true, port, Vec::new()), &mut chat, root).is_empty());
    assert!(!sync.port().is_some());
    // off: a script sent does not start it
    assert!(
        sync.update(rt.handle(), frame(false, port, vec![synced(1, true)]), &mut chat, root)
            .is_empty()
    );
    assert!(!sync.port().is_some());
    // on with a script sent
    assert_eq!(
        sync.update(rt.handle(), frame(true, port, vec![synced(1, true)]), &mut chat, root),
        [Tip::Started(port)]
    );
    assert!(sync.port().is_some());
    assert!(
        sync.update(rt.handle(), frame(true, port, vec![synced(1, true)]), &mut chat, root)
            .is_empty()
    );
    // off: stopped and the port is free again
    assert_eq!(
        sync.update(rt.handle(), frame(false, port, vec![synced(1, true)]), &mut chat, root),
        [Tip::Stopped]
    );
    assert!(!sync.port().is_some());
    // back on: the script is still sent, it starts again, on the same
    // port once the stopped server has let go of it (no failure tip)
    let mut tips = Vec::new();
    for _ in 0..100 {
        tips.extend(sync.update(rt.handle(), frame(true, port, vec![synced(1, true)]), &mut chat, root));
        if sync.port().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(tips, [Tip::Started(port)]);
    // logout: stopped without a tip, and forgotten
    let mut out = frame(true, port, vec![synced(1, true)]);
    out.logged_in = false;
    assert!(sync.update(rt.handle(), out, &mut chat, root).is_empty());
    assert!(!sync.port().is_some());
    assert!(sync.update(rt.handle(), frame(true, port, Vec::new()), &mut chat, root).is_empty());
    assert!(!sync.port().is_some());
}

#[test]
fn a_taken_port_fails_once_and_is_tried_again_with_the_next_script() {
    let rt = runtime();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("a listener");
    let port = taken.local_addr().expect("its address").port();
    let mut sync = EditorSync::default();
    let mut chat = ChatTap::default();
    let root = |_| None;
    assert_eq!(
        sync.update(rt.handle(), frame(true, port, vec![synced(1, true)]), &mut chat, root),
        [Tip::Failed(port)]
    );
    // not every frame
    assert!(
        sync.update(rt.handle(), frame(true, port, vec![synced(1, true)]), &mut chat, root)
            .is_empty()
    );
    // a second script sent: tried again
    assert_eq!(
        sync.update(
            rt.handle(),
            frame(true, port, vec![synced(1, true), synced(2, true)]),
            &mut chat,
            root
        ),
        [Tip::Failed(port)]
    );
    drop(taken);
}

#[test]
fn only_a_script_that_may_be_edited_is_offered_and_kept_while_it_is_open() {
    let rt = runtime();
    let port = free_port();
    let mut sync = EditorSync::default();
    let mut chat = ChatTap::default();
    let root = |_| None;
    // a script sent while it may not be edited never starts the server
    assert!(
        sync.update(rt.handle(), frame(true, port, vec![synced(1, false)]), &mut chat, root)
            .is_empty()
    );
    assert!(!sync.port().is_some());
    sync.update(
        rt.handle(),
        frame(true, port, vec![synced(1, false), synced(2, true)]),
        &mut chat,
        root,
    );
    assert!(sync.port().is_some());
    assert_eq!(sync.subs.ids(), [script_id(target(2).object, target(2).item)]);
    // once offered, a passing loss of the rights (contents reloading) keeps it
    sync.update(rt.handle(), frame(true, port, vec![synced(2, false)]), &mut chat, root);
    assert_eq!(sync.subs.ids().len(), 1);
    // its editor closed or stopped: gone
    sync.update(rt.handle(), frame(true, port, Vec::new()), &mut chat, root);
    assert!(sync.subs.ids().is_empty());
}

#[test]
fn a_script_let_go_of_is_offered_again_only_while_it_may_be_edited() {
    let rt = runtime();
    let port = free_port();
    let mut sync = EditorSync::default();
    let mut chat = ChatTap::default();
    let root = |_| None;
    sync.update(rt.handle(), frame(true, port, vec![synced(1, true)]), &mut chat, root);
    let sid = script_id(target(1).object, target(1).item);
    assert_eq!(sync.subs.ids(), std::slice::from_ref(&sid));
    // the editor let go of it (script.unsubscribe by its follower)
    sync.subs.subscribe(3, &sid, |_| true).expect("followed");
    sync.subs.unsubscribe(3, &sid);
    // its rights are gone meanwhile: not offered again
    sync.update(rt.handle(), frame(true, port, vec![synced(1, false)]), &mut chat, root);
    assert!(sync.subs.ids().is_empty());
    // they are back: offered again
    sync.update(rt.handle(), frame(true, port, vec![synced(1, true)]), &mut chat, root);
    assert_eq!(sync.subs.ids(), [sid]);
}

#[test]
fn a_new_port_is_used_once_it_settles_and_a_low_one_is_the_default() {
    assert_eq!(listen_port(0), crate::ui::scripts::settings::WEBSOCKET_PORT);
    assert_eq!(listen_port(80), crate::ui::scripts::settings::WEBSOCKET_PORT);
    assert_eq!(listen_port(1024), 1024);
    let rt = runtime();
    let (first, second, third) = (free_port(), free_port(), free_port());
    let mut sync = EditorSync::default();
    let mut chat = ChatTap::default();
    let root = |_| None;
    let start = Instant::now();
    let at = |port: u16, ms: u64| {
        let mut f = frame(true, port, vec![synced(1, true)]);
        f.now = start + Duration::from_millis(ms);
        f
    };
    assert_eq!(sync.update(rt.handle(), at(first, 0), &mut chat, root), [Tip::Started(first)]);
    // a field being dragged: every value it passes stays unused
    assert!(sync.update(rt.handle(), at(second, 100), &mut chat, root).is_empty());
    assert!(sync.update(rt.handle(), at(third, 200), &mut chat, root).is_empty());
    assert!(sync.update(rt.handle(), at(third, 600), &mut chat, root).is_empty());
    assert_eq!(sync.port(), Some(first));
    // settled: the server moves there, once
    assert_eq!(sync.update(rt.handle(), at(third, 1000), &mut chat, root), [Tip::Started(third)]);
    assert_eq!(sync.port(), Some(third));
    assert!(sync.update(rt.handle(), at(third, 2000), &mut chat, root).is_empty());
    // back to the port it listens on before settling: nothing moves
    assert!(sync.update(rt.handle(), at(first, 2100), &mut chat, root).is_empty());
    assert!(sync.update(rt.handle(), at(third, 2200), &mut chat, root).is_empty());
    assert!(sync.update(rt.handle(), at(third, 5000), &mut chat, root).is_empty());
    assert_eq!(sync.port(), Some(third));
}

#[test]
fn requests_follow_the_legacy_viewer_and_refuse_unknown_files() {
    let rt = runtime();
    let server = Server::start(
        rt.handle(),
        Config {
            port: 0,
            limits: Limits::default(),
            methods: METHODS,
        },
        hello(),
    )
    .expect("a server");
    let mut subs = Subscriptions::default();
    let sid = subs.offer(target(1).object, target(1).item, "door", false, target(1));
    let id = json!(1);
    let jobs = Arc::new(AtomicUsize::new(0));
    let call = Call {
        conn: 7,
        id: &id,
        responder: server.responder(),
        runtime: rt.handle(),
        syntax_id: Uuid::nil(),
        jobs: &jobs,
    };
    let open = |t: &ScriptTarget| *t == target(1);
    let root = |_| Some(Uuid::from_u128(200));
    let ask = |subs: &mut Subscriptions<ScriptTarget>, method: &str, params: Value| answer(subs, &call, method, &params, &open, &root);
    let reply = ask(&mut subs, "script.subscribe", json!({ "script_id": sid }))
        .expect("answered")
        .expect("ok");
    assert_eq!(reply["success"], json!(true));
    assert_eq!(reply["root_id"], json!(Uuid::from_u128(200).to_string()));
    // an id the viewer did not offer
    let reply = ask(&mut subs, "script.subscribe", json!({ "script_id": "0".repeat(32) }))
        .expect("answered")
        .expect("ok");
    assert_eq!(reply["status"], json!(2));
    // connection 7 is not one of this server's: its script was let go of
    assert_eq!(subs.connection(&sid), None);
    let reply = ask(&mut subs, "script.subscribe", json!({})).expect("answered").expect("ok");
    assert_eq!(reply["success"], json!(false));
    let list = ask(&mut subs, "script.list", json!({})).expect("answered").expect("ok");
    assert_eq!(list["script_ids"], json!([sid.clone()]));
    // with its separator at the end, as LLFile::tmpdir
    assert!(
        list["temp_dir"]
            .as_str()
            .is_some_and(|d| d.len() > 1 && d.ends_with(std::path::MAIN_SEPARATOR))
    );
    assert_eq!(
        ask(&mut subs, "language.syntax.id", json!({})).expect("answered").expect("ok")["id"],
        json!(Uuid::nil().to_string())
    );
    // only the two keyword files, by their exact names
    for name in [
        "",
        "../lsl_keywords.xml",
        "lsl_keywords.xml/..",
        "LUA_KEYWORDS.XML",
        "C:\\Windows\\win.ini",
        "keywords_lsl_default.xml",
    ] {
        let reply = ask(&mut subs, "language.syntax.get", json!({ "filename": name })).expect("answered");
        assert_eq!(
            reply.map_err(|e| e.code),
            Err(aurora_editor_sync::rpc::code::INVALID_PARAMS),
            "{name}"
        );
    }
    assert_eq!(
        ask(&mut subs, "language.syntax", json!({ "kind": "defs.cobol" }))
            .expect("answered")
            .map_err(|e| e.code),
        Err(aurora_editor_sync::rpc::code::INVALID_PARAMS)
    );
    // read on another thread
    assert!(ask(&mut subs, "language.syntax", json!({ "kind": "defs.lua" })).is_none());
    // as many reads at once as allowed, then busy
    jobs.store(MAX_SYNTAX_JOBS, Ordering::Release);
    assert_eq!(
        ask(&mut subs, "language.syntax", json!({ "kind": "defs.lua" })).map(|r| r.map_err(|e| e.code)),
        Some(Err(RpcError::unavailable().code))
    );
    assert_eq!(jobs.load(Ordering::Acquire), MAX_SYNTAX_JOBS);
    jobs.store(0, Ordering::Release);
    // another connection cannot let it go
    subs.subscribe(7, &sid, |_| true).expect("followed again");
    let other = Call { conn: 8, ..call };
    assert_eq!(
        answer(&mut subs, &other, "script.unsubscribe", &json!({ "script_id": sid }), &open, &root),
        Some(Ok(Value::Null))
    );
    assert_eq!(subs.connection(&sid), Some(7));
}

#[test]
fn llsd_becomes_the_legacy_json() {
    let mut map = aurora_llsd::Map::new();
    map.insert("b".into(), Llsd::Boolean(true));
    map.insert("i".into(), Llsd::Integer(-3));
    map.insert("r".into(), Llsd::Real(1.5));
    map.insert("nan".into(), Llsd::Real(f64::NAN));
    map.insert("u".into(), Llsd::Uuid(Uuid::from_u128(1)));
    map.insert("d".into(), Llsd::Date(951_782_400.0));
    map.insert("bin".into(), Llsd::Binary(vec![1, 2]));
    map.insert(
        "a".into(),
        Llsd::Array(vec![Llsd::Undef, Llsd::String("x".into()), Llsd::Uri("http://a".into())]),
    );
    assert_eq!(
        llsd_json(&Llsd::Map(map)),
        json!({
            "b": true, "i": -3, "r": 1.5, "nan": null,
            "u": "00000000-0000-0000-0000-000000000001",
            "d": "2000-02-29T00:00:00Z", "bin": null,
            "a": [null, "x", "http://a"],
        })
    );
    assert_eq!(iso_date(0.0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_date(-1.0), "1969-12-31T23:59:59Z");
}

#[test]
fn the_bundled_slua_keywords_are_served_as_json() {
    let doc = Source::BundledLua.llsd().expect("the bundled file");
    let defs = llsd_json(&doc);
    assert!(defs.is_object());
}

#[test]
fn the_agent_name_is_the_legacy_username() {
    assert_eq!(agent_name("Ann", "Resident"), "Ann");
    assert_eq!(agent_name("Ann", ""), "Ann");
    assert_eq!(agent_name("Ann", "Smith"), "Ann Smith");
}

#[test]
fn only_object_debug_and_owner_lines_are_tapped_and_bounded() {
    let mut tap = ChatTap::default();
    let line = |source_type, chat_type| aurora_net::ChatMessage {
        from_name: "Box".into(),
        source_id: Uuid::from_u128(1),
        owner_id: Uuid::from_u128(2),
        source_type,
        chat_type,
        region: None,
        position: Default::default(),
        message: "hello".into(),
    };
    use aurora_net::{ChatSourceType as S, ChatType as T};
    tap.offer(&line(S::Object, T::Debug));
    assert!(tap.take().is_empty(), "off");
    tap.on = true;
    tap.offer(&line(S::Object, T::Debug));
    tap.offer(&line(S::Object, T::OwnerSay));
    tap.offer(&line(S::Object, T::Normal));
    tap.offer(&line(S::Agent, T::Debug));
    assert_eq!(tap.take().len(), 2);
    for _ in 0..MAX_CHAT + 10 {
        tap.offer(&line(S::Object, T::Debug));
    }
    assert_eq!(tap.take().len(), MAX_CHAT);
}
