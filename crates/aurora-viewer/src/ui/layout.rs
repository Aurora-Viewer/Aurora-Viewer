//! UI layout kept between sessions (Firestorm saves floater rects): egui's
//! memory (window positions and sizes, collapsed floaters) is written to
//! `ui_layout.ron` on exit and restored at startup. Which panels are open
//! lives in the settings.

use std::path::PathBuf;

fn path() -> PathBuf {
    crate::settings::config_dir().join("ui_layout.ron")
}

/// Restore the saved layout. The style and options set by the skin are
/// kept (only positions, sizes and per-window state come from the file).
///
/// Call it before `fonts::apply`: egui keeps the font definitions that wait
/// for the next pass in the memory this replaces, so fonts installed before
/// it are dropped and the interface starts with egui's own.
pub fn load(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read(path()) else {
        return;
    };
    if let Err(e) = std::str::from_utf8(&bytes).map_err(|e| e.to_string()).and_then(|s| restore(ctx, s)) {
        log::warn!("ui layout ignored: {e}");
    }
}

fn restore(ctx: &egui::Context, saved: &str) -> Result<(), String> {
    let mem = ron::from_str::<egui::Memory>(saved).map_err(|e| e.to_string())?;
    let options = ctx.options(|o| o.clone());
    ctx.memory_mut(|m| *m = mem);
    ctx.options_mut(|o| *o = options);
    Ok(())
}

pub fn save(ctx: &egui::Context) {
    let mem = ctx.memory(|m| m.clone());
    match ron::to_string(&mem) {
        Ok(text) => {
            let p = path();
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // write then rename: never leave a half-written file
            let tmp = p.with_extension("ron.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
        Err(e) => log::warn!("ui layout not saved: {e}"),
    }
}

/// Forget the saved layout ("Tout réinitialiser").
pub fn forget(ctx: &egui::Context) {
    let _ = std::fs::remove_file(path());
    let options = ctx.options(|o| o.clone());
    ctx.memory_mut(|m| *m = egui::Memory::default());
    ctx.options_mut(|o| *o = options);
}

#[cfg(test)]
mod tests {
    #[test]
    fn layout_round_trip_keeps_window_rects() {
        let ctx = egui::Context::default();
        let run = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new("t")
                    .id(egui::Id::new("floater_t"))
                    .default_pos([123.0, 45.0])
                    .show(ui.ctx(), |ui| {
                        ui.label("x");
                    });
                ui.ctx().data_mut(|d| d.insert_persisted(egui::Id::new("min"), true));
            });
            out.textures_delta.clear();
        };
        run(&ctx);
        let text = ron::to_string(&ctx.memory(|m| m.clone())).expect("serialize");
        let back: egui::Memory = ron::from_str(&text).expect("deserialize");
        let ctx2 = egui::Context::default();
        ctx2.memory_mut(|m| *m = back);
        assert_eq!(ctx2.data_mut(|d| d.get_persisted::<bool>(egui::Id::new("min"))), Some(true));
        let rect = ctx2.memory(|m| m.area_rect(egui::Id::new("floater_t")));
        assert!(rect.is_some_and(|r| (r.min.x - 123.0).abs() < 1.0), "{rect:?}");
    }

    /// The order of `App::new`: the interface font chosen in Preferences is
    /// installed after the saved layout. The other way round, a session with
    /// a saved layout started with egui's own fonts (and without the system
    /// fallback faces, #127), whatever the setting.
    #[test]
    fn fonts_installed_after_the_layout_are_kept() {
        let saved = ron::to_string(&egui::Context::default().memory(|m| m.clone())).expect("serialize");
        let first_font = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
            ctx.fonts(|f| f.definitions().families[&egui::FontFamily::Proportional][0].clone())
        };
        let ctx = egui::Context::default();
        super::restore(&ctx, &saved).expect("restore");
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::NOTO);
        assert_eq!(first_font(&ctx), "aurora-main");

        let ctx = egui::Context::default();
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::NOTO);
        super::restore(&ctx, &saved).expect("restore");
        assert_ne!(first_font(&ctx), "aurora-main", "egui keeps pending fonts now: the order is free");
    }
}
