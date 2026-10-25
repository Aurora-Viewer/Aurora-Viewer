use super::*;

fn viewer() -> ViewerSide {
    ViewerSide {
        version: "Aurora Viewer 0.7.5.0".into(),
        language: "fr".into(),
        run_time: 400.0,
        foreground_time: 200.0,
        foreground_frames: 12_000,
        frames: 24_000,
        render_time: 390.0,
        jitter: Jitter {
            session: 0.1,
            frametime_variance: 0.2,
            period: 0.3,
        },
        agents_in_view: 3,
        meters_traveled: 42.5,
        mem_use_kb: 1_048_576.0,
        system: SystemInfo {
            ram_kb: 33_554_432,
            os: "Windows 11 Pro".into(),
            cpu: "AMD Ryzen 9 7950X (4500 MHz)".into(),
            cpu_sse: vec!["1".into(), "2".into(), "3".into()],
            hardware_concurrency: 32,
            // made-up values, never a real machine's
            mac_address: "02-00-5e-10-20-30".into(),
            serial_number: "b".repeat(32),
            gpu: GpuInfo {
                name: "NVIDIA GeForce RTX 4090".into(),
                vendor_id: 0x10de,
                driver: "NVIDIA 581.57".into(),
                api: "Vulkan".into(),
                vram_mb: 24_564,
                max_texture_size: 16_384,
            },
        },
        shader_level: 5,
        display_names: true,
        show_usernames: false,
        inventory_usable: true,
    }
}

fn kind(v: &Llsd) -> &'static str {
    match v {
        Llsd::Undef => "undef",
        Llsd::Boolean(_) => "bool",
        Llsd::Integer(_) => "int",
        Llsd::Real(_) => "real",
        Llsd::String(_) => "string",
        Llsd::Uuid(_) => "uuid",
        Llsd::Date(_) => "date",
        Llsd::Uri(_) => "uri",
        Llsd::Binary(_) => "binary",
        Llsd::Array(_) => "array",
        Llsd::Map(_) => "map",
    }
}

fn at<'a>(v: &'a Llsd, path: &str) -> &'a Llsd {
    path.split('.').fold(v, |v, key| &v[key])
}

