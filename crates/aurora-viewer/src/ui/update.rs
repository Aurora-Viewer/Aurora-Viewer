//! Update prompt preview. User choices follow PromptOptionalUpdate and
//! LLVelopack's optional download flow (indra/newview/llvelopack.cpp,
//! originally LGPL 2.1). Real installation follows the approved preview.

use super::icons::Icons;
use super::login::{LoginAction, LoginForm};
use super::news::LoginInfo;
use super::widgets::{flat_button_sized, primary_button_sized};
use crate::settings::Settings;
use crate::theme::Palette;
use egui::{Frame, Margin, RichText, Sense, Vec2};
use std::time::Instant;

enum Phase {
    Available,
    Downloading(f32),
    Ready,
}

/// A synthetic login screen owns its settings: preview buttons cannot change
/// saved credentials, update preferences, or files in the viewer directory.
pub struct UpdateDemo {
    settings: Settings,
    form: LoginForm,
    info: LoginInfo,
    phase: Phase,
    started: Option<Instant>,
    open: bool,
    maintenance: aurora_updater::Maintenance,
    /// The installed and offered versions the cards show.
    versions: (&'static str, &'static str),
}

impl UpdateDemo {
    pub fn from_env(settings: &Settings) -> Option<Self> {
        std::env::var_os("AURORA_DEMO")?;
        let mode = std::env::var("AURORA_DEMO_UPDATE").ok()?;
        let mut settings = settings.clone();
        settings.username.clear();
        settings.remember_username = false;
        settings.remember_password = false;
        // `beta`: the offer a viewer on the Beta channel gets for a nightly
        let versions = if mode == "beta" {
            ("0.7.6", "0.7.7-beta.20261025.1")
        } else {
            ("0.2.0", "0.3.0")
        };
        let phase = match mode.as_str() {
            "progress" => Phase::Downloading(0.42),
            "ready" => Phase::Ready,
            _ => Phase::Available,
        };
        Some(Self {
            settings,
            form: LoginForm::default(),
            info: LoginInfo::preview_release(versions.1),
            phase,
            started: None,
            open: mode != "ignored",
            maintenance: demo_maintenance(),
            versions,
        })
    }

    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, icons: &Icons, logo: Option<&egui::TextureHandle>) -> LoginAction {
        self.form.block_reason = Some(aurora_i18n::tr!("update-login-test"));
        let action = super::login::show(ui, p, icons, logo, &mut self.settings, &mut self.form, &self.info, None);
        if matches!(action, LoginAction::CheckUpdates) {
            self.phase = Phase::Available;
            self.started = None;
            self.open = true;
        }
        if self.open {
            self.prompt(ui.ctx(), p, icons);
        }
        // Only quit is actionable on this synthetic, offline login screen.
        match action {
            LoginAction::Quit => LoginAction::Quit,
            _ => LoginAction::None,
        }
    }

    fn prompt(&mut self, ctx: &egui::Context, p: &Palette, icons: &Icons) {
        if let Some(started) = self.started {
            let fraction = (started.elapsed().as_secs_f32() / 6.0).min(1.0);
            self.phase = if fraction >= 1.0 {
                Phase::Ready
            } else {
                Phase::Downloading(fraction)
            };
            if fraction >= 1.0 {
                self.started = None;
            }
            ctx.request_repaint();
        }
        let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 620.0);
        aurora_render::blurred_backdrop(ctx, egui::Modal::default_area(egui::Id::unique("viewer_update")).layer());
        let response = super::widgets::Modal::new(egui::Id::unique("viewer_update"))
            .transparent_backdrop()
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = Vec2::new(10.0, 9.0);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
                    icon(ui, icons, "download-filled", rect.shrink(10.0), p.violet_light);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("update-demo-badge")).size(10.0).color(p.amber));
                        let title = match self.phase {
                            Phase::Available => aurora_i18n::tr!("update-title"),
                            Phase::Downloading(_) => aurora_i18n::tr!("update-downloading-title"),
                            Phase::Ready => aurora_i18n::tr!("update-ready-title"),
                        };
                        ui.label(RichText::new(title).size(23.0).strong().color(p.ink));
                    });
                });
                ui.label(RichText::new(aurora_i18n::tr!("update-intro")).size(13.0).color(p.muted));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let card_width = (width - 44.0) / 2.0;
                    version_card(ui, p, card_width, aurora_i18n::tr!("update-current"), self.versions.0, p.danger);
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    icon(ui, icons, "arrow-right", rect, p.muted);
                    version_card(ui, p, card_width, aurora_i18n::tr!("update-new"), self.versions.1, p.success);
                });
                ui.add_space(8.0);
                match self.phase {
                    Phase::Available => {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("update-release-notes"))
                                .size(13.0)
                                .strong()
                                .color(p.ink),
                        );
                        card(ui, p).show(ui, |ui| {
                            ui.set_width(width - 28.0);
                            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                                ui.label(RichText::new(aurora_i18n::tr!("update-demo-notes")).size(13.0).color(p.muted));
                            });
                        });
                    }
                    Phase::Downloading(fraction) => {
                        ui.add_space(9.0);
                        ui.label(RichText::new(aurora_i18n::tr!("update-downloading")).size(13.0).color(p.ink));
                        super::widgets::progress(ui, p, "update_download_demo", fraction, width, "{}", None);
                        ui.label(
                            RichText::new(aurora_i18n::tr!(
                                "update-transfer",
                                downloaded = aurora_i18n::number(350.0 * f64::from(fraction), 1),
                                total = "350"
                            ))
                            .size(12.0)
                            .color(p.muted),
                        );
                        ui.add_space(12.0);
                    }
                    Phase::Ready => {
                        ui.add_space(9.0);
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                            icon(ui, icons, "circle-check-filled", rect, p.success);
                            ui.label(RichText::new(aurora_i18n::tr!("update-ready")).size(14.0).color(p.ink));
                        });
                        super::widgets::progress(ui, p, "update_ready", 1.0, width, "", Some(p.success));
                        ui.add_space(20.0);
                    }
                }
                ui.add_space(8.0);
                maintenance_summary(ui, p, self.maintenance);
                ui.label(RichText::new(aurora_i18n::tr!("update-demo-safe")).size(11.0).color(p.muted_dim));
                ui.add_space(BEFORE_BUTTONS);
                match self.phase {
                    Phase::Available => {
                        ui.horizontal(|ui| {
                            let size = Vec2::new((width - 20.0) / 3.0, 34.0);
                            let skip = flat_button_sized(ui, p, aurora_i18n::tr!("update-skip"), size)
                                .on_hover_text(aurora_i18n::tr!("update-skip-hint"))
                                .clicked();
                            let later = flat_button_sized(ui, p, aurora_i18n::tr!("update-later"), size)
                                .on_hover_text(aurora_i18n::tr!("update-later-hint"))
                                .clicked();
                            if skip || later {
                                self.open = false;
                            }
                            if primary_button_sized(ui, p, aurora_i18n::tr!("update-install"), size).clicked() {
                                self.phase = Phase::Downloading(0.0);
                                self.started = Some(Instant::now());
                            }
                        });
                    }
                    Phase::Downloading(_) => {
                        if flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(width, 34.0)).clicked() {
                            self.phase = Phase::Available;
                            self.started = None;
                        }
                    }
                    Phase::Ready => {
                        if primary_button_sized(ui, p, aurora_i18n::tr!("update-demo-finish"), Vec2::new(width, 34.0)).clicked() {
                            self.open = false;
                            self.started = None;
                        }
                    }
                }
            });
        if response.should_close() {
            self.open = false;
            self.started = None;
        }
    }
}

