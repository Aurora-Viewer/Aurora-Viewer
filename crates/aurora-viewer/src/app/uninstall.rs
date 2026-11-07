//! The app's side of the removal from this computer (`uninstall.rs`,
//! `ui/uninstall.rs`): the plan when Preferences asks for it, the removal
//! on a thread of its own (it may start each Blender found, for minutes),
//! then the result window. `AURORA_DEMO_UNINSTALL` shows each step, as a dry
//! run.

use super::App;
use crate::ui::uninstall::{Action, UninstallUi};
use crate::uninstall::{self, Login, Report};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Duration;

#[derive(Default)]
pub(super) struct Uninstall {
    ui: UninstallUi,
    running: Option<Receiver<Report>>,
}

impl App {
    /// Preferences asked: the confirmation, with what this system holds.
    pub(super) fn uninstall_asked(&mut self) {
        let system = uninstall::system::for_run();
        let logins = if system.dry_run() {
            uninstall::system::demo_logins()
        } else {
            uninstall::saved_logins(Some(Login {
                grid: self.settings.login_uri(),
                user: self.settings.username.clone(),
            }))
        };
        self.uninstall.ui.confirm = Some(uninstall::plan(system.as_ref(), &logins));
    }

    /// The confirmation and the result window, drawn with the preferences.
    pub(super) fn uninstall_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if let Some(rx) = &self.uninstall.running {
            match rx.try_recv() {
                Ok(report) => {
                    log::info!(
                        "removal from this computer: finished{}{}",
                        if report.dry_run { " (dry run)" } else { "" },
                        if report.failed() { ", some parts not removed" } else { "" }
                    );
                    // shown again if it was closed while the removal ran
                    self.uninstall.ui.report = Some(report);
                    self.uninstall.ui.open = true;
                    self.uninstall.running = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(250)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.uninstall.running = None,
            }
        }
        if self.uninstall.ui.confirm.is_none() && !self.uninstall.ui.open {
            return;
        }
        if self.uninstall.ui.bases.is_empty() {
            self.uninstall.ui.bases = uninstall::known_bases();
        }
        for action in crate::ui::uninstall::show(ctx, p, &mut self.uninstall.ui) {
            match action {
                Action::Run(plan) => self.uninstall_start(plan, ctx),
                Action::OpenFolder(dir) => {
                    if let Err(e) = crate::open_path::open(&dir) {
                        log::warn!("removal from this computer: folder not opened ({e})");
                    }
                }
                Action::Quit => self.request_exit(),
            }
        }
    }

    fn uninstall_start(&mut self, plan: uninstall::Plan, ctx: &egui::Context) {
        self.uninstall.ui.open = true;
        if self.uninstall.running.is_some() {
            return;
        }
        self.uninstall.ui.report = None;
        // the cache folder of this run, and the one chosen for the next
        let mut chosen_cache: Vec<PathBuf> = crate::cache_location::chosen().map(Path::to_path_buf).into_iter().collect();
        if let Some(next) = crate::cache_location::root_for(&self.settings.cache_location)
            && !chosen_cache.contains(&next)
        {
            chosen_cache.push(next);
        }
        let transcripts = super::folders::chat_log_root(&self.settings.chat_log_location);
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("aurora-uninstall".into()).spawn(move || {
            let mut system = uninstall::system::for_run();
            log::info!(
                "removal from this computer: {} parts{}",
                plan.parts.len(),
                if system.dry_run() { " (dry run)" } else { "" }
            );
            let (steps, blender) = uninstall::run(&plan, system.as_mut());
            let remaining = uninstall::remaining(uninstall::folder_candidates(&chosen_cache, transcripts, &blender), Path::exists);
            let _ = tx.send(Report {
                steps,
                blender,
                remaining,
                dry_run: system.dry_run(),
            });
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.uninstall.running = Some(rx),
            Err(e) => log::warn!("removal from this computer: no thread ({e})"),
        }
    }

    /// `AURORA_DEMO_UNINSTALL=confirm|running|result|left`: the preferences
    /// on the action, then its confirmation, the result window while it
    /// runs, or once the dry run finished (`left`: scrolled to the folders).
    pub(super) fn uninstall_demo(&mut self, mode: &str) {
        self.panels.settings = true;
        crate::ui::options::demo(&mut self.options_ui, "system:remove-aurora");
        match mode {
            "running" => self.uninstall.ui.open = true,
            "result" | "left" => {
                self.uninstall.ui.scroll_to_left = if mode == "left" { 30 } else { 0 };
                self.uninstall_asked();
                if let Some(plan) = self.uninstall.ui.confirm.take() {
                    let ctx = self.egui_ctx.clone();
                    self.uninstall_start(plan, &ctx);
                }
            }
            _ => self.uninstall_asked(),
        }
    }
}
