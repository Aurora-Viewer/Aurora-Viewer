//! Preferences floater: categories on the left, grouped settings on the
//! right (label column + control column), flat Aurora style.

mod camera;
mod commands;
mod practical;

use super::fonts::{self, SystemFont};
use super::icons::Icons;
use super::widgets::Floater;
use crate::settings::Settings;
use crate::theme::Palette;
use egui::{Color32, CornerRadius, RichText, Sense, Vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    Binding(crate::keybinds::Action, usize),
    HudDrag,
}

#[derive(Default)]
pub struct OptionsUi {
    pub tab: usize,
    system_fonts: Option<Vec<SystemFont>>,
    font_filter: String,
    /// Disk cache usage (bytes) and whether a clear is running; set by the app.
    pub cache_usage: u64,
    pub cache_clearing: bool,
    /// Detected video memory (MB) and integrated GPU; set by the app.
    pub vram: (Option<u64>, bool),
    /// Set by the "describe nearby objects" button, cleared by the app.
    pub describe_nearby: bool,
    /// Audio devices (set by the app) and microphone level (0..1).
    pub output_devices: Vec<String>,
    pub input_devices: Vec<String>,
    pub mic_level: f32,
    /// Media plugins folder found (empty: none) and plugins running.
    pub media_plugins: String,
    pub media_running: usize,
    /// Shortcut or gesture waiting for its next input, captured by the app.
    pub capture: Option<Capture>,
    clear: Option<(crate::keybinds::Action, usize)>,
    /// "Tout réinitialiser" confirmation open.
    confirm_reset: bool,
    /// Sound assets being typed (Firestorm name -> text).
    ui_sound_text: std::collections::HashMap<&'static str, String>,
}

#[derive(Default)]
pub struct OptionsResult {
    pub changed: bool,
    pub font_changed: bool,
    pub clear_cache: bool,
    /// Devices, volumes or voice settings changed: apply to the audio engine.
    pub audio_changed: bool,
    /// List the audio devices again.
    pub refresh_devices: bool,
    /// Every setting went back to its default.
    pub reset: bool,
}

/// Category tabs (index = `OptionsUi::tab`).
fn categories() -> [(&'static str, &'static str); 13] {
    [
        (aurora_i18n::tr!("ui-options-graphics"), "Command_Environments_Icon"),
        (aurora_i18n::tr!("common-interface"), "Command_Appearance_Icon"),
        (aurora_i18n::tr!("ui-options-fonts"), "Command_Chat_Icon"),
        (aurora_i18n::tr!("common-chat"), "nearbychat_18"),
        (aurora_i18n::tr!("ui-options-controls"), "Command_Move_Icon"),
        (aurora_i18n::tr!("ui-options-colors"), "drop"),
        (aurora_i18n::tr!("ui-options-network-and-cache"), "Command_Inventory_Icon"),
        (aurora_i18n::tr!("ui-options-sound-and-voice"), "speaker-high"),
        (aurora_i18n::tr!("ui-options-shortcuts"), "keyboard"),
        (aurora_i18n::tr!("common-debug"), "code"),
        (aurora_i18n::tr!("common-camera"), "camera"),
        (aurora_i18n::tr!("ui-options-commands"), "terminal-window"),
        (aurora_i18n::tr!("ui-options-practical"), "hand-grabbing"),
    ]
}

/// Order of the tabs in the sidebar.
const ORDER: [usize; 13] = [0, 1, 2, 3, TAB_COMMANDS, 7, 4, TAB_CAMERA, TAB_PRACTICAL, 8, 5, 6, 9];

pub const TAB_AUDIO: usize = 7;
pub const TAB_CHAT: usize = 3;
pub const TAB_CAMERA: usize = 10;
pub const TAB_COMMANDS: usize = 11;
pub const TAB_PRACTICAL: usize = 12;

const LABEL_W: f32 = 180.0;

fn category(ui: &mut egui::Ui, p: &Palette, icons: &Icons, label: &str, icon: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    let fill = if selected {
        p.violet.gamma_multiply(0.28)
    } else if resp.hovered() {
        p.raised
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(2), fill);
    if selected {
        ui.painter()
            .rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())), 0.0, p.violet);
    }
    if let Some(t) = icons.get(icon) {
        let ir = egui::Rect::from_center_size(rect.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0));
        ui.painter().image(
            t.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if selected { p.ink } else { p.muted },
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(34.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if selected { p.ink } else { p.muted },
    );
    resp.clicked()
}

/// A titled group of settings drawn as a flat card.
/// Phosphor icon of a settings group (by its title).
fn group_icon(title: &str) -> Option<&'static str> {
    Some(match title {
        value if value == aurora_i18n::text("ui-options-quality") => "sliders",
        value if value == aurora_i18n::text("common-light") => "sun",
        value if value == aurora_i18n::text("ui-options-reflections") => "drop",
        value if value == aurora_i18n::text("common-image") => "image",
        value if value == aurora_i18n::text("ui-options-smoothness") => "speedometer",
        value if value == aurora_i18n::text("ui-options-layout") => "layout",
        value if value == aurora_i18n::text("preferences-group-skin") => "palette",
        value if value == aurora_i18n::text("ui-options-size-and-preview") => "text-aa",
        value if value == aurora_i18n::text("ui-options-bundled-fonts") => "text-t",
        value if value == aurora_i18n::text("ui-options-system-fonts") => "monitor",
        value if value == aurora_i18n::text("common-nearby-chat") => "chat-circle-dots",
        value if value == aurora_i18n::text("ui-options-messages") => "chat-text",
        value if value == aurora_i18n::text("ui-options-emitter") => "user-circle",
        value if value == aurora_i18n::text("ui-options-chat-mentions") => "at",
        value if value == aurora_i18n::text("ui-options-mouse") => "mouse",
        value if value == aurora_i18n::text("preferences-group-huds") => "hand-grabbing",
        value if value == aurora_i18n::text("ui-options-keyboard") => "keyboard",
        value if value == aurora_i18n::text("ui-options-name-tag-color") => "tag",
        value if value == aurora_i18n::text("ui-options-mini-map-markers") => "map-trifold",
        value if value == aurora_i18n::text("ui-options-disk-cache") => "hard-drives",
        value if value == aurora_i18n::text("ui-options-devices") => "speaker-hifi",
        value if value == aurora_i18n::text("ui-options-volumes") => "speaker-high",
        value if value == aurora_i18n::text("common-voice") => "waveform",
        value if value == aurora_i18n::text("ui-options-push-to-talk") => "microphone",
        value if value == aurora_i18n::text("common-movement") => "person-simple-walk",
        value if value == aurora_i18n::text("common-camera") || value == aurora_i18n::text("common-viewpoint") => "camera",
        value if value == aurora_i18n::text("common-camera-movement") => "sliders",
        value if value == aurora_i18n::text("common-behavior") => "arrows-clockwise",
        value if value == aurora_i18n::text("common-limits") => "prohibit",
        value if value == aurora_i18n::text("common-mouselook") => "crosshair",
        value if value == aurora_i18n::text("ui-options-look-at") => "eye",
        value if value == aurora_i18n::text("common-communication") => "chats-circle",
        value if value == aurora_i18n::text("ui-options-groups-and-blocking") => "prohibit",
        value if value == aurora_i18n::text("ui-options-auto-replies") => "paper-plane-tilt",
        value if value == aurora_i18n::text("ui-options-automatic-rejections") => "hand-waving",
        value if value == aurora_i18n::text("common-command-line") || value == aurora_i18n::text("common-other-commands") => {
            "terminal-window"
        }
        value if value == aurora_i18n::text("common-teleport-commands") => "map-pin",
        value if value == aurora_i18n::text("common-camera-and-display-commands") => "camera",
        value if value == aurora_i18n::text("common-windows") => "squares-four",
        _ => return None,
    })
}

fn group(ui: &mut egui::Ui, p: &Palette, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if let Some(t) = group_icon(title).and_then(super::icons::global) {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
            ui.painter().image(
                t.id(),
                r,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                p.violet_light,
            );
        }
        ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(p.violet_light));
    });
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            body(ui);
        });
    ui.add_space(10.0);
}

/// One row: label (with optional hint) on the left, control on the right.
fn row(ui: &mut egui::Ui, p: &Palette, label: &str, hint: &str, control: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(LABEL_W, 22.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            p.ink,
        );
        if !hint.is_empty() {
            resp.on_hover_text(hint);
        }
        changed = control(ui);
    });
    changed
}

const SLIDER_W: f32 = 230.0;