/// Every field of send_viewer_stats and LLViewerStats::addToMessage, with
/// the LLSD type the Legacy Viewer gives it (S32 → int, F32 / F64 → real).
#[test]
fn the_report_has_every_field_of_the_legacy_viewer_with_its_type() {
    let report = body(&viewer(), &NetCounters::default(), 61.5, 1_800_000_000, 2);
    let fields = [
        ("agent.start_time", "int"),
        ("agent.run_time", "real"),
        ("agent.foreground_time", "real"),
        ("agent.fps", "real"),
        ("agent.normalized_session_jitter", "real"),
        ("agent.normalized_frametime_variance", "real"),
        ("agent.normalized_period_jitter", "real"),
        ("agent.version", "string"),
        ("agent.language", "string"),
        ("agent.sim_fps", "real"),
        ("agent.agents_in_view", "int"),
        ("agent.ping", "real"),
        ("agent.meters_traveled", "real"),
        ("agent.regions_visited", "int"),
        ("agent.mem_use", "real"),
        ("agent.translation.on", "bool"),
        ("agent.translation.chars_seen", "int"),
        ("system.ram", "int"),
        ("system.os", "string"),
        ("system.cpu", "string"),
        ("system.cpu_sse", "array"),
        ("system.address_size", "int"),
        ("system.os_bitness", "int"),
        ("system.hardware_concurrency", "int"),
        ("system.mac_address", "string"),
        ("system.serial_number", "string"),
        ("system.gpu", "string"),
        ("system.gpu_class", "int"),
        ("system.gpu_memory_bandwidth", "real"),
        ("system.gpu_vendor", "string"),
        ("system.gpu_version", "string"),
        ("system.opengl_version", "string"),
        ("system.gl.gpu_vendor", "string"),
        ("system.gl.gpu_version", "string"),
        ("system.gl.opengl_version", "string"),
        ("system.gl.vram", "int"),
        ("system.gl.max_samples", "int"),
        ("system.gl.num_texture_image_units", "int"),
        ("system.gl.max_sample_mask_words", "int"),
        ("system.gl.max_color_texture_samples", "int"),
        ("system.gl.max_depth_texture_samples", "int"),
        ("system.gl.max_integer_samples", "int"),
        ("system.gl.max_vertex_range", "int"),
        ("system.gl.max_index_range", "int"),
        ("system.gl.max_texture_size", "int"),
        ("system.gl.is_ati", "bool"),
        ("system.gl.is_nvidia", "bool"),
        ("system.gl.is_intel", "bool"),
        ("system.gl.gl_renderer", "string"),
        ("system.shader_level", "int"),
        ("downloads.world_kbytes", "real"),
        ("downloads.object_kbytes", "real"),
        ("downloads.texture_kbytes", "real"),
        ("downloads.mesh_kbytes", "real"),
        ("stats.net.in.kbytes", "real"),
        ("stats.net.in.packets", "int"),
        ("stats.net.in.compressed_packets", "int"),
        ("stats.net.in.savings", "real"),
        ("stats.net.out.kbytes", "real"),
        ("stats.net.out.packets", "int"),
        ("stats.net.out.compressed_packets", "int"),
        ("stats.net.out.savings", "real"),
        ("stats.failures.send_packet", "int"),
        ("stats.failures.dropped", "int"),
        ("stats.failures.resent", "int"),
        ("stats.failures.failed_resends", "int"),
        ("stats.failures.off_circuit", "int"),
        ("stats.failures.invalid", "int"),
        ("stats.failures.missing_updater", "bool"),
        ("stats.misc.string_1", "string"),
        ("stats.misc.string_2", "string"),
        ("stats.misc.int_1", "int"),
        ("stats.misc.int_2", "int"),
        ("inventory.usable", "bool"),
        ("inventory.validation_info", "map"),
        ("DisplayNamesEnabled", "bool"),
        ("DisplayNamesShowUsername", "bool"),
        ("MinimalSkin", "bool"),
        ("misc.Version", "bool"),
        ("misc.Vertex Buffers Enabled", "real"),
        ("AgentPositionSnaps", "real"),
    ];
    for (path, expected) in fields {
        let value = if path == "misc.Vertex Buffers Enabled" {
            &report["misc"]["Vertex Buffers Enabled"]
        } else {
            at(&report, path)
        };
        assert_eq!(kind(value), expected, "{path}");
    }
    if cfg!(windows) {
        assert_eq!(kind(at(&report, "stats.misc.VulkanMaxApiVersion")), "string");
    }
    // the session id is added by the caller, after any logging
    assert_eq!(kind(&report["session_id"]), "undef");
    // and nothing else: the Legacy Viewer sends no preferences and no `ui`
    let Llsd::Map(top) = &report else { panic!("a map") };
    let keys: Vec<&str> = top.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "AgentPositionSnaps",
            "DisplayNamesEnabled",
            "DisplayNamesShowUsername",
            "MinimalSkin",
            "agent",
            "downloads",
            "inventory",
            "misc",
            "stats",
            "system"
        ]
    );
}

#[test]
fn the_report_carries_the_values_it_was_given() {
    let counters = NetCounters {
        bytes_in: 10 * 1024,
        packets_in: 300,
        zero_coded_in: 100,
        zero_coded_bytes_in: 4 * 1024,
        zero_decoded_bytes_in: 6 * 1024,
        bytes_out: 2048,
        packets_out: 50,
        send_failures: 1,
        dropped: 2,
        resent: 3,
        failed_resends: 4,
        off_circuit: 5,
        invalid: 6,
        texture_bytes: 1024 * 1024,
        mesh_bytes: 512,
        ping_ms: 87.0,
    };
    let report = body(&viewer(), &counters, 61.5, 1_800_000_000, 2);
    assert_eq!(at(&report, "agent.start_time").as_i32(), 1_800_000_000 - 400);
    assert_eq!(at(&report, "agent.run_time").as_f64(), 400.0);
    assert_eq!(at(&report, "agent.fps").as_f64(), 60.0);
    assert_eq!(at(&report, "agent.sim_fps").as_f64(), 61.5);
    assert_eq!(at(&report, "agent.ping").as_f64(), 87.0);
    assert_eq!(at(&report, "agent.regions_visited").as_i32(), 2);
    assert_eq!(at(&report, "agent.version").as_str(), "Aurora Viewer 0.7.5.0");
    assert_eq!(at(&report, "system.gpu").as_str(), "NVIDIA Class -1 NVIDIA NVIDIA GeForce RTX 4090");
    assert_eq!(at(&report, "system.gpu_vendor").as_str(), "NVIDIA");
    assert!(at(&report, "system.gl.is_nvidia").as_bool());
    assert_eq!(at(&report, "system.opengl_version").as_str(), "Vulkan");
    assert_eq!(at(&report, "downloads.texture_kbytes").as_f64(), 1024.0);
    assert_eq!(at(&report, "downloads.mesh_kbytes").as_f64(), 0.5);
    assert_eq!(at(&report, "stats.net.in.kbytes").as_f64(), 10.0);
    assert_eq!(at(&report, "stats.net.in.compressed_packets").as_i32(), 100);
    assert_eq!(at(&report, "stats.net.in.savings").as_f64(), 2.0);
    assert_eq!(at(&report, "stats.failures.dropped").as_i32(), 2);
    assert_eq!(at(&report, "stats.failures.failed_resends").as_i32(), 4);
    assert_eq!(at(&report, "stats.failures.invalid").as_i32(), 6);
}