/// Space above the row of buttons: no line, the buttons' faces are enough.
const BEFORE_BUTTONS: f32 = 18.0;

/// Inset card of the update window (versions, release notes), with the
/// corners of the theme.
fn card(ui: &egui::Ui, p: &Palette) -> Frame {
    Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(Margin::same(14))
}

fn version_card(ui: &mut egui::Ui, p: &Palette, width: f32, label: &str, version: &str, color: egui::Color32) {
    let (number, build) = version_parts(version);
    card(ui, p).show(ui, |ui| {
        ui.set_width(width - 28.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.label(RichText::new(label).size(11.0).color(p.muted));
            ui.label(RichText::new(number).size(28.0).strong().color(color));
            if let Some(build) = build {
                ui.label(RichText::new(build).size(11.0).color(p.muted));
            }
        });
    });
}

/// A nightly beta reads as its number and "beta", its build day under it:
/// `0.7.7-beta.20261025.1` is "0.7.7 beta", built on 2026-10-25, #1.
fn version_parts(version: &str) -> (String, Option<String>) {
    let beta = aurora_updater::feed::version(version).filter(|v| aurora_updater::feed::is_beta(v) && v.build.is_empty());
    let Some(v) = beta else { return (version.to_owned(), None) };
    let number = format!("{}.{}.{} {}", v.major, v.minor, v.patch, aurora_i18n::tr!("update-beta-label"));
    let mut ids = v.pre.as_str().split('.').skip(1);
    let build = match (ids.next(), ids.next()) {
        (Some(day), Some(n)) if day.len() == 8 && day.bytes().all(|b| b.is_ascii_digit()) => Some(aurora_i18n::tr!(
            "update-beta-build",
            date = format!("{}-{}-{}", &day[..4], &day[4..6], &day[6..]),
            number = n.to_owned()
        )),
        _ => None,
    };
    (number, build)
}