fn slider<T: egui::emath::Numeric>(ui: &mut egui::Ui, v: &mut T, range: std::ops::RangeInclusive<T>, suffix: &str) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    ui.add(
        egui::Slider::new(v, range)
            .suffix(suffix)
            .custom_parser(aurora_i18n::parse_number)
            .trailing_fill(true),
    )
    .changed()
}

/// Stepped slider with a named level shown next to it.
fn stepped(ui: &mut egui::Ui, p: &Palette, v: &mut u8, labels: &[&str]) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    let max = labels.len().saturating_sub(1) as u8;
    let r = ui.add(
        egui::Slider::new(v, 0..=max)
            .custom_parser(aurora_i18n::parse_number)
            .step_by(1.0)
            .trailing_fill(true)
            .show_value(false),
    );
    let label = labels.get((*v).min(max) as usize).copied().unwrap_or("");
    ui.label(RichText::new(label).size(12.0).color(p.violet_pale));
    r.changed()
}

/// Color swatch with a picker popup (opaque).
fn swatch(ui: &mut egui::Ui, v: &mut crate::settings::Rgba) -> bool {
    let mut col = Color32::from_rgba_unmultiplied(v[0], v[1], v[2], v[3]);
    ui.spacing_mut().interact_size = Vec2::new(38.0, 18.0);
    let changed = egui::widgets::color_picker::color_edit_button_srgba(ui, &mut col, egui::widgets::color_picker::Alpha::Opaque).changed();
    if changed {
        *v = col.to_array();
    }
    changed
}

/// Grid of labelled swatches, three per row (Firestorm color preferences).
fn swatches(ui: &mut egui::Ui, id: &str, items: &mut [(&str, &mut crate::settings::Rgba)]) -> bool {
    let mut changed = false;
    egui::Grid::new(id).num_columns(3).spacing([24.0, 8.0]).show(ui, |ui| {
        for (i, (label, v)) in items.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.set_min_width(150.0);
                changed |= swatch(ui, v);
                ui.label(RichText::new(*label).size(12.5));
            });
            if i % 3 == 2 {
                ui.end_row();
            }
        }
    });
    changed
}

fn format_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        aurora_i18n::tr!("memory-gb", value = aurora_i18n::number(mb / 1024.0, 1))
    } else {
        aurora_i18n::tr!("memory-mb", value = aurora_i18n::number(mb, 0))
    }
}

/// Debug overlays (not saved: off again at the next launch).
fn debug_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    let mut c = false;
    let d = &mut s.debug;
    ui.label(
        RichText::new(aurora_i18n::tr!(
            "ui-options-diagnostic-tools-drawn-over-the-view-disabled-next-time"
        ))
        .size(12.0)
        .color(p.muted),
    );
    ui.add_space(6.0);
    group(ui, p, aurora_i18n::tr!("ui-options-avatars"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-complexity-in-name-tags"),
            aurora_i18n::tr!("ui-options-below-the-name-from-green-to-red-based-on-2"),
            |ui| toggle(ui, p, &mut d.complexity_tags),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-skeletons"),
            aurora_i18n::tr!("ui-options-nearby-avatar-bones-32-m-after-animation"),
            |ui| toggle(ui, p, &mut d.skeletons),
        );
    });
    group(ui, p, aurora_i18n::tr!("common-rendering"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("common-show-transparency"),
            aurora_i18n::tr!("ui-options-ctrl-alt-t-transparent-masked-and-invisible-faces-in"),
            |ui| toggle(ui, p, &mut d.show_alpha),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-rigged-objects-too"),
            aurora_i18n::tr!("ui-options-includes-rigged-bodies-clothes-and-attachments"),
            |ui| toggle(ui, p, &mut d.show_alpha_rigged),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-wireframe"),
            aurora_i18n::tr!("ui-options-triangle-edges-over-the-scene"),
            |ui| toggle(ui, p, &mut d.wireframe),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-show-glow"),
            aurora_i18n::tr!("ui-options-glow-amount-instead-of-the-image-red-glow-10"),
            |ui| toggle(ui, p, &mut d.glow_view),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-objects-with-glow"),
            aurora_i18n::tr!("ui-options-outlines-objects-with-glowing-faces-in-orange"),
            |ui| toggle(ui, p, &mut d.glow_objects),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-lights"),
            aurora_i18n::tr!("ui-options-light-positions-and-radii-in-their-colors"),
            |ui| toggle(ui, p, &mut d.lights),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("common-reflection-probes"),
            aurora_i18n::tr!("ui-options-probe-influence-turquoise-for-automatic-purple-for-manual-gray"),
            |ui| toggle(ui, p, &mut d.probes),
        );
    });
    group(ui, p, aurora_i18n::tr!("ui-options-objects-and-culling"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-bounding-boxes"),
            aurora_i18n::tr!("ui-options-bounding-box-of-each-object-within-96-m"),
            |ui| toggle(ui, p, &mut d.bounds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("common-culling"),
            aurora_i18n::tr!("ui-options-each-object-s-tested-sphere-green-drawn-red-outside"),
            |ui| toggle(ui, p, &mut d.culling),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-freeze-culling"),
            aurora_i18n::tr!("ui-options-keep-the-current-culling-view-move-the-camera-to"),
            |ui| toggle(ui, p, &mut d.freeze_culling),
        );
    });
    c
}

/// A label and a multi-line response text under it.
fn response_text(ui: &mut egui::Ui, p: &Palette, label: &str, text: &mut String) -> bool {
    ui.label(RichText::new(label).size(12.5).color(p.ink));
    ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY))
        .changed()
}

fn toggle(ui: &mut egui::Ui, p: &Palette, v: &mut bool) -> bool {
    super::widgets::switch(ui, p, v).changed()
}

pub fn show(ctx: &egui::Context, p: &Palette, icons: &Icons, s: &mut Settings, st: &mut OptionsUi, open: &mut bool) -> OptionsResult {
    let mut r = OptionsResult::default();
    let screen = ctx.content_rect();
    Floater::new(
        "options",
        aurora_i18n::tr!("common-preferences"),
        egui::pos2(screen.center().x - 330.0, 80.0),
        Vec2::new(660.0, 470.0),
    )
    .help(aurora_i18n::tr!("ui-options-changes-apply-immediately-and-are-saved-on-closing"))
    .show(ctx, p, open, |ui| {
        let h = (ui.available_height() - 4.0).max(200.0);
        ui.horizontal_top(|ui| {
            // ---- categories
            ui.allocate_ui_with_layout(Vec2::new(150.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_height(h);
                ui.spacing_mut().item_spacing.y = 2.0;
                for i in ORDER {
                    let (label, icon) = categories()[i];
                    if category(ui, p, icons, label, icon, st.tab == i) {
                        st.capture = None;
                        st.tab = i;
                    }
                }
                // reset everything, at the bottom of the column
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(4.0);
                    let b = ui.add(
                        egui::Button::new(
                            RichText::new(aurora_i18n::tr!("ui-options-reset-everything-2"))
                                .size(12.0)
                                .color(p.muted),
                        )
                        .frame(false),
                    );
                    if b.on_hover_text(aurora_i18n::tr!("ui-options-reset-all-settings")).clicked() {
                        st.confirm_reset = true;
                    }
                });
            });
            ui.add_space(6.0);
            // ---- content
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(categories()[st.tab.min(categories().len() - 1)].0)
                        .size(17.0)
                        .color(p.ink),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(h - 34.0)
                    .min_scrolled_height(h - 34.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width() - 8.0);
                        r.changed |= match st.tab {
                            TAB_AUDIO => audio_page(ui, p, s, st, &mut r.audio_changed, &mut r.refresh_devices),
                            8 => keys_page(ui, p, s, st),
                            9 => debug_page(ui, p, s),
                            TAB_CAMERA => camera::page(ui, p, s),
                            TAB_COMMANDS => commands::page(ui, p, &mut s.chat_commands),
                            TAB_PRACTICAL => practical::page(ui, p, s, st),
                            _ => content(ui, p, s, st, &mut r.font_changed, &mut r.clear_cache),
                        };
                    });
            });
        });
    });
    if st.confirm_reset && *open {
        let modal = egui::Modal::new(egui::Id::new("options_reset")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-options-reset-everything"))
                    .size(15.0)
                    .strong()
                    .color(p.ink),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "ui-options-all-settings-graphics-interface-fonts-chat-sound-and-voice"
                ))
                .size(12.5)
                .color(p.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let yes = ui.add(
                    egui::Button::new(RichText::new(aurora_i18n::tr!("common-reset")).color(Color32::WHITE))
                        .fill(p.danger)
                        .corner_radius(CornerRadius::same(2)),
                );
                if yes.clicked() {
                    s.reset_keeping_account();
                    st.capture = None;
                    r.changed = true;
                    r.font_changed = true;
                    r.audio_changed = true;
                    r.reset = true;
                    st.confirm_reset = false;
                }
                if super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    st.confirm_reset = false;
                }
            });
        });
        if modal.should_close() {
            st.confirm_reset = false;
        }
    }
    r
}

