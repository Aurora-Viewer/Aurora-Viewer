//! Aurora Viewer — a Second Life client in Rust (wgpu / Vulkan).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod ao;
mod app;
mod bug_report;
mod build;
mod cache;
mod camera;
mod cli;
mod cmdline;
mod crash;
mod credentials;
mod cursors;
mod demo;
mod frame_profile;
mod interaction;
mod keybinds;
mod link_trust;
mod links;
mod logging;
mod media;
mod scene;
mod screen_rate;
mod settings;
mod slurl;
mod theme;
mod ui;
mod ui_sound;
mod updater;
mod voice;
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

    // Leave a core for the render thread; rayon handles decoding/meshing.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.saturating_sub(1).max(2))
        .thread_name(|i| format!("aurora-worker-{i}"))
        .build_global();

    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = app::App::new()?;
    event_loop.run_app(&mut app)?;
    Ok(())
}