#[test]
fn a_report_from_the_first_half_period_says_run_time_zero() {
    let mut early = viewer();
    early.run_time = 149.0;
    let report = body(&early, &NetCounters::default(), 0.0, 1_800_000_000, 1);
    assert_eq!(at(&report, "agent.run_time").as_f64(), 0.0);
    // the start time is still the real one
    assert_eq!(at(&report, "agent.start_time").as_i32(), 1_800_000_000 - 149);
    // no foreground time yet: no division by zero
    let mut idle = viewer();
    idle.foreground_time = 0.0;
    let report = body(&idle, &NetCounters::default(), 0.0, 1_800_000_000, 1);
    assert_eq!(at(&report, "agent.fps").as_f64(), 0.0);
}

#[test]
fn reports_leave_on_arrival_then_every_five_minutes() {
    let mut s = Schedule::default();
    let t0 = Instant::now();
    assert!(!s.due(t0, false), "not before the agent arrived");
    assert!(s.due(t0, true), "at once on arrival");
    assert_eq!(s.sent(t0, 600, 10.0), 60.0, "sim_fps from the start");
    assert!(!s.due(t0 + Duration::from_secs(299), true));
    let t1 = t0 + SEND_PERIOD;
    assert!(s.due(t1, true));
    // 300 s later, 9000 more frames
    assert_eq!(s.sent(t1, 9600, 310.0), 30.0, "sim_fps since the previous report");
    assert!(!s.due(t1 + Duration::from_secs(1), true));
}

#[test]
fn frame_jitter_follows_update_frame_stats() {
    // steady frames: no jitter
    let mut steady = FrameJitter::default();
    for _ in 0..400 {
        steady.frame(1.0 / 60.0);
    }
    assert_eq!(steady.stats.session, 0.0);
    assert_eq!(steady.stats.period, 0.0);
    // alternating 10 ms / 30 ms frames: every frame after the first jitters (the first frame times
    // nothing) by 20 ms, over a mean frame time of 20 ms
    let mut uneven = FrameJitter::default();
    for i in 0..600 {
        uneven.frame(if i % 2 == 0 { 0.010 } else { 0.030 });
    }
    assert!((uneven.stats.session - 1.0).abs() < 1e-2, "{}", uneven.stats.session);
    assert!((uneven.stats.period - 1.0).abs() < 1e-2, "{}", uneven.stats.period);
    // the jitter is constant: its deviation, so the "variance", is 0
    assert!(uneven.stats.frametime_variance.abs() < 1e-6, "{}", uneven.stats.frametime_variance);
}

#[test]
fn the_vendor_short_name_comes_from_the_pci_id() {
    let gpu = |vendor_id| GpuInfo {
        vendor_id,
        ..GpuInfo::default()
    };
    assert_eq!(gpu(0x10de).vendor_short(), "NVIDIA");
    assert_eq!(gpu(0x1002).vendor_short(), "AMD");
    assert_eq!(gpu(0x8086).vendor_short(), "INTEL");
    assert_eq!(gpu(0x5143).vendor_short(), "MISC");
}