fn content(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi, font_changed: &mut bool, clear_cache: &mut bool) -> bool {
    let mut c = false;
    match st.tab {
        0 => {
            group(ui, p, aurora_i18n::tr!("ui-options-quality"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("common-draw-distance"),
                    aurora_i18n::tr!("ui-options-objects-beyond-this-distance-are-not-drawn-96-m"),
                    |ui| slider(ui, &mut s.draw_distance, 32.0..=512.0, " m"),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-level-of-detail-lod"),
                    aurora_i18n::tr!("ui-options-lod-factor-for-sl-objects-and-meshes-2-by"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.lod_factor, 0.5..=4.0)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(0.125)
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-fully-rendered-avatars"),
                    aurora_i18n::tr!("ui-options-nearest-avatars-are-fully-rendered-others-use-impostors-periodically"),
                    |ui| slider(ui, &mut s.max_avatars, 1..=100, ""),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-maximum-avatar-complexity"),
                    aurora_i18n::tr!("ui-options-above-this-limit-the-avatar-appears-as-a-gray"),
                    |ui| {
                        use crate::scene::complexity::slider;
                        ui.spacing_mut().slider_width = SLIDER_W;
                        let mut pos = slider::from_limit(s.max_complexity);
                        let text = |v: f64| {
                            let limit = slider::to_limit(v.round() as u32);
                            if limit == 0 {
                                aurora_i18n::tr!("ui-options-unlimited").to_owned()
                            } else {
                                format!("{} k", limit / 1000)
                            }
                        };
                        let r = ui.add(
                            egui::Slider::new(&mut pos, 1..=slider::OFF)
                                .custom_parser(aurora_i18n::parse_number)
                                .custom_formatter(move |v, _| text(v))
                                .trailing_fill(true),
                        );
                        if r.changed() {
                            s.max_complexity = slider::to_limit(pos);
                        }
                        r.changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-maximum-worn-surface-area"),
                    aurora_i18n::tr!("ui-options-above-this-limit-the-avatar-also-appears-as-a"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.max_attachment_area, 0.0..=10_000.0)
                                .custom_parser(aurora_i18n::parse_number)
                                .logarithmic(true)
                                .step_by(10.0)
                                .custom_formatter(|v, _| {
                                    if v < 1.0 {
                                        aurora_i18n::tr!("ui-options-unlimited").to_owned()
                                    } else {
                                        format!("{v:.0} m²")
                                    }
                                })
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-loading-avatars"),
                    aurora_i18n::tr!("ui-options-particle-cloud-until-the-avatar-and-its-attachments-load"),
                    |ui| {
                        stepped(
                            ui,
                            p,
                            &mut s.loading_avatars,
                            &[
                                aurora_i18n::tr!("ui-options-cloud-bar"),
                                aurora_i18n::tr!("ui-options-bar-only"),
                                aurora_i18n::tr!("common-nothing"),
                            ],
                        )
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-limit-applied"),
                    aurora_i18n::tr!("ui-options-always-fully-render-friends-or-fully-render-only-friends"),
                    |ui| {
                        stepped(
                            ui,
                            p,
                            &mut s.complexity_mode,
                            &[
                                aurora_i18n::tr!("ui-options-to-everyone"),
                                aurora_i18n::tr!("ui-options-except-friends"),
                                aurora_i18n::tr!("ui-options-friends-only"),
                            ],
                        )
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-occlusion"),
                    aurora_i18n::tr!("ui-options-objects-hidden-behind-others-walls-floors-are-not-drawn"),
                    |ui| toggle(ui, p, &mut s.occlusion_culling),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("common-particles"),
                    aurora_i18n::tr!("ui-options-maximum-particles-0-disabled"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        let r = ui.add(
                            egui::Slider::new(&mut s.max_particles, 0..=8192)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(256.0)
                                .trailing_fill(true),
                        );
                        r.changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-auto-texture-memory"),
                    aurora_i18n::tr!("ui-options-based-on-detected-video-memory"),
                    |ui| toggle(ui, p, &mut s.texture_budget_auto),
                );
                if s.texture_budget_auto {
                    let (vram, integrated) = st.vram;
                    let budget = crate::settings::auto_texture_budget(vram, integrated);
                    let gpu = match vram {
                        Some(v) if !integrated => aurora_i18n::tr!("ui-options-mb-of-video-memory-detected", value0 = format!("{}", v)),
                        _ if integrated => aurora_i18n::tr!("ui-options-integrated-graphics").to_owned(),
                        _ => aurora_i18n::tr!("ui-options-video-memory-not-detected").to_owned(),
                    };
                    row(ui, p, aurora_i18n::tr!("ui-options-texture-memory"), &gpu, |ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("status-budget-mb", budget = format!("{}", budget))).color(p.ink));
                        false
                    });
                } else {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-texture-memory"),
                        aurora_i18n::tr!("ui-options-textures-beyond-this-distance-use-a-lower-resolution"),
                        |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W;
                            ui.add(
                                egui::Slider::new(&mut s.texture_budget_mb, 256..=32768)
                                    .custom_parser(aurora_i18n::parse_number)
                                    .logarithmic(true)
                                    .suffix(aurora_i18n::tr!("memory-mb-suffix"))
                                    .trailing_fill(true),
                            )
                            .changed()
                        },
                    );
                }
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-anisotropic-filtering"),
                    aurora_i18n::tr!("ui-options-sharp-textures-viewed-at-an-angle"),
                    |ui| {
                        let label = |v: u16| {
                            if v <= 1 {
                                aurora_i18n::tr!("common-disabled").to_owned()
                            } else {
                                format!("{v}×")
                            }
                        };
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("aniso")
                            .width(SLIDER_W)
                            .selected_text(label(s.anisotropy))
                            .show_ui(ui, |ui| {
                                for v in [1u16, 2, 4, 8, 16] {
                                    changed |= ui.selectable_value(&mut s.anisotropy, v, label(v)).changed();
                                }
                            });
                        changed
                    },
                );
            });
            group(ui, p, aurora_i18n::tr!("common-light"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-shadow-quality"),
                    aurora_i18n::tr!("ui-options-cascaded-shadow-resolution-and-distance"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        let r = ui.add(
                            egui::Slider::new(&mut s.shadow_quality, 0..=4)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(1.0)
                                .trailing_fill(true)
                                .show_value(false),
                        );
                        let label = [
                            aurora_i18n::tr!("ui-options-disabled-2"),
                            aurora_i18n::tr!("ui-options-low"),
                            aurora_i18n::tr!("common-medium"),
                            aurora_i18n::tr!("ui-options-high"),
                            aurora_i18n::tr!("ui-options-ultra"),
                        ][s.shadow_quality.min(4) as usize];
                        ui.label(RichText::new(label).size(12.0).color(p.violet_pale));
                        s.shadows = s.shadow_quality > 0;
                        r.changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-ambient-occlusion"),
                    aurora_i18n::tr!("ui-options-darkens-corners-and-contact-areas-ssao"),
                    |ui| {
                        stepped(
                            ui,
                            p,
                            &mut s.ssao,
                            &[
                                aurora_i18n::tr!("ui-options-disabled"),
                                aurora_i18n::tr!("ui-options-low"),
                                aurora_i18n::tr!("ui-options-high"),
                            ],
                        )
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("common-time-of-day"),
                    aurora_i18n::tr!("ui-options-shared-environment-region-parcel-eep-day-cycle"),
                    |ui| {
                        let names = super::bars::time_of_day();
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("tod")
                            .width(SLIDER_W)
                            .selected_text(names[s.time_of_day.min(4) as usize])
                            .show_ui(ui, |ui| {
                                for (i, n) in names.iter().enumerate() {
                                    changed |= ui.selectable_value(&mut s.time_of_day, i as u8, *n).changed();
                                }
                            });
                        changed
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-environment-fade"),
                    aurora_i18n::tr!("ui-options-fade-duration-when-you-change-the-environment-yourself-selector"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.env_manual_transition, 0.0..=60.0)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(0.5)
                                .suffix(" s")
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-keep-environment"),
                    aurora_i18n::tr!("ui-options-restore-the-local-environment-selected-sky-water-and-day"),
                    |ui| toggle(ui, p, &mut s.env_persist),
                );
                c |= row(ui, p, aurora_i18n::tr!("ui-options-exposure"), "", |ui| {
                    ui.spacing_mut().slider_width = SLIDER_W;
                    ui.add(
                        egui::Slider::new(&mut s.exposure, 0.25..=4.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .logarithmic(true)
                            .trailing_fill(true),
                    )
                    .changed()
                });
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-sharpness"),
                    aurora_i18n::tr!("ui-options-enhances-fine-details-amd-cas-0-4-as-in"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.sharpen, 0.0..=1.0)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(0.05)
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-glow"),
                    aurora_i18n::tr!("ui-options-glow-around-glowing-faces-as-in-firestorm"),
                    |ui| toggle(ui, p, &mut s.glow),
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-reflections"), |ui| {
                let levels = [
                    aurora_i18n::tr!("ui-options-disabled-3"),
                    aurora_i18n::tr!("ui-options-low-14"),
                    aurora_i18n::tr!("ui-options-medium-12"),
                    aurora_i18n::tr!("ui-options-high-34"),
                    aurora_i18n::tr!("ui-options-ultra-full"),
                ];
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-water-reflections"),
                    aurora_i18n::tr!("ui-options-planar-scene-reflection-high-and-ultra-are-costly"),
                    |ui| stepped(ui, p, &mut s.water_reflections, &levels),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-mirrors"),
                    aurora_i18n::tr!("ui-options-sl-mirror-probes-render-resolution-low-by-default"),
                    |ui| stepped(ui, p, &mut s.mirrors, &levels),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-screen-space-reflections-ssr"),
                    aurora_i18n::tr!("ui-options-shiny-surfaces-expensive-disabled-by-default"),
                    |ui| toggle(ui, p, &mut s.ssr),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("common-reflection-probes"),
                    aurora_i18n::tr!("ui-options-what-shiny-surfaces-reflect-as-in-firestorm"),
                    |ui| {
                        stepped(
                            ui,
                            p,
                            &mut s.reflection_probes,
                            &[
                                aurora_i18n::tr!("ui-options-sky-only"),
                                aurora_i18n::tr!("ui-options-manual"),
                                aurora_i18n::tr!("ui-options-manual-terrain"),
                                aurora_i18n::tr!("ui-options-entire-scene"),
                            ],
                        )
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-probe-count"),
                    aurora_i18n::tr!("ui-options-active-probes-around-you-video-memory"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.probe_count, 8..=64)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(8.0)
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
            });
            group(ui, p, aurora_i18n::tr!("common-image"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-anti-aliasing"),
                    aurora_i18n::tr!("ui-options-msaa-sharp-but-costly-fxaa-lightweight-smaa-lightweight-and"),
                    |ui| {
                        let names = [
                            aurora_i18n::tr!("common-none"),
                            "FXAA",
                            "MSAA 2×",
                            "MSAA 4×",
                            "MSAA 8×",
                            "TAA",
                            "SMAA",
                        ];
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("aa")
                            .width(SLIDER_W)
                            .selected_text(names[(s.antialiasing as usize).min(names.len() - 1)])
                            .show_ui(ui, |ui| {
                                for (i, n) in names.iter().enumerate() {
                                    changed |= ui.selectable_value(&mut s.antialiasing, i as u8, *n).changed();
                                }
                            });
                        changed
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-tone-mapping"),
                    aurora_i18n::tr!("ui-options-khronos-pbr-neutral-is-used-by-second-life"),
                    |ui| {
                        let names = [aurora_i18n::tr!("ui-options-khronos-pbr-neutral"), "ACES"];
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("tonemap")
                            .width(SLIDER_W)
                            .selected_text(names[s.tonemapper.min(1) as usize])
                            .show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut s.tonemapper, 0, names[0]).changed();
                                changed |= ui.selectable_value(&mut s.tonemapper, 1, names[1]).changed();
                            });
                        changed
                    },
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-smoothness"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-vertical-synchronization"),
                    aurora_i18n::tr!("ui-options-synchronizes-frames-with-the-monitor-refresh-rate-no-tearing"),
                    |ui| toggle(ui, p, &mut s.vsync),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("extra-limit-frame-rate"),
                    aurora_i18n::tr!("ui-options-caps-frames-per-second-less-heat-and-noise-enabled"),
                    |ui| {
                        let mut ch = toggle(ui, p, &mut s.fps_cap);
                        ui.add_enabled_ui(s.fps_cap, |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W - 50.0;
                            ch |= ui
                                .add(
                                    egui::Slider::new(&mut s.fps_limit, 10..=360)
                                        .custom_parser(aurora_i18n::parse_number)
                                        .suffix(aurora_i18n::tr!("status-fps"))
                                        .trailing_fill(true),
                                )
                                .changed();
                        });
                        ch
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-limit-when-unfocused"),
                    aurora_i18n::tr!("ui-options-when-the-window-is-not-in-the-foreground-the"),
                    |ui| {
                        let mut ch = toggle(ui, p, &mut s.background_fps_cap);
                        ui.add_enabled_ui(s.background_fps_cap, |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W - 50.0;
                            ch |= ui
                                .add(
                                    egui::Slider::new(&mut s.background_fps_limit, 1..=120)
                                        .custom_parser(aurora_i18n::parse_number)
                                        .suffix(aurora_i18n::tr!("status-fps"))
                                        .trailing_fill(true),
                                )
                                .changed();
                        });
                        ch
                    },
                );
            });
        }
        1 => {
            group(ui, p, aurora_i18n::tr!("language-choice"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("language-choice"),
                    aurora_i18n::tr!("language-help"),
                    |ui| {
                        let selected = if s.language == "default" {
                            aurora_i18n::tr!("language-system")
                        } else {
                            aurora_i18n::available_languages()
                                .find(|&(code, _)| code == s.language)
                                .map_or_else(|| aurora_i18n::text_in("en", "language-name"), |(_, name)| name)
                        };
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("ui-language")
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(&mut s.language, "default".to_owned(), aurora_i18n::tr!("language-system"))
                                    .changed();
                                for (code, name) in aurora_i18n::available_languages() {
                                    changed |= ui.selectable_value(&mut s.language, code.to_owned(), name).changed();
                                }
                            });
                        changed
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("language-share"),
                    aurora_i18n::tr!("language-share-help"),
                    |ui| toggle(ui, p, &mut s.language_is_public),
                );
            });
            group(ui, p, aurora_i18n::tr!("update-settings"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("update-settings"),
                    aurora_i18n::tr!("update-settings-help"),
                    |ui| {
                        let mut changed = false;
                        let label = match s.update_mode {
                            crate::updater::Mode::Disabled => aurora_i18n::tr!("update-mode-disabled"),
                            crate::updater::Mode::Ask => aurora_i18n::tr!("update-mode-ask"),
                            crate::updater::Mode::Automatic => aurora_i18n::tr!("update-mode-auto"),
                        };
                        egui::ComboBox::from_id_salt("update-mode").selected_text(label).show_ui(ui, |ui| {
                            for (value, label) in [
                                (crate::updater::Mode::Disabled, aurora_i18n::tr!("update-mode-disabled")),
                                (crate::updater::Mode::Ask, aurora_i18n::tr!("update-mode-ask")),
                                (crate::updater::Mode::Automatic, aurora_i18n::tr!("update-mode-auto")),
                            ] {
                                changed |= ui.selectable_value(&mut s.update_mode, value, label).changed();
                            }
                        });
                        changed
                    },
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-layout"), |ui| {
                c |= row(ui, p, aurora_i18n::tr!("bug-toolbar"), aurora_i18n::tr!("bug-toolbar-help"), |ui| {
                    toggle(ui, p, &mut s.show_bug_report)
                });
                c |= row(ui, p, aurora_i18n::tr!("ui-options-interface-scale"), "", |ui| {
                    slider(ui, &mut s.ui_scale, 0.75..=2.0, "×")
                });
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-loading-backdrop"),
                    aurora_i18n::tr!("ui-options-loading-and-teleport-screens-show-your-last-view-blurred"),
                    |ui| toggle(ui, p, &mut s.loading_backdrop),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-name-distance"),
                    aurora_i18n::tr!("ui-options-0-hide-avatar-names"),
                    |ui| slider(ui, &mut s.name_tag_distance, 0.0..=128.0, " m"),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-complexity-in-name-tags"),
                    aurora_i18n::tr!("ui-options-below-the-name-from-green-to-red-based-on"),
                    |ui| toggle(ui, p, &mut s.tag_complexity),
                );
                if s.tag_complexity {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-silhouettes-only"),
                        aurora_i18n::tr!("ui-options-only-avatars-above-the-limit-shown-as-silhouettes"),
                        |ui| toggle(ui, p, &mut s.tag_complexity_too_complex_only),
                    );
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-mine-too"),
                        aurora_i18n::tr!("ui-options-your-complexity-below-your-name"),
                        |ui| toggle(ui, p, &mut s.tag_complexity_own),
                    );
                }
                c |= row(ui, p, aurora_i18n::tr!("ui-options-performance-at-startup"), "", |ui| {
                    toggle(ui, p, &mut s.show_perf)
                });
                c |= row(ui, p, aurora_i18n::tr!("ui-options-mini-map-at-startup"), "", |ui| {
                    toggle(ui, p, &mut s.show_minimap)
                });
            });
            // panel_preferences_UI.xml « Utiliser des fenêtres distinctes pour : »
            group(ui, p, aurora_i18n::tr!("common-windows"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-landmarks-and-place-profiles"),
                    aurora_i18n::tr!("ui-options-separate-windows-for-landmarks-teleport-history-details-place-profiles"),
                    |ui| toggle(ui, p, &mut s.standalone_place_details),
                );
            });
            group(ui, p, aurora_i18n::tr!("common-inventory"), |ui| {
                c |= super::inventory::preferences(ui, p, &mut s.inventory);
            });
            group(ui, p, aurora_i18n::tr!("ui-options-names"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-display-names"),
                    aurora_i18n::tr!("ui-options-names-chosen-by-residents-otherwise-account-names"),
                    |ui| toggle(ui, p, &mut s.use_display_names),
                );
                if s.use_display_names {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-with-username"),
                        aurora_i18n::tr!("ui-options-below-the-name-in-name-tags-beside-it-in"),
                        |ui| toggle(ui, p, &mut s.show_usernames),
                    );
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-in-first-last-format"),
                        aurora_i18n::tr!("ui-options-jane-jane-doe-instead-of-jane-jane-doe"),
                        |ui| toggle(ui, p, &mut s.legacy_username_format),
                    );
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-announce-name-changes"),
                        aurora_i18n::tr!("ui-options-in-chat-when-a-resident-changes-their-display-name"),
                        |ui| toggle(ui, p, &mut s.display_name_notices),
                    );
                }
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-hide-resident"),
                    aurora_i18n::tr!("ui-options-last-name-for-recent-accounts"),
                    |ui| toggle(ui, p, &mut s.trim_resident),
                );
            });
            group(ui, p, aurora_i18n::tr!("extra-skin"), |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-options-customize-colors-icons-and-the-toolbar-in"))
                        .size(12.0)
                        .color(p.muted),
                );
                let dir = crate::settings::config_dir().join("skins").join("aurora");
                ui.label(RichText::new(dir.display().to_string()).size(12.0).monospace().color(p.teal));
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-options-theme-json-layout-json-icons-reloaded-automatically"))
                        .size(11.0)
                        .color(p.muted_dim),
                );
            });
        }
        2 => {
            group(ui, p, aurora_i18n::tr!("ui-options-bundled-fonts"), |ui| {
                for builtin in [fonts::NOTO, fonts::INTER, fonts::ROBOTO] {
                    let sel = s.font == builtin || (s.font.is_empty() && builtin == fonts::NOTO);
                    let label = if builtin == fonts::NOTO {
                        aurora_i18n::tr!("ui-options-default", builtin = builtin.to_string())
                    } else {
                        builtin.to_owned()
                    };
                    if ui.radio(sel, RichText::new(label).size(13.0)).clicked() && !sel {
                        s.font = builtin.to_owned();
                        *font_changed = true;
                        c = true;
                    }
                }
            });
            group(ui, p, aurora_i18n::tr!("ui-options-system-fonts"), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut st.font_filter)
                        .hint_text(aurora_i18n::tr!("ui-options-search-fonts"))
                        .desired_width(ui.available_width()),
                );
                let list = st.system_fonts.get_or_insert_with(fonts::system_fonts);
                let filter = st.font_filter.to_lowercase();
                egui::ScrollArea::vertical().id_salt("sysfonts").max_height(150.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for f in list.iter().filter(|f| filter.is_empty() || f.name.to_lowercase().contains(&filter)) {
                        let key = format!("system:{}", f.path.display());
                        let sel = s.font == key;
                        if ui.radio(sel, RichText::new(&f.name).size(13.0)).clicked() && !sel {
                            s.font = key;
                            *font_changed = true;
                            c = true;
                        }
                    }
                });
            });
            group(ui, p, aurora_i18n::tr!("ui-options-size-and-preview"), |ui| {
                c |= row(ui, p, aurora_i18n::tr!("ui-options-text-size"), "", |ui| {
                    slider(ui, &mut s.font_scale, 0.8..=1.4, "×")
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-options-the-violet-wolf-howls-under-the-northern-lights"))
                        .size(15.0)
                        .color(p.ink),
                );
                ui.label(RichText::new(aurora_i18n::tr!("font-sample")).size(12.0).color(p.muted));
            });
        }
        3 => {
            group(ui, p, aurora_i18n::tr!("common-nearby-chat"), |ui| {
                c |= row(ui, p, aurora_i18n::tr!("ui-options-show-time"), "", |ui| {
                    toggle(ui, p, &mut s.chat_timestamps)
                });
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-chat-bubble-duration"),
                    aurora_i18n::tr!("ui-options-0-no-chat-bubbles-above-the-chat-bar"),
                    |ui| slider(ui, &mut s.chat_toast_seconds, 0.0..=60.0, " s"),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-warn-before-external-links"),
                    aurora_i18n::tr!("ui-options-warn-before-opening-a-link-to-a-site-that"),
                    |ui| toggle(ui, p, &mut s.warn_external_links),
                );
                c |= row(ui, p, aurora_i18n::tr!("ui-options-conversations-at-startup"), "", |ui| {
                    toggle(ui, p, &mut s.show_chat)
                });
            });
            group(ui, p, aurora_i18n::tr!("ui-options-groups-and-blocking"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-no-group-chats"),
                    aurora_i18n::tr!("extra-reject-all-group-chats-fsmuteallgroups"),
                    |ui| toggle(ui, p, &mut s.mute_all_groups),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-groups-without-notices"),
                    aurora_i18n::tr!("ui-options-reject-chat-from-groups-whose-notices-you-do-not"),
                    |ui| toggle(ui, p, &mut s.mute_groups_without_notices),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-report-in-chat"),
                    aurora_i18n::tr!("ui-options-announce-additions-to-and-removals-from-the-block-list"),
                    |ui| toggle(ui, p, &mut s.report_blocks),
                );
            });
            // Firestorm: Préférences > Confidentialité > Réponses auto. 1 et 2
            let a = &mut s.autoresponse;
            group(ui, p, aurora_i18n::tr!("ui-options-auto-replies"), |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-options-enable-these-modes-in-communicate-online-status"))
                        .size(11.5)
                        .color(p.muted),
                );
                c |= response_text(ui, p, aurora_i18n::tr!("ui-options-in-do-not-disturb-mode"), &mut a.dnd_text);
                c |= response_text(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-to-everyone-in-auto-reply-mode"),
                    &mut a.autorespond_text,
                );
                c |= response_text(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-in-auto-respond-to-non-friends-mode"),
                    &mut a.nonfriends_text,
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-reply-when-away"),
                    aurora_i18n::tr!("ui-options-send-the-text-below-when-i-am-away"),
                    |ui| toggle(ui, p, &mut a.send_away),
                );
                c |= response_text(ui, p, aurora_i18n::tr!("ui-options-when-i-am-away"), &mut a.away_text);
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-reply-to-blocked-residents"),
                    aurora_i18n::tr!("ui-options-send-the-text-below-to-blocked-people-their-im"),
                    |ui| toggle(ui, p, &mut a.send_muted),
                );
                c |= response_text(ui, p, aurora_i18n::tr!("ui-options-to-blocked-people"), &mut a.muted_text);
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-away-after"),
                    aurora_i18n::tr!("ui-options-become-away-after-this-time-without-keyboard-or-mouse"),
                    |ui| slider(ui, &mut a.away_after_minutes, 0..=60, " min"),
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-automatic-rejections"), |ui| {
                c |= response_text(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-in-reject-teleports-mode"),
                    &mut a.reject_teleports_text,
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-except-my-friends"),
                    aurora_i18n::tr!("ui-options-do-not-reject-or-auto-respond-to-my-friends"),
                    |ui| toggle(ui, p, &mut a.dont_reject_friends_teleports),
                );
                c |= response_text(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-in-reject-friendship-requests-mode"),
                    &mut a.reject_friendship_text,
                );
            });
        }
        4 => {
            group(ui, p, aurora_i18n::tr!("ui-options-mouse"), |ui| {
                c |= row(ui, p, aurora_i18n::tr!("ui-options-sensitivity"), "", |ui| {
                    slider(ui, &mut s.mouse_sensitivity, 0.1..=5.0, "×")
                });
                c |= row(ui, p, aurora_i18n::tr!("ui-options-invert-vertical-axis"), "", |ui| {
                    toggle(ui, p, &mut s.invert_mouse)
                });
            });
            group(ui, p, aurora_i18n::tr!("ui-options-keyboard"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-arrows-strafe"),
                    aurora_i18n::tr!("ui-options-left-right-strafe-instead-of-turning-shift-reverses-this"),
                    |ui| toggle(ui, p, &mut s.arrows_strafe),
                );
                row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-shortcuts"),
                    aurora_i18n::tr!("ui-options-keys-and-mouse-buttons-for-each-action"),
                    |ui| {
                        if super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-edit-shortcuts")).clicked() {
                            st.tab = 8;
                        }
                        false
                    },
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-look-at"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-look-at-tracking"),
                    aurora_i18n::tr!("ui-options-your-avatar-s-head-and-eyes-follow-the-cursor"),
                    |ui| toggle(ui, p, &mut s.look_at),
                );
                if s.look_at {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-look-at-range"),
                        aurora_i18n::tr!("ui-options-more-distant-targets-are-limited-to-this-distance-from"),
                        |ui| {
                            let unlimited = s.look_at_range >= crate::settings::LOOK_AT_UNLIMITED;

                            slider(
                                ui,
                                &mut s.look_at_range,
                                1.0..=crate::settings::LOOK_AT_UNLIMITED,
                                if unlimited {
                                    aurora_i18n::tr!("ui-options-m-unlimited")
                                } else {
                                    " m"
                                },
                            )
                        },
                    );
                }
            });
        }
        5 => {
            let k = &mut s.colors;
            group(ui, p, aurora_i18n::tr!("ui-options-name-tag-color"), |ui| {
                c |= swatches(
                    ui,
                    "tags",
                    &mut [
                        (aurora_i18n::tr!("ui-options-my-name"), &mut k.tag_me),
                        (aurora_i18n::tr!("common-friends"), &mut k.tag_friend),
                        (aurora_i18n::tr!("ui-options-ignored"), &mut k.tag_muted),
                        ("Linden Lab", &mut k.tag_linden),
                        (aurora_i18n::tr!("ui-options-matches"), &mut k.tag_match),
                        (aurora_i18n::tr!("ui-options-non-matches"), &mut k.tag_mismatch),
                    ],
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-mini-map-markers"), |ui| {
                c |= swatches(
                    ui,
                    "map",
                    &mut [
                        (aurora_i18n::tr!("common-me"), &mut k.map_me),
                        (aurora_i18n::tr!("common-others"), &mut k.map_other),
                        (aurora_i18n::tr!("common-friends"), &mut k.map_friend),
                        ("Linden Lab", &mut k.map_linden),
                        (aurora_i18n::tr!("ui-options-ignored"), &mut k.map_muted),
                    ],
                );
                ui.add_space(4.0);
                c |= ui
                    .checkbox(
                        &mut k.map_ranges,
                        RichText::new(aurora_i18n::tr!("ui-options-show-whisper-normal-chat-and-shout-ranges-on-the"))
                            .size(12.5)
                            .color(p.ink),
                    )
                    .changed();
                ui.add_enabled_ui(k.map_ranges, |ui| {
                    egui::Grid::new("ranges").num_columns(3).spacing([24.0, 6.0]).show(ui, |ui| {
                        for (label, on, col) in [
                            (aurora_i18n::tr!("ui-options-whisper"), &mut k.map_whisper_on, &mut k.map_whisper),
                            (aurora_i18n::tr!("ui-options-normal-voice"), &mut k.map_say_on, &mut k.map_say),
                            (aurora_i18n::tr!("ui-options-shout"), &mut k.map_shout_on, &mut k.map_shout),
                        ] {
                            ui.horizontal(|ui| {
                                c |= ui.checkbox(on, "").changed();
                                c |= swatch(ui, col);
                                ui.label(RichText::new(label).size(12.5).color(p.ink));
                            });
                        }
                        ui.end_row();
                    });
                });
            });
            group(ui, p, aurora_i18n::tr!("ui-options-messages"), |ui| {
                c |= swatches(
                    ui,
                    "msgs",
                    &mut [
                        (aurora_i18n::tr!("ui-options-my-messages"), &mut k.chat_mine),
                        (aurora_i18n::tr!("common-others"), &mut k.chat_others),
                        (aurora_i18n::tr!("common-objects"), &mut k.chat_objects),
                        (aurora_i18n::tr!("common-friends"), &mut k.chat_friends),
                        ("Linden Lab", &mut k.chat_linden),
                        (aurora_i18n::tr!("ui-options-ignored"), &mut k.chat_muted),
                        (aurora_i18n::tr!("common-system"), &mut k.chat_system),
                        (aurora_i18n::tr!("ui-options-errors"), &mut k.chat_errors),
                        (aurora_i18n::tr!("ui-options-object-ims"), &mut k.chat_object_im),
                        (aurora_i18n::tr!("common-owner"), &mut k.chat_owner),
                        (aurora_i18n::tr!("status-urls"), &mut k.chat_urls),
                        (aurora_i18n::tr!("ui-options-url-path"), &mut k.chat_slurl),
                        (aurora_i18n::tr!("ui-options-direct"), &mut k.chat_direct),
                    ],
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-emitter"), |ui| {
                c |= swatches(
                    ui,
                    "sender",
                    &mut [
                        (aurora_i18n::tr!("ui-options-avatars"), &mut k.sender_avatar),
                        (aurora_i18n::tr!("common-objects"), &mut k.sender_object),
                    ],
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-chat-mentions"), |ui| {
                c |= swatches(
                    ui,
                    "mentions",
                    &mut [
                        (aurora_i18n::tr!("ui-options-font"), &mut k.mention_text),
                        (aurora_i18n::tr!("ui-options-residents"), &mut k.mention_residents),
                        (aurora_i18n::tr!("common-me"), &mut k.mention_me),
                    ],
                );
            });
            ui.add_space(6.0);
            if ui
                .button(RichText::new(aurora_i18n::tr!("ui-options-default-colors")).size(12.5))
                .clicked()
            {
                *k = crate::settings::ColorSettings::default();
                c = true;
            }
        }
        _ => {
            group(ui, p, aurora_i18n::tr!("ui-options-disk-cache"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-maximum-size"),
                    aurora_i18n::tr!("ui-options-textures-meshes-animations-shared-across-regions-objects-per-region"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.cache_size_mb, 512..=65536)
                                .custom_parser(aurora_i18n::parse_number)
                                .logarithmic(true)
                                .suffix(aurora_i18n::tr!("memory-mb-suffix"))
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-usage"),
                    aurora_i18n::tr!("ui-options-least-recently-used-files-are-removed-when-the-limit"),
                    |ui| {
                        let used = st.cache_usage;
                        let limit = s.cache_size_mb as u64 * 1024 * 1024;
                        let frac = (used as f32 / limit.max(1) as f32).clamp(0.0, 1.0);
                        ui.add(egui::ProgressBar::new(frac).desired_width(SLIDER_W).fill(p.violet).text(format!(
                            "{} / {}",
                            format_bytes(used),
                            format_bytes(limit)
                        )));
                        false
                    },
                );
                row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-clear-cache"),
                    aurora_i18n::tr!("ui-options-everything-will-be-downloaded-again-as-needed"),
                    |ui| {
                        let label = if st.cache_clearing {
                            aurora_i18n::tr!("ui-options-deleting")
                        } else {
                            aurora_i18n::tr!("ui-options-clear-cache")
                        };
                        let b = ui.add_enabled(!st.cache_clearing, egui::Button::new(RichText::new(label).color(p.ink)));
                        if b.clicked() {
                            *clear_cache = true;
                        }
                        false
                    },
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-logs"), |ui| {
                row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-session-log"),
                    aurora_i18n::tr!("ui-options-aurora-log-previous-session-aurora-previous-log"),
                    |ui| {
                        if ui
                            .button(RichText::new(aurora_i18n::tr!("ui-options-open-folder")).color(p.ink))
                            .clicked()
                        {
                            crate::logging::open_logs_dir();
                        }
                        false
                    },
                );
                row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-nearby-objects"),
                    aurora_i18n::tr!("ui-options-write-faces-lights-and-materials-within-12-m-to"),
                    |ui| {
                        if ui
                            .button(RichText::new(aurora_i18n::tr!("ui-options-describe")).color(p.ink))
                            .clicked()
                        {
                            st.describe_nearby = true;
                        }
                        false
                    },
                );
            });
        }
    }
    c
}

