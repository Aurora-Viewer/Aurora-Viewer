//! Render a URL with the installed media plugins and save one frame:
//! `cargo run -p aurora-media --example snapshot -- <url> <out.png> [seconds]`

use aurora_media::{BrowserSettings, MediaEvent, MediaPlugin, PluginPaths, mime_from_scheme, plugin_for_mime};
use std::time::{Duration, Instant};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();
    let url = args.get(1).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "data:text/html,<body style='background:%23302040;color:white;font:64px sans-serif'><h1>Aurora media</h1><p>CEF via SLPlugin</p></body>".into());
    let out = args.get(2).cloned().unwrap_or_else(|| "media_snapshot.png".into());
    let secs: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let paths = PluginPaths::discover(None).expect("no SLPlugin / llplugin found");
    println!("plugins: {paths:?}");
    let mime = mime_from_scheme(&url).unwrap_or_else(|| "text/html".into());
    let plugin = plugin_for_mime(&mime);
    let cache = std::env::temp_dir().join("aurora_media_test_cache");
    let browser = BrowserSettings {
        cache_path: format!("{}\\", cache.display()),
        ..Default::default()
    };
    let mut m = MediaPlugin::launch(&paths, plugin, 0, 0, &browser, "").expect("launch");
    let start = Instant::now();
    let mut loaded = false;
    let mut updates = 0;
    while start.elapsed() < Duration::from_secs(secs) {
        m.idle();
        if !loaded && m.is_running() {
            m.load_uri(&url);
            m.start(1.0);
            m.set_volume(0.2);
            loaded = true;
        }
        for e in m.take_events() {
            match e {
                MediaEvent::ContentUpdated => updates += 1,
                other => println!("event: {other:?}"),
            }
        }
        if m.failed() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "status {:?}, {updates} updates, media {:?}, texture {:?}, location {}",
        m.status(),
        m.media_size(),
        m.texture_size(),
        m.location
    );
    if let Some(f) = m.frame() {
        let (w, h) = (f.media_width as usize, f.media_height as usize);
        let tw = f.texture_width as usize;
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            let src_row = if f.bottom_up { h - 1 - y } else { y };
            for x in 0..w {
                let s = (src_row * tw + x) * 4;
                let d = (y * w + x) * 4;
                rgba[d] = f.data[s + 2];
                rgba[d + 1] = f.data[s + 1];
                rgba[d + 2] = f.data[s];
                rgba[d + 3] = 255;
            }
        }
        image::save_buffer(&out, &rgba, w as u32, h as u32, image::ColorType::Rgba8).expect("png");
        println!("saved {out}");
    } else {
        println!("no frame");
    }
}