/// The approved preview's layout, now fed by the live update controller.
pub fn show_real(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    updater: &mut crate::updater::Updater,
    settings: &mut Settings,
    runtime: &tokio::runtime::Handle,
) -> bool {
    use crate::updater::State;
    if !updater.open {
        return false;
    }
    let mut quit = false;
    let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 620.0);
    let required = updater.required();
    aurora_render::blurred_backdrop(ctx, egui::Modal::default_area(egui::Id::unique("viewer_update")).layer());
    let response = super::widgets::Modal::new(egui::Id::unique("viewer_update"))
        .transparent_backdrop()
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = Vec2::new(10.0, 9.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
                icon(ui, icons, "download-filled", rect.shrink(10.0), p.violet_light);
                let title = match updater.state {
                    State::Checking => aurora_i18n::tr!("ui-login-checking"),
                    State::Downloading(_) | State::Handoff => aurora_i18n::tr!("update-downloading-title"),
                    State::Ready => aurora_i18n::tr!("update-ready-title"),
                    State::Error(_) | State::Rollback => aurora_i18n::tr!("update-error-title"),
                    State::Current => aurora_i18n::tr!("ui-login-up-to-date"),
                    _ if required => aurora_i18n::tr!("update-required-title"),
                    _ => aurora_i18n::tr!("update-title"),
                };
                ui.label(RichText::new(title).size(23.0).strong().color(p.ink));
            });
            if let Some(r) = &updater.release {
                ui.label(
                    RichText::new(if required {
                        aurora_i18n::tr!("update-required")
                    } else {
                        aurora_i18n::tr!("update-intro")
                    })
                    .size(13.0)
                    .color(p.muted),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let card = (width - 44.0) / 2.0;
                    version_card(ui, p, card, aurora_i18n::tr!("update-current"), aurora_version::version(), p.danger);
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    icon(ui, icons, "arrow-right", rect, p.muted);
                    version_card(ui, p, card, aurora_i18n::tr!("update-new"), &r.version, p.success);
                });
            }
            ui.add_space(8.0);
            match updater.state {
                State::Offer => {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("update-release-notes"))
                            .size(13.0)
                            .strong()
                            .color(p.ink),
                    );
                    card(ui, p).show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                            let notes = updater
                                .release
                                .as_ref()
                                .map(|r| r.notes.as_str())
                                .filter(|s| !s.is_empty())
                                .unwrap_or(aurora_i18n::tr!("update-no-notes"));
                            ui.label(RichText::new(notes).size(13.0).color(p.muted));
                        });
                    });
                }
                State::Downloading(progress) => {
                    ui.label(if progress.preparing {
                        aurora_i18n::tr!("update-preparing")
                    } else {
                        aurora_i18n::tr!("update-downloading")
                    });
                    let fraction = progress.received as f32 / progress.total.max(1) as f32;
                    super::widgets::progress(ui, p, "update_download", fraction, width, "{}", None);
                    ui.label(
                        RichText::new(aurora_i18n::tr!(
                            "update-transfer",
                            downloaded = aurora_i18n::number(progress.received as f64 / 1_048_576.0, 1),
                            total = aurora_i18n::number(progress.total as f64 / 1_048_576.0, 1)
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                }
                State::Ready => {
                    ui.label(aurora_i18n::tr!("update-ready"));
                }
                State::Handoff => {
                    ui.spinner();
                    ui.label(aurora_i18n::tr!("update-restarting"));
                }
                State::Checking => {
                    ui.spinner();
                }
                State::Error(kind) => {
                    ui.label(RichText::new(failure_text(kind)).color(p.danger));
                }
                State::Rollback => {
                    ui.label(RichText::new(aurora_i18n::tr!("update-rollback")).color(p.danger));
                }
                _ => {}
            }
            ui.add_space(8.0);
            if let Some(release) = &updater.release {
                maintenance_summary(ui, p, release.maintenance);
            }
            ui.add_space(BEFORE_BUTTONS);
            match updater.state {
                State::Offer => {
                    ui.horizontal(|ui| {
                        let size = Vec2::new((width - 20.0) / if required { 2.0 } else { 3.0 }, 34.0);
                        if required {
                            if flat_button_sized(ui, p, aurora_i18n::tr!("common-quit"), size).clicked() {
                                quit = true;
                            }
                        } else {
                            if flat_button_sized(ui, p, aurora_i18n::tr!("update-skip"), size)
                                .on_hover_text(aurora_i18n::tr!("update-skip-hint"))
                                .clicked()
                            {
                                if let Some(r) = &updater.release {
                                    settings.ignored_update = r.tag.clone();
                                    settings.save();
                                }
                                updater.open = false;
                            }
                            if flat_button_sized(ui, p, aurora_i18n::tr!("update-later"), size).clicked() {
                                updater.open = false;
                            }
                        }
                        if primary_button_sized(ui, p, aurora_i18n::tr!("update-install"), size).clicked() {
                            updater.download(runtime);
                        }
                    });
                }
                State::Downloading(_) => {
                    if flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(width, 34.0)).clicked() {
                        updater.cancel();
                    }
                }
                State::Ready => {
                    if primary_button_sized(ui, p, aurora_i18n::tr!("update-restart"), Vec2::new(width, 34.0)).clicked() {
                        updater.restart();
                    }
                }
                State::Error(_) | State::Rollback => {
                    let url = updater
                        .release
                        .as_ref()
                        .map_or(aurora_updater::feed::RELEASE_PAGE, |r| r.url.as_str());
                    if flat_button_sized(ui, p, aurora_i18n::tr!("update-open-release"), Vec2::new(width, 34.0)).clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                    if flat_button_sized(
                        ui,
                        p,
                        if required {
                            aurora_i18n::tr!("common-quit")
                        } else {
                            aurora_i18n::tr!("common-close")
                        },
                        Vec2::new(width, 34.0),
                    )
                    .clicked()
                    {
                        if required {
                            quit = true;
                        } else {
                            updater.open = false;
                        }
                    }
                }
                State::Checking | State::Current | State::Idle => {
                    if flat_button_sized(ui, p, aurora_i18n::tr!("common-close"), Vec2::new(width, 34.0)).clicked() {
                        updater.open = false;
                    }
                }
                State::Handoff => {}
            }
        });
    if response.should_close() && !required && !updater.busy() {
        updater.open = false;
    }
    quit
}

