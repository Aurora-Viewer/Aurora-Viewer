//! Aurora Viewer — a Second Life client in Rust (wgpu / Vulkan).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod app;
mod build;
mod cache;
mod camera;
mod cli;
mod cmdline;
mod credentials;
mod cursors;
mod demo;
mod frame_profile;
mod interaction;
mod keybinds;
mod links;
mod logging;
mod media;
mod scene;
mod settings;
mod slurl;
mod theme;
mod ui;
mod ui_sound;
mod voice;
mod world;

fn main() -> anyhow::Result<()> {
    cli::get();
    logging::init();

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