/// Device combo: "" = system default.
fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut String, devices: &[String]) -> bool {
    let mut changed = false;
    let shown = if value.is_empty() {
        aurora_i18n::tr!("ui-options-system-default").to_owned()
    } else {
        value.clone()
    };
    egui::ComboBox::from_id_salt(id)
        .width(SLIDER_W)
        .selected_text(shown)
        .truncate()
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(value, String::new(), aurora_i18n::tr!("ui-options-system-default"))
                .changed();
            for d in devices {
                changed |= ui.selectable_value(value, d.clone(), d).changed();
            }
            if !value.is_empty() && !devices.contains(value) {
                ui.label(
                    RichText::new(aurora_i18n::tr!("status-value-missing", value = value.to_string()))
                        .size(12.0)
                        .weak(),
                );
            }
        });
    changed
}

/// "Son et voix": devices, the volumes of the top-right panel (same
/// settings, so both stay in sync), voice and microphone behaviour.
fn audio_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi, audio_changed: &mut bool, refresh: &mut bool) -> bool {
    let mut c = false;
    let a = &mut s.audio;
    group(ui, p, aurora_i18n::tr!("ui-options-devices"), |ui| {
        let out = row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-audio-output"),
            aurora_i18n::tr!("ui-options-speakers-or-headset-used-by-the-viewer"),
            |ui| device_combo(ui, "audio_out", &mut a.output_device, &st.output_devices),
        );
        let inp = row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-microphone"),
            aurora_i18n::tr!("ui-options-input-used-for-voice"),
            |ui| device_combo(ui, "audio_in", &mut a.input_device, &st.input_devices),
        );
        c |= out | inp;
        *audio_changed |= out | inp;
        row(ui, p, "", "", |ui| {
            if super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-refresh-list")).clicked() {
                *refresh = true;
            }
            false
        });
    });
    group(ui, p, aurora_i18n::tr!("ui-options-volumes"), |ui| {
        for (i, name) in crate::settings::audio_channels().iter().enumerate() {
            let changed = row(ui, p, name, "", |ui| {
                let mut ch = false;
                ui.spacing_mut().slider_width = SLIDER_W - 40.0;
                ch |= ui
                    .add_enabled(
                        !a.muted[i] && a.enabled[i],
                        egui::Slider::new(&mut a.volume[i], 0.0..=1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .trailing_fill(true)
                            .show_value(false),
                    )
                    .changed();
                ui.label(RichText::new(format!("{:>3.0} %", a.volume[i] * 100.0)).size(12.0).color(p.muted));
                let mut on = !a.muted[i];
                if super::widgets::switch(ui, p, &mut on)
                    .on_hover_text(if on {
                        aurora_i18n::tr!("common-cut")
                    } else {
                        aurora_i18n::tr!("ui-options-restore")
                    })
                    .changed()
                {
                    a.muted[i] = !on;
                    ch = true;
                }
                if i > 0 {
                    ch |= ui
                        .checkbox(&mut a.enabled[i], "")
                        .on_hover_text(aurora_i18n::tr!("ui-options-enable-this-channel"))
                        .changed();
                }
                ch
            });
            c |= changed;
            *audio_changed |= changed;
        }
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-auto-music"),
            aurora_i18n::tr!("ui-options-play-parcel-music-when-entering"),
            |ui| toggle(ui, p, &mut a.music_autoplay),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-collision-sounds"),
            aurora_i18n::tr!("ui-options-collisions-between-objects-and-avatars-as-in-firestorm-enabled"),
            |ui| toggle(ui, p, &mut a.collision_sounds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-gesture-sounds"),
            aurora_i18n::tr!("ui-options-sounds-played-by-other-avatars-gestures"),
            |ui| toggle(ui, p, &mut a.gesture_sounds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-interface-sounds"),
            aurora_i18n::tr!("ui-options-all-interface-sounds-clicks-windows-conversations-l-teleportation-menus"),
            |ui| toggle(ui, p, &mut a.ui_sounds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-auto-media"),
            aurora_i18n::tr!("ui-options-auto-play-parcel-media-after-standing-still-for-5"),
            |ui| toggle(ui, p, &mut a.media_autoplay),
        );
    });
    c |= ui_sounds_group(ui, p, a, st);
    group(ui, p, aurora_i18n::tr!("common-voice"), |ui| {
        let v = row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-enable-voice"),
            aurora_i18n::tr!("ui-options-connect-to-regional-voice-chat-webrtc"),
            |ui| toggle(ui, p, &mut a.voice_enabled),
        );
        c |= v;
        *audio_changed |= v;
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-microphone-button"),
            aurora_i18n::tr!("ui-options-toggle-click-once-to-enable-again-to-disable-hold"),
            |ui| {
                let mut ch = false;
                ch |= ui
                    .selectable_value(&mut a.mic_hold, false, aurora_i18n::tr!("ui-options-toggle"))
                    .changed();
                ch |= ui
                    .selectable_value(&mut a.mic_hold, true, aurora_i18n::tr!("common-hold-to-talk"))
                    .changed();
                ch
            },
        );
        let g = row(ui, p, aurora_i18n::tr!("ui-options-microphone-gain"), "", |ui| {
            slider(ui, &mut a.mic_gain, 0.0..=2.0, "×")
        });
        c |= g;
        *audio_changed |= g;
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-microphone-level"),
            aurora_i18n::tr!("ui-options-speak-to-test"),
            |ui| {
                let lvl = st.mic_level.clamp(0.0, 1.0);
                let col = if lvl > 0.85 {
                    p.danger
                } else if lvl > 0.6 {
                    p.amber
                } else {
                    p.teal
                };
                ui.add(egui::ProgressBar::new(lvl).desired_width(SLIDER_W).desired_height(8.0).fill(col));
                false
            },
        );
    });
    group(ui, p, aurora_i18n::tr!("ui-options-push-to-talk"), |ui| {
        let kb = s.keybinds.clone();
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-push-to-talk-key"),
            aurora_i18n::tr!("ui-options-key-or-mouse-button-middle-click-by-default-also"),
            |ui| {
                for slot in 0..2 {
                    binding_cell(ui, p, &kb, st, crate::keybinds::Action::PushToTalk, slot);
                }
                false
            },
        );
    });
    let m = &mut s.media;
    group(ui, p, aurora_i18n::tr!("ui-options-media-web-and-video"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-enable-media"),
            aurora_i18n::tr!("ui-options-parcel-and-prim-web-pages-and-videos-audiostreamingmedia"),
            |ui| toggle(ui, p, &mut m.enabled),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-prim-media"),
            aurora_i18n::tr!("ui-options-shared-media-displayed-on-object-faces-primmediamasterenabled"),
            |ui| toggle(ui, p, &mut m.prim_media),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-on-other-avatars"),
            aurora_i18n::tr!("ui-options-media-on-other-people-s-attachments-mediashowonothers-disabled-by"),
            |ui| toggle(ui, p, &mut m.show_on_others),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-first-click-interaction"),
            aurora_i18n::tr!("ui-options-the-first-click-goes-directly-to-the-page-when"),
            |ui| toggle(ui, p, &mut m.first_click_interact),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-hud-media"),
            aurora_i18n::tr!("ui-options-auto-play-hud-media-mediaautoplayhuds"),
            |ui| toggle(ui, p, &mut m.autoplay_huds),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-scripted-media"),
            aurora_i18n::tr!("ui-options-allow-scripts-to-start-parcel-media-permallowscriptedmedia-disabled-by"),
            |ui| toggle(ui, p, &mut m.allow_scripted),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-simultaneous-players"),
            aurora_i18n::tr!("ui-options-maximum-pages-videos-loaded-at-once-plugininstancestotal"),
            |ui| {
                ui.spacing_mut().slider_width = SLIDER_W - 40.0;
                let mut n = m.max_instances as u32;
                let ch = ui
                    .add(
                        egui::Slider::new(&mut n, 1..=16)
                            .custom_parser(aurora_i18n::parse_number)
                            .trailing_fill(true),
                    )
                    .changed();
                m.max_instances = n as usize;
                ch
            },
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-plugins-folder"),
            aurora_i18n::tr!("ui-options-folder-containing-slplugin-and-llplugin-blank-the-viewer-then"),
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut m.plugin_dir)
                        .desired_width(SLIDER_W)
                        .hint_text("automatique"),
                )
                .changed()
            },
        );
        let status = if st.media_plugins.is_empty() {
            aurora_i18n::tr!("ui-options-not-found-install-firestorm-or-copy-its-llplugin-folder").to_string()
        } else {
            aurora_i18n::tr!(
                "status-active-player-s",
                value0 = st.media_plugins.to_string(),
                value1 = format!("{}", st.media_running)
            )
        };
        row(ui, p, aurora_i18n::tr!("ui-options-plugins"), "", |ui| {
            ui.label(RichText::new(status).size(12.0).color(p.muted));
            false
        });
    });
    if let Some((a, slot)) = st.clear.take() {
        s.keybinds.set(a, slot, None);
        c = true;
    }
    c
}