fn demo_maintenance() -> aurora_updater::Maintenance {
    let choice = std::env::var("AURORA_DEMO_UPDATE_MAINTENANCE").unwrap_or_default();
    aurora_updater::Maintenance {
        clear_cache: matches!(choice.as_str(), "cache" | "both"),
        reset_settings: matches!(choice.as_str(), "settings" | "both"),
    }
}

fn maintenance_summary(ui: &mut egui::Ui, p: &Palette, effects: aurora_updater::Maintenance) {
    ui.label(
        RichText::new(aurora_i18n::tr!("update-effects-title"))
            .size(12.0)
            .strong()
            .color(p.ink),
    );
    ui.label(
        RichText::new(if effects.clear_cache {
            aurora_i18n::tr!("update-cache-clear")
        } else {
            aurora_i18n::tr!("update-cache-keep")
        })
        .size(12.0)
        .color(if effects.clear_cache { p.amber } else { p.muted }),
    );
    ui.label(
        RichText::new(if effects.reset_settings {
            aurora_i18n::tr!("update-settings-reset")
        } else {
            aurora_i18n::tr!("update-settings-migrate")
        })
        .size(12.0)
        .color(if effects.reset_settings { p.amber } else { p.muted }),
    );
    ui.label(
        RichText::new(aurora_i18n::tr!("update-effects-restart"))
            .size(11.0)
            .color(p.muted_dim),
    );
}

fn failure_text(kind: aurora_updater::Failure) -> &'static str {
    use aurora_updater::Failure;
    match kind {
        Failure::Network => aurora_i18n::tr!("update-error-network"),
        Failure::Integrity => aurora_i18n::tr!("update-error-integrity"),
        Failure::Filesystem | Failure::Busy => aurora_i18n::tr!("update-error-filesystem"),
        Failure::Package => aurora_i18n::tr!("update-error-package"),
        // macOS: an application to replace, not a folder to extract
        Failure::Unsupported if cfg!(target_os = "macos") => aurora_i18n::tr!("update-error-unsupported-macos"),
        Failure::Unsupported => aurora_i18n::tr!("update-error-unsupported"),
        Failure::Startup => aurora_i18n::tr!("update-error-startup"),
    }
}

