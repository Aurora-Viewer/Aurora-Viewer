//! Aurora Viewer — a Second Life client in Rust (wgpu / Vulkan).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod allocator;
mod alt_codes;
mod ao;
mod app;
mod atomic_file;
mod autopilot;
mod autoreplace;
mod blender_live;
// Bug report form, disabled for now: "Report a bug" (login screen, top bar,
// crash window) opens GitHub's issue chooser in the browser instead
// (`links::NEW_ISSUE`). Its code stays in the tree, out of the build
// (`bug_report/`, `ui/bug_report.rs`, `app/bug_report.rs`,
// `crash/report.rs`): to bring it back, uncomment every line marked "Bug
// report form (disabled, see main.rs)".
// mod bug_report;
mod build;
mod cache;
mod cache_location;
mod camera;
mod cli;
mod cmdline;
mod colorblind;
mod crash;
mod creation_permissions;
mod credentials;
mod cursors;
mod deep_profile;
mod demo;
mod dev_toolbox;
mod disk_io;
mod dof;
mod external_url;
mod fast_hash;
mod frame_par;
mod frame_profile;
mod gestures;
mod gif_search;
mod gpu_driver;
mod graphics_presets;
mod grid_status;
mod grids;
mod high_contrast;
mod hover_tips;
mod ik_edit;
mod imports;
mod interaction;
mod interface_colors;
mod joystick;
mod keybinds;
mod keywords;
mod link_preview;
mod link_trust;
mod links;
mod logging;
mod login_accounts;
mod login_places;
mod login_wait;
mod lsl_keywords;
#[cfg(target_os = "macos")]
mod mac_quit;
mod machine_id;
mod media;
mod movement;
mod net_adapters;
mod net_profile;
mod overlay_hooks;
mod own_motion;
mod physics_editor;
mod raw_mouse;
mod report_abuse;
mod scene;
mod screen_rate;
mod settings;
mod shape_editor;
mod slapp;
mod slurl;
mod slurl_handoff;
mod snapshot;
mod spelling;
mod stream_title;
mod sys_probe;
mod system_info;
mod system_notify;
mod theme;
mod tolerant;
mod ui;
mod ui_sound;
mod updater;
mod url_scheme;
mod voice;
mod wearable_editor;
mod window_hold;
mod window_show;
mod world;

/// Hybrid-GPU hints (#894): on a PC with an integrated and a dedicated GPU,
/// the NVIDIA (Optimus) and AMD (PowerXpress) drivers run a program that
/// exports these on the dedicated one. The Legacy Viewer exports the same
/// (llgl.cpp). Under Vulkan the NVIDIA driver may not read them; the
/// renderer's own choice (`aurora_render::adapter_choice`) does the rest.
/// build.rs adds them to the export table (MSVC).
#[cfg(all(windows, target_env = "msvc"))]
mod gpu_hints {
    #[unsafe(no_mangle)]
    #[used]
    pub static NvOptimusEnablement: u32 = 1;
    #[unsafe(no_mangle)]
    #[used]
    pub static AmdPowerXpressRequestHighPerformance: i32 = 1;
}