/// One binding button: click to capture, right click to clear.
fn binding_cell(
    ui: &mut egui::Ui,
    p: &Palette,
    kb: &crate::keybinds::KeyBindings,
    st: &mut OptionsUi,
    a: crate::keybinds::Action,
    slot: usize,
) {
    let capturing = st.capture == Some(Capture::Binding(a, slot));
    let b = kb.get(a)[slot].clone();
    let (text, col) = if capturing {
        (aurora_i18n::tr!("common-press-a-key").to_owned(), Color32::WHITE)
    } else if let Some(b) = &b {
        (b.label(), p.ink)
    } else {
        ("—".to_owned(), p.muted_dim)
    };
    let fill = if capturing { p.violet } else { p.raised };
    let resp = ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(col))
            .fill(fill)
            .corner_radius(CornerRadius::same(2))
            .min_size(Vec2::new(132.0, 20.0)),
    );
    if resp.clicked() {
        st.capture = if capturing { None } else { Some(Capture::Binding(a, slot)) };
    }
    if resp.secondary_clicked() && b.is_some() {
        st.clear = Some((a, slot));
    }
    let others: Vec<&str> = b
        .as_ref()
        .map(|b| kb.users(b).into_iter().filter(|x| *x != a).map(crate::keybinds::label).collect())
        .unwrap_or_default();
    let tip = if capturing {
        aurora_i18n::tr!("ui-options-key-combination-ctrl-shift-alt-or-mouse-button-wheel").to_owned()
    } else if others.is_empty() {
        aurora_i18n::tr!("ui-options-click-change-right-click-clear").to_owned()
    } else {
        aurora_i18n::tr!(
            "ui-options-click-change-right-click-clear-also-used-by",
            value0 = others.join(", ").to_string()
        )
    };
    // conflict marker
    if !capturing && !others.is_empty() {
        ui.painter()
            .circle_filled(resp.rect.right_top() + Vec2::new(-4.0, 4.0), 3.0, p.amber);
    }
    resp.on_hover_text(tip);
}