#[test]
fn dropped_packets_are_the_gaps_no_late_packet_filled() {
    let stats = NetStats::default();
    stats.seq_gaps.store(10, Ordering::Relaxed);
    stats.seq_late.store(7, Ordering::Relaxed);
    stats.ping_avg_ms.store(120, Ordering::Relaxed);
    let n = NetCounters::read(&stats);
    assert_eq!(n.dropped, 3);
    assert_eq!(n.ping_ms, 120.0);
    // a late packet counted before its gap never wraps the counter
    stats.seq_late.store(12, Ordering::Relaxed);
    assert_eq!(NetCounters::read(&stats).dropped, 0);
}

#[tokio::test]
async fn a_post_that_fails_is_only_logged() {
    let http = reqwest::Client::new();
    let report = body(&viewer(), &NetCounters::default(), 0.0, 0, 0);
    // nothing listens on the discard port of localhost
    assert!(!post(&http, "http://127.0.0.1:9/cap", &report).await);
}

#[test]
fn the_adapter_address_is_formatted_like_send_viewer_stats() {
    // a made-up, locally administered address
    assert_eq!(mac_address_string(Some([0x02, 0x00, 0x5e, 0x10, 0x20, 0x3f])), "02-00-5e-10-20-3f");
    assert_eq!(mac_address_string(None), "00-00-00-00-00-00");
}

#[test]
fn the_logged_report_hides_the_machine_identifiers() {
    let report = body(&viewer(), &NetCounters::default(), 0.0, 0, 0);
    // the report itself carries them
    assert_eq!(at(&report, "system.mac_address").as_str(), "02-00-5e-10-20-30");
    assert_eq!(at(&report, "system.serial_number").as_str(), "b".repeat(32));
    // its logged copy does not, and keeps the rest
    let logged = aurora_llsd::to_xml_string(&loggable(&report));
    assert!(!logged.contains("02-00-5e-10-20-30"), "{logged}");
    assert!(!logged.contains(&"b".repeat(32)), "{logged}");
    assert!(logged.contains("NVIDIA GeForce RTX 4090"), "{logged}");
}

#[tokio::test]
async fn any_2xx_answer_is_a_post_even_with_an_empty_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!("http://{}/cap", listener.local_addr().expect("address"));
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0; 65536];
            let _ = stream.read(&mut buf).await;
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        }
    });
    let http = reqwest::Client::new();
    let report = body(&viewer(), &NetCounters::default(), 0.0, 0, 0);
    assert!(post(&http, &url, &report).await);
}

/// A capability that answers after `delay`, served by `rt`.
fn slow_capability(rt: &tokio::runtime::Runtime, delay: Duration) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).expect("listener");
    let url = format!("http://{}/cap", listener.local_addr().expect("address"));
    rt.spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0; 65536];
                let _ = stream.read(&mut buf).await;
                tokio::time::sleep(delay).await;
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
            });
        }
    });
    url
}

/// Post one report the way the session does, counted in `posts`.
fn post_counted(rt: &tokio::runtime::Runtime, posts: &std::sync::Arc<PostsInFlight>, url: String) {
    let posts = posts.clone();
    let report = body(&viewer(), &NetCounters::default(), 0.0, 0, 0);
    posts.begin();
    rt.spawn(async move {
        post(&reqwest::Client::new(), &url, &report).await;
        posts.end();
    });
}

#[test]
fn the_exit_waits_for_the_last_report_up_to_its_bound_and_no_longer() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let bound = Duration::from_secs(2);

    // nothing being posted: no wait at all
    let posts = std::sync::Arc::new(PostsInFlight::default());
    let start = Instant::now();
    assert!(posts.wait(bound));
    assert!(start.elapsed() < Duration::from_millis(50), "{:?}", start.elapsed());

    // a capability answering in 300 ms: the exit waits that long, not 2 s
    let fast = slow_capability(&rt, Duration::from_millis(300));
    post_counted(&rt, &posts, fast);
    let start = Instant::now();
    assert!(posts.wait(bound), "the report finished");
    let waited = start.elapsed();
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_millis(1500),
        "{waited:?}"
    );

    // one answering in 10 s: the exit gives up at the bound
    let slow = slow_capability(&rt, Duration::from_secs(10));
    post_counted(&rt, &posts, slow);
    let start = Instant::now();
    assert!(!posts.wait(bound), "still being posted");
    let waited = start.elapsed();
    assert!(waited >= bound && waited < bound + Duration::from_millis(800), "{waited:?}");
    rt.shutdown_background();
}