fn main() -> anyhow::Result<()> {
    // Preferences › Graphics › Overlays, read from the file alone: nothing
    // else of the viewer has started yet
    let allow_overlays = settings::Settings::allow_overlays_at_start();
    // SAFETY: the first lines of main, before any other thread starts (the
    // read above starts none).
    let vulkan_layers_off = unsafe { aurora_render::vulkan_layers::disable(allow_overlays) };
    // SAFETY: as above, no other thread yet.
    let dozen_off = unsafe { aurora_render::vulkan_drivers::disable() };
    if aurora_updater::install::run_helper()? {
        return Ok(());
    }
    cli::get();
    anyhow::ensure!(
        std::env::var_os("AURORA_UPDATE_TEST_ROOT").is_none() || aurora_updater::test_environment().is_some(),
        "invalid isolated update test installation"
    );
    anyhow::ensure!(
        !aurora_version::update_test_build() || aurora_updater::test_environment().is_some(),
        "update fixture binary requires an isolated test installation"
    );
    if crash::helper()? {
        return Ok(());
    }
    // a link opened while a viewer runs goes to it, before this one opens
    // anything (LLAppViewer::initConfiguration, SLURLPassToOtherInstance)
    if let Some(url) = cli::get().url.as_deref()
        && external_url::classify(url) != external_url::Request::Ignored
        && slurl_handoff::hand_off(url)
    {
        return Ok(());
    }
    aurora_i18n::select("default", cli::get().language.as_deref());
    crash::start();
    logging::init();
    if !vulkan_layers_off.is_empty() {
        let names: Vec<_> = vulkan_layers_off.iter().map(|l| format!("{} ({})", l.name, l.program)).collect();
        log::info!(
            "Vulkan layers kept out of the viewer (overlays: crashes and lost images, #901): {}; overlays {} in Preferences › Graphics, AURORA_VULKAN_LAYERS=keep loads them all",
            names.join(", "),
            if allow_overlays { "allowed" } else { "not allowed" }
        );
    }
    if dozen_off {
        log::info!(
            "Vulkan driver kept out of the viewer: Dozen (Vulkan on Direct3D 12, from Microsoft's compatibility pack; crashed an old Direct3D 12 driver at start), {}={}; AURORA_VULKAN_DRIVERS=keep loads it",
            aurora_render::vulkan_drivers::DISABLE_VARIABLE,
            aurora_render::vulkan_drivers::DOZEN_GLOB
        );
    }
    // the hooks injected later (Discord's overlay…) cannot be kept out:
    // they are named in the log when they arrive
    overlay_hooks::start();

    let result = run_viewer();
    if let Err(error) = &result {
        crash::failure(&format!("Viewer startup / event loop error: {error:#}"));
    }
    crash::finished(result.is_ok());
    result
}

fn run_viewer() -> anyhow::Result<()> {
    if crash::demo() {
        return Ok(());
    }
    // the login's device ids, ready long before the first login
    machine_id::start();

    // The global pool: the workers that help the main thread in the frame's
    // parallel loops (frame_par.rs), and the UI's parallel searches. Leave a
    // core for the render thread; decoding and meshing have their own pool
    // (scene/jobs.rs).
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    // the frame's threads one step above normal (frame_priority.rs)
    use aurora_render::frame_priority::{PriorityMode, set_raised};
    set_raised(PriorityMode::from_env() != PriorityMode::Normal);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.saturating_sub(1).max(2))
        .thread_name(|i| format!("aurora-worker-{i}"))
        .build_global();

    let mut event_loop = winit::event_loop::EventLoop::builder();
    // the absolute positions of remote desktops made motions (raw_mouse.rs)
    #[cfg(windows)]
    raw_mouse::install(&mut event_loop);
    let event_loop = event_loop.build()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    // the cache folder of this run, before anything uses it
    cache_location::init();
    // private browser profiles a crash left behind (search web tab)
    let private_profiles = settings::cache_dir().join(media::private_profile::ROOT);
    let _ = std::thread::Builder::new()
        .name("aurora-web-profile".into())
        .spawn(move || media::private_profile::sweep(&private_profiles));
    // the browser profiles of the earlier runs: set aside before the first
    // browser starts, deleted in the background (cef_cache.rs)
    let browser_cache = media::browser_cache(&settings::cache_dir());
    media::cef_cache::set_aside(&browser_cache);
    let _ = std::thread::Builder::new()
        .name("aurora-web-cache".into())
        .spawn(move || media::cef_cache::purge(&browser_cache));
    // the browser profiles of the earlier builds, kept in the wrong folder
    #[cfg(unix)]
    {
        let cache = settings::cache_dir();
        let _ = std::thread::Builder::new()
            .name("aurora-web-misplaced".into())
            .spawn(move || media::remove_misplaced_cache(&cache));
    }
    let mut app = app::App::new()?;
    let result = event_loop.run_app(app.as_mut());
    // later launches no longer hand their link to this closing viewer
    slurl_handoff::release();
    // the browsers stop with the app; their private profiles are removed
    // while the plugins let go of their files
    drop(app);
    media::private_profile::finish(std::time::Duration::from_secs(8));
    result?;
    Ok(())
}