/// "Raccourcis": every rebindable action, two bindings each.
fn keys_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut c = false;
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-options-click-a-shortcut-then-press-the-desired-key-or"))
            .size(12.0)
            .color(p.muted),
    );
    ui.add_space(6.0);
    let kb = s.keybinds.clone();
    for (section, actions) in crate::keybinds::sections() {
        group(ui, p, section, |ui| {
            for (a, label) in actions.iter() {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, 22.0), Sense::hover());
                    ui.painter().text(
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        *label,
                        egui::FontId::proportional(13.0),
                        p.ink,
                    );
                    for slot in 0..2 {
                        binding_cell(ui, p, &kb, st, *a, slot);
                    }
                });
            }
        });
    }
    if super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-restore-default-shortcuts")).clicked() {
        s.keybinds = Default::default();
        st.capture = None;
        c = true;
    }
    if let Some((a, slot)) = st.clear.take() {
        s.keybinds.set(a, slot, None);
        c = true;
    }
    c
}

/// Préférences › Son et voix › Sons de l'interface: each sound on / off, its
/// asset and a preview, the IM modes and the L$ threshold
/// (FSPanelPreferenceUISounds, fspanelpreferenceuisounds.cpp).
fn ui_sounds_group(ui: &mut egui::Ui, p: &Palette, a: &mut crate::settings::AudioSettings, st: &mut OptionsUi) -> bool {
    use crate::ui_sound::{CATALOG, ImSoundMode, UiSound};
    let mut c = false;
    group(ui, p, aurora_i18n::tr!("ui-options-interface-sounds"), |ui| {
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "ui-options-same-sounds-and-defaults-as-firestorm-these-are-second"
            ))
            .size(12.0)
            .color(p.muted),
        );
        ui.add_space(4.0);
        let u = &mut a.ui;
        let modes = [
            (
                aurora_i18n::tr!("ui-options-private-messages"),
                "PlayModeUISndNewIncomingIMSession",
                &mut u.im_mode,
                UiSound::NewIncomingImSession,
            ),
            (
                aurora_i18n::tr!("ui-options-group-messages"),
                "PlayModeUISndNewIncomingGroupIMSession",
                &mut u.group_mode,
                UiSound::NewIncomingGroupImSession,
            ),
            (
                aurora_i18n::tr!("ui-options-conferences"),
                "PlayModeUISndNewIncomingConfIMSession",
                &mut u.conf_mode,
                UiSound::NewIncomingConfImSession,
            ),
        ];
        let mut previews = Vec::new();
        for (label, hint, mode, sound) in modes {
            c |= row(ui, p, label, hint, |ui| {
                let mut ch = false;
                egui::ComboBox::from_id_salt(hint)
                    .width(SLIDER_W)
                    .selected_text(mode.label())
                    .show_ui(ui, |ui| {
                        for m in ImSoundMode::ALL {
                            ch |= ui.selectable_value(mode, m, m.label()).changed();
                        }
                    });
                if super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-listen")).clicked() {
                    previews.push(sound);
                }
                ch
            });
        }
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-l-sound-threshold"),
            aurora_i18n::tr!("ui-options-balance-change-above-which-received-spent-l-sounds-are"),
            |ui| {
                ui.add(
                    egui::DragValue::new(&mut u.money_threshold)
                        .custom_parser(aurora_i18n::parse_number)
                        .range(0.0..=100_000.0)
                        .speed(1.0)
                        .suffix(" L$"),
                )
                .changed()
            },
        );
        egui::CollapsingHeader::new(
            RichText::new(aurora_i18n::tr!("ui-options-per-sound-choices"))
                .size(13.0)
                .color(p.ink),
        )
        .id_salt("ui_sounds_each")
        .show(ui, |ui| {
            for e in CATALOG
                .iter()
                .filter(|e| !e.sound.caller_gated() || e.sound == UiSound::TrackerBeacon)
            {
                let hint = aurora_i18n::tr!(
                    "extra-uisnd-sound-asset-empty-no-sound-playmodeuisnd",
                    value0 = e.name.to_string(),
                    value1 = e.name.to_string()
                );
                c |= row(ui, p, (e.label)(), &hint, |ui| {
                    let mut on = u.plays(e.sound);
                    let mut ch = super::widgets::switch(ui, p, &mut on).changed();
                    if ch {
                        u.set_plays(e.sound, on);
                    }
                    let text = st.ui_sound_text.entry(e.name).or_insert_with(|| uuid_text(u.uuid(e.sound)));
                    let parsed = parse_uuid_text(text);
                    let edit = ui.add(
                        egui::TextEdit::singleline(text)
                            .desired_width(290.0)
                            .font(egui::TextStyle::Monospace)
                            .text_color(if parsed.is_some() { p.ink } else { p.danger })
                            .hint_text(aurora_i18n::tr!("ui-options-no-sound")),
                    );
                    if edit.changed()
                        && let Some(id) = parse_uuid_text(text)
                    {
                        u.set_uuid(e.sound, id);
                        ch = true;
                    }
                    if super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-listen")).clicked() {
                        previews.push(e.sound);
                    }
                    if u.uuid(e.sound) != e.uuid && super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-default")).clicked() {
                        u.set_uuid(e.sound, e.uuid);
                        *text = uuid_text(e.uuid);
                        ch = true;
                    }
                    ch
                });
            }
        });
        // force_sound: heard even when switched off
        for s in previews {
            if let Some(id) = u.resolve(s, true) {
                super::sound_cues::preview(ui.ctx(), id);
            }
        }
    });
    c
}

fn uuid_text(id: uuid::Uuid) -> String {
    if id.is_nil() { String::new() } else { id.to_string() }
}

/// A typed sound asset: a UUID, or empty for no sound.
fn parse_uuid_text(text: &str) -> Option<uuid::Uuid> {
    let t = text.trim();
    if t.is_empty() {
        Some(uuid::Uuid::nil())
    } else {
        uuid::Uuid::parse_str(t).ok()
    }
}