fn icon(ui: &egui::Ui, icons: &Icons, name: &str, rect: egui::Rect, color: egui::Color32) {
    if let Some(texture) = icons.get(name) {
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(ctx: &egui::Context, demo: &mut UpdateDemo, width: f32, events: Vec<egui::Event>) -> egui::FullOutput {
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                demo.show(ui, &p, &icons, None);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn label_position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos + egui::vec2(3.0, 5.0)),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| label_position(shape, label)),
            _ => None,
        }
    }

    fn click(ctx: &egui::Context, demo: &mut UpdateDemo, width: f32, label: &str) {
        let output = draw(ctx, demo, width, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| label_position(&shape.shape, label))
            .expect(label);
        for pressed in [true, false] {
            draw(
                ctx,
                demo,
                width,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    #[test]
    fn a_beta_version_reads_as_its_number_with_its_build_day_under_it() {
        let (number, build) = version_parts("0.7.7-beta.20261025.12");
        assert!(number.starts_with("0.7.7 ") && !number.contains("2026"), "{number}");
        let build = build.expect("build line");
        assert!(build.contains("2026-10-25") && build.contains("12"), "{build}");
        for plain in ["0.7.6", "0.7.7-rc.1", "0.7.6+3.gabcdef123", "0.7.7-beta.1+2.gabc"] {
            assert_eq!(version_parts(plain), (plain.to_owned(), None), "{plain}");
        }
    }

    #[test]
    fn required_offer_has_no_skip_or_later_and_escape_cannot_close_it() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let mut updater = crate::updater::Updater::new(runtime.handle(), aurora_updater::feed::Channel::Stable);
        updater.release = Some(aurora_updater::feed::Release {
            tag: "v0.3.0".into(),
            version: "0.3.0".into(),
            notes: "Synthetic notes".into(),
            url: aurora_updater::feed::RELEASE_PAGE.into(),
            archive: None,
            checksum: None,
            required: true,
            maintenance: Default::default(),
        });
        updater.state = crate::updater::State::Offer;
        updater.open = true;
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let mut settings = Settings::default();
        // Modal areas size themselves on the first frame before painting.
        let mut warmup = ctx.run_ui(egui::RawInput::default(), |ui| {
            show_real(ui.ctx(), &p, &icons, &mut updater, &mut settings, runtime.handle());
        });
        warmup.textures_delta.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                ..Default::default()
            },
            |ui| {
                show_real(ui.ctx(), &p, &icons, &mut updater, &mut settings, runtime.handle());
            },
        );
        output.textures_delta.clear();
        for label in [aurora_i18n::tr!("update-skip"), aurora_i18n::tr!("update-later")] {
            assert!(
                !output.shapes.iter().any(|s| label_position(&s.shape, label).is_some()),
                "forbidden choice: {label}"
            );
        }
        for label in [aurora_i18n::tr!("common-quit"), aurora_i18n::tr!("update-install")] {
            assert!(
                output.shapes.iter().any(|s| label_position(&s.shape, label).is_some()),
                "expected choice: {label}"
            );
        }
        let mut escape = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ui| {
                show_real(ui.ctx(), &p, &icons, &mut updater, &mut settings, runtime.handle());
            },
        );
        escape.textures_delta.clear();
        assert!(updater.open && updater.blocks_login());
    }

    #[test]
    fn manual_check_reopens_skipped_release_on_wide_and_narrow_login() {
        for width in [1600.0, 860.0] {
            let ctx = egui::Context::default();
            let mut demo = UpdateDemo {
                settings: Settings::default(),
                form: LoginForm::default(),
                info: LoginInfo::preview_release("0.3.0"),
                versions: ("0.2.0", "0.3.0"),
                phase: Phase::Available,
                started: None,
                open: true,
                maintenance: Default::default(),
            };
            draw(&ctx, &mut demo, width, vec![]);
            click(&ctx, &mut demo, width, aurora_i18n::tr!("update-skip"));
            assert!(!demo.open, "the skipped proposal must close");
            click(&ctx, &mut demo, width, aurora_i18n::tr!("update-check"));
            let output = draw(&ctx, &mut demo, width, vec![]);
            assert!(demo.open, "the manual check must reopen the skipped release");
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| label_position(&shape.shape, aurora_i18n::tr!("update-title")).is_some()),
                "the proposal must be visible again"
            );
        }
    }
}
