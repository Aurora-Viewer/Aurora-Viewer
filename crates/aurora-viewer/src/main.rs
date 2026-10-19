//! Aurora Viewer — a Second Life client in Rust (wgpu / Vulkan).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod alt_codes;
mod ao;
mod app;
mod atomic_file;
mod autopilot;
// Bug report form, disabled for now: "Report a bug" (login screen, top bar,
// crash window) opens GitHub's issue chooser in the browser instead
// (`links::NEW_ISSUE`). Its code stays in the tree, out of the build
// (`bug_report/`, `ui/bug_report.rs`, `app/bug_report.rs`,
// `crash/report.rs`): to bring it back, uncomment every line marked "Bug
// report form (disabled, see main.rs)".
// mod bug_report;
mod build;
mod cache;
mod camera;
mod cli;
mod cmdline;
mod colorblind;
mod crash;
mod credentials;
mod cursors;
mod deep_profile;
mod demo;
mod disk_io;
mod dof;
mod frame_par;
mod frame_profile;
mod gestures;
mod graphics_presets;
mod imports;
mod interaction;
mod keybinds;
mod link_trust;
mod links;
mod logging;
mod login_accounts;
mod login_places;
mod media;
mod movement;
mod net_profile;
mod own_motion;
mod physics_editor;
mod report_abuse;
mod scene;
mod screen_rate;
mod settings;
mod shape_editor;
mod slurl;
mod snapshot;
mod sys_probe;
mod system_notify;
mod theme;
mod tolerant;
mod ui;
mod ui_sound;
mod updater;
mod voice;
mod window_hold;
mod window_show;
mod world;

fn main() -> anyhow::Result<()> {
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
    aurora_i18n::select("default", cli::get().language.as_deref());
    crash::start();
    logging::init();

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

    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    // private browser profiles a crash left behind (search web tab)
    let private_profiles = settings::cache_dir().join(media::private_profile::ROOT);
    let _ = std::thread::Builder::new()
        .name("aurora-web-profile".into())
        .spawn(move || media::private_profile::sweep(&private_profiles));
    let mut app = app::App::new()?;
    let result = event_loop.run_app(&mut app);
    // the browsers stop with the app; their private profiles are removed
    // while the plugins let go of their files
    drop(app);
    media::private_profile::finish(std::time::Duration::from_secs(8));
    result?;
    Ok(())
}
