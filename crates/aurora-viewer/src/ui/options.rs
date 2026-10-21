//! Preferences floater: categories on the left, grouped settings on the
//! right (label column + control column), flat Aurora style.

mod accessibility;
mod audio;
pub use audio::AudioSection;
pub(crate) use audio::{ear_choice, noise_choice};
mod aurora;
mod camera;
mod commands;
mod dof;
pub mod privacy;
mod scale;
pub(crate) mod sections;

use super::fonts::{self, SystemFont};
use super::icons::Icons;
use super::widgets::{Floater, keycap};
use crate::settings::Settings;
use crate::theme::Palette;
use egui::{RichText, Sense, Vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    Binding(crate::keybinds::Action, usize),
}

#[derive(Default)]
pub struct OptionsUi {
    pub tab: usize,
    interface_scale: scale::ScaleSlider,
    /// The account's content ceiling and the rating it wants, while logged
    /// in (set by the app).
    pub maturity: Option<aurora_net::search::AgentAccess>,
    /// The account settings the grid keeps (Privacy), while logged in (set
    /// by the app).
    pub account: Option<privacy::Account>,
    system_fonts: Option<Vec<SystemFont>>,
    font_filter: String,
    pub shortcut_filter: String,
    /// Disk cache usage (bytes) and whether a clear is running; set by the app.
    pub cache_usage: u64,
    pub cache_clearing: bool,
    /// Detected video memory (MB) and integrated GPU; set by the app.
    pub vram: (Option<u64>, bool),
    /// The sky in use is tone mapped (not classic); set by the app each
    /// frame: the tone mapper choice is greyed otherwise.
    pub sky_tone_mapped: bool,
    /// Audio devices (set by the app) and microphone level (0..1).
    pub output_devices: Vec<String>,
    pub input_devices: Vec<String>,
    pub mic_level: f32,
    /// Media plugins folder found (empty: none) and plugins running.
    pub media_plugins: String,
    pub media_running: usize,
    /// Aurora's media plugins pack (set by the app), and its "Retry" button
    /// (cleared by the app).
    pub media_pack: crate::media::pack::PackState,
    pub media_pack_retry: bool,
    /// Shortcut or gesture waiting for its next input, captured by the app.
    pub capture: Option<Capture>,
    capture_modifier: Option<winit::keyboard::KeyCode>,
    clear: Option<(crate::keybinds::Action, usize)>,
    /// "Reset everything" confirmation open.
    confirm_reset: bool,
    /// "Reset this section" confirmation open, with the page it resets.
    confirm_reset_section: Option<usize>,
    /// Sound assets being typed (Firestorm name -> text).
    ui_sound_text: std::collections::HashMap<&'static str, String>,
    /// The keyword alerts' sound asset being typed, and the asset it was
    /// started from.
    keyword_sound_text: Option<(uuid::Uuid, String)>,
    /// Folder of the conversation transcripts of the account logged in
    /// (set by the app; none on the login screen).
    pub chat_log_dir: Option<std::path::PathBuf>,
    /// What the last erasing of the transcripts did (set by the app).
    pub chat_log_status: String,
    /// The transcripts are in a folder the user chose (set by the app).
    pub chat_log_custom: bool,
    /// "Erase this account's history?" confirmation open.
    confirm_erase_logs: bool,
    /// The page opens scrolled to its end (`AURORA_DEMO_OPTIONS=<n>:end`).
    pub scroll_to_end: bool,
    /// Graphics opens on the tone mapping row (`AURORA_DEMO_OPTIONS=0:tonemap`).
    pub scroll_to_tone_mapping: bool,
    /// Graphics opens on the indirect light row (`0:occlusion`), between
    /// the contact shadows and the ambient occlusion, or on the bloom row
    /// (`0:bloom`): the rows the Deluxe level turns on.
    pub scroll_to_occlusion: bool,
    pub scroll_to_bloom: bool,
    pub scroll_to_name_tags: bool,
    /// The page opens on the first (1) or second (2) of its marked rows
    /// (`AURORA_DEMO_OPTIONS=<n>:new` / `:new2`, captures).
    pub scroll_to_new: u8,
    /// Chat opens on the taskbar and system notifications (`3:attention`).
    pub scroll_to_attention: bool,
    /// Chat opens on the keyword alerts (`3:keywords`).
    pub scroll_to_keywords: bool,
    pub audio_section: Option<AudioSection>,
    pub restart_voice: bool,
}

impl OptionsUi {
    pub fn prepare_scale_input(&mut self, input: &egui::RawInput, pixels_per_point: f32, numeric_captured: bool) {
        self.interface_scale.prepare_input(input, pixels_per_point, numeric_captured);
    }

    /// A HUD modifier on its own is captured on release, leaving time to
    /// press a second key for combinations such as Ctrl+B.
    pub fn capture_key(&mut self, code: winit::keyboard::KeyCode, pressed: bool, repeat: bool) -> Option<Option<crate::keybinds::Input>> {
        use crate::keybinds::{Action, Input};
        use winit::keyboard::KeyCode;
        let capture = self.capture?;
        if repeat {
            return None;
        }
        if pressed && code == KeyCode::Escape {
            return Some(None);
        }
        if Input::is_modifier(code) {
            if matches!(capture, Capture::Binding(Action::HudDrag, _)) {
                if pressed {
                    self.capture_modifier = Some(code);
                } else if self.capture_modifier == Some(code) {
                    return Some(Some(Input::key(code)));
                }
            }
            return None;
        }
        pressed.then(|| Some(Input::key(code)))
    }

    /// Opens the "Reset this section" confirmation for the page shown.
    pub fn ask_reset_section(&mut self) {
        self.confirm_reset_section = Some(self.tab);
    }

    pub fn take_capture(&mut self) -> Option<Capture> {
        self.capture_modifier = None;
        self.capture.take()
    }
}

#[derive(Default)]
pub struct OptionsResult {
    /// Settings that need the general application path (rendering, network,
    /// crash recovery, theme). UI zoom has its own per-frame path below.
    pub changed: bool,
    pub ui_scale_changed: bool,
    pub font_changed: bool,
    pub clear_cache: bool,
    /// Devices, volumes or voice settings changed: apply to the audio engine.
    pub audio_changed: bool,
    /// List the audio devices again.
    pub refresh_devices: bool,
    /// Every setting went back to its default.
    pub reset: bool,
    /// The settings of one page went back to their defaults (its tab).
    pub reset_section: Option<usize>,
    /// The camera settings went back to their defaults: the view follows.
    pub reset_camera: bool,
    /// Show the folder of the conversation transcripts.
    pub open_chat_logs: bool,
    /// Choose another folder for the cache or the transcripts, or go back
    /// to the default one (the app asks for the folder and moves the files).
    pub folder: Option<(Folder, FolderChoice)>,
    /// Erase the conversation transcripts of the account (confirmed).
    pub erase_chat_logs: bool,
    /// The content rating chosen (SIM_ACCESS_*), to send to the server.
    pub maturity: Option<u8>,
    /// An account setting of Privacy changed, to send to the server.
    pub account: Option<crate::world::user_info::Change>,
}

/// A folder the user can choose (Preferences › Network and cache, Chat).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folder {
    Cache,
    ChatLogs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderChoice {
    /// Ask for a folder.
    Pick,
    /// Back to the default folder.
    Default,
}

/// A row the demo can open the page on (`scroll_to_new`): the marks of a
/// page count down in the order they are drawn.
fn scroll_mark(ui: &mut egui::Ui, st: &mut OptionsUi) {
    if st.scroll_to_new == 0 {
        return;
    }
    st.scroll_to_new -= 1;
    if st.scroll_to_new == 0 {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
}

/// A path as shown in the preferences: elided to the width, whole on hover.
fn path_label(ui: &mut egui::Ui, p: &Palette, dir: &std::path::Path) {
    // without the verbatim prefix Windows gives a canonical path
    let path = dir.display().to_string();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned();
    ui.add(super::widgets::elided_label(
        ui,
        RichText::new(&path).size(11.5).color(p.muted_dim),
        ui.available_width(),
    ))
    .on_hover_text(&path);
}

/// Category tabs (index = `OptionsUi::tab`).
fn categories() -> [(&'static str, &'static str); 15] {
    [
        (aurora_i18n::tr!("ui-options-graphics"), "device-desktop-filled"),
        (aurora_i18n::tr!("common-interface"), "app-window-filled"),
        (aurora_i18n::tr!("ui-options-fonts"), "file-typography-filled"),
        (aurora_i18n::tr!("common-chat"), "messages-filled"),
        (aurora_i18n::tr!("ui-options-controls"), "device-gamepad-filled"),
        (aurora_i18n::tr!("ui-options-colors"), "palette-filled"),
        (aurora_i18n::tr!("ui-options-network-and-cache"), "cloud-data-connection-filled"),
        (aurora_i18n::tr!("ui-options-sound-and-voice"), "headset-filled"),
        (aurora_i18n::tr!("ui-options-shortcuts"), "keyboard-filled"),
        (aurora_i18n::tr!("common-debug"), "code-circle-filled"),
        (aurora_i18n::tr!("common-camera"), "camera-filled"),
        (aurora_i18n::tr!("ui-options-commands"), "square-rounded-chevron-right-filled"),
        (aurora_i18n::tr!("ui-options-privacy"), "lock-filled"),
        (aurora_i18n::tr!("ui-options-aurora"), "sparkles-filled"),
        (aurora_i18n::tr!("ui-options-accessibility"), "accessible-filled"),
    ]
}

/// The name of a page, as in the sidebar.
pub fn tab_name(tab: usize) -> &'static str {
    categories()[tab.min(categories().len() - 1)].0
}

/// Number of pages.
#[cfg(test)]
fn tab_count() -> usize {
    categories().len()
}

/// Order of the tabs in the sidebar.
const ORDER: [usize; 14] = [
    TAB_AURORA,
    0,
    1,
    2,
    TAB_ACCESSIBILITY,
    3,
    TAB_COMMANDS,
    7,
    4,
    TAB_CAMERA,
    8,
    5,
    TAB_PRIVACY,
    6,
];

pub const TAB_GRAPHICS: usize = 0;
pub const TAB_INTERFACE: usize = 1;
pub const TAB_NETWORK: usize = 6;
pub const TAB_AUDIO: usize = 7;
pub const TAB_CHAT: usize = 3;
pub const TAB_CAMERA: usize = 10;
pub const TAB_COMMANDS: usize = 11;
pub const TAB_PRIVACY: usize = 12;
/// What Aurora does differently from Firestorm, with the way back to it.
pub const TAB_AURORA: usize = 13;
/// Colour blindness (#418), and what helps reading the interface.
pub const TAB_ACCESSIBILITY: usize = 14;
/// The former Debug page, now the Dev Toolbox (ui/dev_toolbox.rs): its index
/// stays reserved so that the other pages keep theirs; it is never shown.
pub const TAB_RETIRED_DEBUG: usize = 9;

const LABEL_W: f32 = 180.0;

fn category(ui: &mut egui::Ui, p: &Palette, _icons: &Icons, label: &str, icon: &str, selected: bool) -> bool {
    super::widgets::navigation(ui, p, icon, label, selected).clicked()
}

/// A titled group of settings drawn as a flat card.
/// Tabler icon of a settings group (by its title).
fn group_icon(title: &str) -> Option<&'static str> {
    Some(match title {
        value if value == aurora_i18n::text("voice-processing") => "adjustments-filled",
        value if value == aurora_i18n::text("voice-indicators") => "wave-sine",
        value if value == aurora_i18n::text("ui-options-presets") => "gauge-filled",
        value if value == aurora_i18n::text("ui-options-quality") => "adjustments-filled",
        value if value == aurora_i18n::text("common-light") => "sun-filled",
        value if value == aurora_i18n::text("ui-options-reflections") => "droplet-filled",
        value if value == aurora_i18n::text("common-image") => "photo-filled",
        value if value == aurora_i18n::text("ui-options-smoothness") => "dashboard-filled",
        value if value == aurora_i18n::text("ui-options-layout") => "layout-filled",
        value if value == aurora_i18n::text("preferences-group-skin") => "paint-filled",
        value if value == aurora_i18n::text("ui-options-size-and-preview") => "text-size",
        value if value == aurora_i18n::text("ui-options-bundled-fonts") => "typography",
        value if value == aurora_i18n::text("ui-options-system-fonts") => "device-desktop-filled",
        value if value == aurora_i18n::text("common-nearby-chat") => "message-filled",
        value if value == aurora_i18n::text("ui-options-messages") => "messages-filled",
        value if value == aurora_i18n::text("ui-options-chat-logs") => "history",
        value if value == aurora_i18n::text("ui-options-emitter") => "user-filled",
        value if value == aurora_i18n::text("ui-options-chat-mentions") => "at",
        value if value == aurora_i18n::text("ui-options-attention") => "bell-ringing-filled",
        value if value == aurora_i18n::text("ui-options-mouse") => "mouse-filled",
        value if value == aurora_i18n::text("preferences-group-huds") => "hand-grab",
        value if value == aurora_i18n::text("ui-options-keyboard") => "keyboard-filled",
        value if value == aurora_i18n::text("ui-options-color-blindness") => "eye-filled",
        value if value == aurora_i18n::text("ui-options-name-tag-color") => "tag-filled",
        value if value == aurora_i18n::text("ui-options-name-tags") => "tag-filled",
        value if value == aurora_i18n::text("ui-options-mini-map-markers") => "map",
        value if value == aurora_i18n::text("ui-options-disk-cache") => "database-filled",
        value if value == aurora_i18n::text("ui-options-devices") => "device-speaker-filled",
        value if value == aurora_i18n::text("ui-options-volumes") => "volume",
        value if value == aurora_i18n::text("common-voice") => "wave-sine",
        value if value == aurora_i18n::text("ui-options-push-to-talk") => "microphone-filled",
        value if value == aurora_i18n::text("common-movement") => "walk",
        value if value == aurora_i18n::text("common-camera") || value == aurora_i18n::text("common-viewpoint") => "camera-filled",
        value if value == aurora_i18n::text("common-camera-movement") => "adjustments-filled",
        value if value == aurora_i18n::text("common-behavior") => "toggle-right-filled",
        value if value == aurora_i18n::text("common-limits") => "barrier-block-filled",
        value if value == aurora_i18n::text("common-mouselook") => "crosshair",
        value if value == aurora_i18n::text("ui-options-head-follows-mouse") => "eye-filled",
        value if value == aurora_i18n::text("ui-options-look-at-targets") => "crosshair",
        value if value == aurora_i18n::text("ui-options-my-look-at") => "lock-filled",
        value if value == aurora_i18n::text("common-communication") => "messages-filled",
        value if value == aurora_i18n::text("ui-options-groups-and-blocking") => "ban",
        value if value == aurora_i18n::text("ui-options-auto-replies") => "message-chatbot-filled",
        value if value == aurora_i18n::text("ui-options-automatic-rejections") => "thumb-down-filled",
        value if value == aurora_i18n::text("common-command-line") || value == aurora_i18n::text("common-other-commands") => "terminal-2",
        value if value == aurora_i18n::text("common-teleport-commands") => "map-pin-filled",
        value if value == aurora_i18n::text("common-teleport-2") => "location-filled",
        value if value == aurora_i18n::text("ui-options-region-restart") => "refresh",
        value if value == aurora_i18n::text("bumps-title") => "bounce-right-filled",
        value if value == aurora_i18n::text("common-camera-and-display-commands") => "camera-filled",
        value if value == aurora_i18n::text("common-windows") => "layout-grid-filled",
        value if value == aurora_i18n::text("ui-options-aurora-appearance") => "paint-filled",
        value if value == aurora_i18n::text("ui-options-aurora-gaze") => "eye-filled",
        value if value == aurora_i18n::text("ui-options-aurora-movement") => "walk",
        value if value == aurora_i18n::text("ui-options-aurora-animations") => "player-play-filled",
        value if value == aurora_i18n::text("ui-options-aurora-avatar-plates") => "tag-filled",
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
        ui.label(RichText::new(title).size(13.0).strong().color(p.ink));
    });
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            body(ui);
        });
    ui.add_space(10.0);
}

/// Room kept between a label that fills its column and the control.
const LABEL_GAP: f32 = 4.0;

/// The label of a row, in a cell `LABEL_W` wide so that the controls of a
/// page stand in one column. A label longer than the column (a translation,
/// a larger font) goes on at the line below instead of running under the
/// control: the row grows for it alone, the column does not move.
fn row_label(ui: &mut egui::Ui, p: &Palette, label: &str) -> egui::Response {
    row_label_marked(ui, p, label, false)
}

/// The size of the experimental mark after a label, and its gap.
const MARK: f32 = 13.0;

/// `row_label`, with the experimental mark right after the text when
/// `experimental` (the text wraps before it).
fn row_label_marked(ui: &mut egui::Ui, p: &Palette, label: &str, experimental: bool) -> egui::Response {
    // the spacing before the control is room for the text too, less a gap
    let mark = if experimental { MARK + LABEL_GAP } else { 0.0 };
    let room = LABEL_W + ui.spacing().item_spacing.x - LABEL_GAP - mark;
    let galley = egui::WidgetText::from(RichText::new(label).size(13.0).color(p.ink)).into_galley(
        ui,
        Some(egui::TextWrapMode::Wrap),
        room,
        egui::TextStyle::Body,
    );
    let height = galley.size().y.max(22.0);
    let size = galley.size();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(LABEL_W, height), Sense::hover());
    let top = rect.center().y - size.y * 0.5;
    ui.painter().galley(egui::pos2(rect.left(), top), galley, p.ink);
    if experimental {
        let at = egui::pos2(rect.left() + size.x + LABEL_GAP + MARK * 0.5, rect.center().y);
        super::widgets::experimental_mark(
            ui,
            p,
            egui::Rect::from_center_size(at, Vec2::splat(MARK)),
            resp.id.with("experimental"),
        );
    }
    resp
}

/// `row` for an experimental option: its label carries the warning mark.
fn row_experimental(ui: &mut egui::Ui, p: &Palette, label: &str, hint: &str, control: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let resp = row_label_marked(ui, p, label, true);
        if !hint.is_empty() {
            resp.on_hover_text(hint);
        }
        changed = control(ui);
    });
    changed
}

/// One row: label (with optional hint) on the left, control on the right.
fn row(ui: &mut egui::Ui, p: &Palette, label: &str, hint: &str, control: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let resp = row_label(ui, p, label);
        if !hint.is_empty() {
            resp.on_hover_text(hint);
        }
        changed = control(ui);
    });
    changed
}

/// Why Aurora's media plugins pack is not there yet.
fn media_pack_error(failure: aurora_updater::Failure) -> &'static str {
    use aurora_updater::Failure;
    match failure {
        Failure::Network | Failure::Busy | Failure::Startup => aurora_i18n::tr!("media-plugins-error-network"),
        Failure::Integrity | Failure::Package => aurora_i18n::tr!("media-plugins-error-integrity"),
        Failure::Filesystem => aurora_i18n::tr!("media-plugins-error-filesystem"),
        Failure::Unsupported => aurora_i18n::tr!("media-plugins-error-unsupported"),
    }
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

/// Slider of whole values (Firestorm's decimal_digits="0").
fn slider_whole(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    ui.add(
        egui::Slider::new(v, range)
            .suffix(suffix)
            .custom_parser(aurora_i18n::parse_number)
            .fixed_decimals(0)
            .step_by(1.0)
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

/// Color swatch opening the compact picker (opaque); `id` tells the
/// swatches of a page apart.
fn swatch(ui: &mut egui::Ui, p: &Palette, id: impl egui::AsIdSalt, v: &mut crate::settings::Rgba) -> bool {
    ui.spacing_mut().interact_size = Vec2::new(38.0, 18.0);
    super::widgets::color_button(ui, p, id, v, false).changed()
}

/// Grid of labelled swatches, three per row (Firestorm color preferences).
fn swatches(ui: &mut egui::Ui, p: &Palette, id: &str, items: &mut [(&str, &mut crate::settings::Rgba)]) -> bool {
    let mut changed = false;
    egui::Grid::new(id).num_columns(3).spacing([24.0, 8.0]).show(ui, |ui| {
        for (i, (label, v)) in items.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.set_min_width(150.0);
                changed |= swatch(ui, p, (id, i), v);
                ui.label(RichText::new(*label).size(12.5));
            });
            if i % 3 == 2 {
                ui.end_row();
            }
        }
    });
    changed
}

/// The colors of a display name that matches or not the username belong to
/// the classic tags: a plate writes every name in the theme's ink.
fn tag_colors_used(s: &Settings) -> bool {
    s.classic_name_tags
}

/// The name colors of the Colors page: by relation (me, friends, blocked,
/// Linden Lab) behind a switch, off by default, for the plates and the
/// classic tags alike; then the classic tags' own pair. Greyed rather than
/// hidden, so that the page keeps its shape.
fn tag_colors(ui: &mut egui::Ui, p: &Palette, k: &mut crate::settings::ColorSettings, classic: bool) -> bool {
    let mut changed = false;
    group(ui, p, aurora_i18n::tr!("ui-options-name-tag-color"), |ui| {
        let label = RichText::new(aurora_i18n::tr!("ui-options-names-by-relation"))
            .size(12.5)
            .color(p.ink);
        changed |= super::widgets::switch_labeled(ui, p, &mut k.relation_names, label).changed();
        let note = aurora_i18n::tr!("ui-options-names-by-relation-note");
        ui.label(RichText::new(note).size(11.5).color(p.muted));
        ui.add_space(4.0);
        ui.add_enabled_ui(k.relation_names, |ui| {
            changed |= swatches(
                ui,
                p,
                "tags",
                &mut [
                    (aurora_i18n::tr!("ui-options-my-name"), &mut k.tag_me),
                    (aurora_i18n::tr!("common-friends"), &mut k.tag_friend),
                    (aurora_i18n::tr!("ui-options-ignored"), &mut k.tag_muted),
                    ("Linden Lab", &mut k.tag_linden),
                ],
            );
        });
        ui.add_space(8.0);
        if !classic {
            let note = aurora_i18n::tr!("ui-options-tag-colors-classic-only");
            ui.label(RichText::new(note).size(11.5).color(p.muted));
        }
        ui.add_enabled_ui(classic, |ui| {
            changed |= swatches(
                ui,
                p,
                "tags-match",
                &mut [
                    (aurora_i18n::tr!("ui-options-matches"), &mut k.tag_match),
                    (aurora_i18n::tr!("ui-options-non-matches"), &mut k.tag_mismatch),
                ],
            );
        });
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

/// The content ratings an account may choose (LLFloaterPreference: the
/// list without "Adult" for an account that is not adult; no choice at all
/// below Moderate), most open first.
pub fn maturity_choices(access: aurora_net::search::AgentAccess) -> Vec<(u8, &'static str)> {
    use aurora_net::search::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};
    let all = [
        (SIM_ACCESS_ADULT, aurora_i18n::tr!("search-maturity-gma")),
        (SIM_ACCESS_MATURE, aurora_i18n::tr!("search-maturity-gm")),
        (SIM_ACCESS_PG, aurora_i18n::tr!("ui-options-content-general")),
    ];
    if access.max < SIM_ACCESS_MATURE {
        return Vec::new();
    }
    all.into_iter().filter(|(m, _)| *m <= access.max).collect()
}

/// "I want to access content rated" (maturity_desired_combobox): a list,
/// or the rating as text when the account has no choice.
fn maturity_choice(ui: &mut egui::Ui, p: &Palette, access: aurora_net::search::AgentAccess) -> Option<u8> {
    let choices = maturity_choices(access);
    let label = |m: u8| match m {
        aurora_net::search::SIM_ACCESS_ADULT => aurora_i18n::tr!("search-maturity-gma"),
        aurora_net::search::SIM_ACCESS_MATURE => aurora_i18n::tr!("search-maturity-gm"),
        _ => aurora_i18n::tr!("ui-options-content-general"),
    };
    if choices.is_empty() {
        ui.horizontal(|ui| {
            super::widgets::maturity_badges_up_to(ui, p, access.preferred, 16.0);
            ui.label(label(access.preferred));
        });
        return None;
    }
    let mut picked = access.preferred;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("ui-maturity")
            .selected_text(label(access.preferred))
            .show_ui(ui, |ui| {
                for (m, name) in &choices {
                    // the badges of the navigation bar, as Firestorm's
                    // rating icons, before each choice
                    ui.horizontal(|ui| {
                        ui.add_sized(Vec2::new(54.0, 16.0), |ui: &mut egui::Ui| {
                            ui.horizontal(|ui| super::widgets::maturity_badges_up_to(ui, p, *m, 14.0)).response
                        });
                        if ui.selectable_label(picked == *m, *name).clicked() {
                            picked = *m;
                        }
                    });
                }
            });
        super::widgets::maturity_badges_up_to(ui, p, access.preferred, 16.0);
    });
    (picked != access.preferred).then_some(picked)
}

/// "Reset this section…" of a page: neutral, red on hover, like "Reset
/// everything…" (#291); `compact` keeps the icon alone (the Aurora page's
/// head, where its own two buttons take the room).
fn reset_section_button(ui: &mut egui::Ui, p: &Palette, tab: usize, compact: bool) -> bool {
    let label = if compact {
        ""
    } else {
        aurora_i18n::tr!("ui-options-reset-section")
    };
    let b = super::widgets::action_button(
        ui,
        p,
        "restore",
        label,
        super::widgets::ActionKind::FlatDanger,
        Vec2::new(if compact { 26.0 } else { 0.0 }, 26.0),
    );
    b.on_hover_text(aurora_i18n::tr!("ui-options-reset-section-help", section = tab_name(tab)))
        .clicked()
}

/// The five graphics levels, side by side; none is lit once a setting of
/// the page no longer matches any of them ("Custom", like Firestorm's
/// slider after a manual change).
fn preset_choice(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    use crate::graphics_presets::Preset;
    let current = Preset::matching(s);
    let options: Vec<(&str, bool)> = Preset::ALL.iter().map(|l| (l.label(), true)).collect();
    let mut selected = current.and_then(|c| Preset::ALL.iter().position(|l| *l == c)).unwrap_or(usize::MAX);
    let changed = super::widgets::segmented(ui, p, &mut selected, &options);
    if current.is_none() && !changed {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-options-preset-custom"))
                .size(12.0)
                .color(p.violet_pale),
        );
    }
    match Preset::ALL.get(selected) {
        Some(level) if changed => {
            level.apply(s);
            true
        }
        _ => false,
    }
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
    if st.tab == TAB_RETIRED_DEBUG {
        st.tab = TAB_AURORA;
    }
    let screen = ctx.content_rect();
    Floater::new(
        "options",
        aurora_i18n::tr!("common-preferences"),
        egui::pos2(screen.center().x - 330.0, 80.0),
        Vec2::new(780.0, 580.0),
    )
    .help(aurora_i18n::tr!("ui-options-changes-apply-immediately-and-are-saved-on-closing"))
    .show(ctx, p, open, |ui| {
        let h = (ui.available_height() - 4.0).max(200.0);
        ui.horizontal_top(|ui| {
            // ---- categories
            ui.allocate_ui_with_layout(Vec2::new(168.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let reset_height = 26.0;
                let reset_gap = 4.0;
                let categories_height = (h - reset_height - reset_gap - ui.spacing().item_spacing.y).max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("preferences_categories")
                    .auto_shrink([false, false])
                    .max_height(categories_height)
                    .min_scrolled_height(categories_height)
                    .show(ui, |ui| {
                        for i in ORDER {
                            let (label, icon) = categories()[i];
                            let clicked = if i == TAB_AURORA {
                                let clicked = aurora::nav_entry(ui, p, label, icon, st.tab == i).clicked();
                                ui.add_space(6.0);
                                clicked
                            } else {
                                category(ui, p, icons, label, icon, st.tab == i)
                            };
                            if clicked {
                                st.capture = None;
                                st.tab = i;
                            }
                        }
                    });
                // reset everything, at the bottom of the column: a real button
                // that turns red on hover, like the erasing of the chat logs;
                // the confirmation below carries the red one (#291)
                ui.add_space(reset_gap);
                let b = super::widgets::action_button(
                    ui,
                    p,
                    "restore",
                    aurora_i18n::tr!("ui-options-reset-everything-2"),
                    super::widgets::ActionKind::FlatDanger,
                    Vec2::new(ui.available_width(), reset_height),
                );
                if b.on_hover_text(aurora_i18n::tr!("ui-options-reset-all-settings")).clicked() {
                    st.confirm_reset = true;
                }
            });
            ui.add_space(6.0);
            // ---- content
            ui.vertical(|ui| {
                let top = ui.cursor().top();
                // the Aurora page carries its title in its own head
                if st.tab != TAB_AURORA {
                    // the page's title, and the reset of this page alone at
                    // its right (the same button as "Reset everything…")
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tab_name(st.tab)).size(17.0).color(p.ink));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(8.0);
                            if reset_section_button(ui, p, st.tab, false) {
                                st.confirm_reset_section = Some(st.tab);
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
                let mut scroll = egui::ScrollArea::vertical().stick_to_bottom(st.scroll_to_end);
                if st.tab == 8 {
                    let search = shortcut_search(ui, p, st);
                    if search.inner {
                        scroll = scroll.vertical_scroll_offset(0.0);
                    }
                }
                // The header grows with its font. A fixed subtraction makes
                // header + viewport exceed h, feeding growth back every frame.
                let scroll_height = (h - (ui.cursor().top() - top)).max(0.0);
                scroll
                    .auto_shrink([false, false])
                    .max_height(scroll_height)
                    .min_scrolled_height(scroll_height)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width() - 8.0);
                        r.changed |= match st.tab {
                            TAB_AUDIO => audio::page(ui, p, s, st, &mut r.audio_changed, &mut r.refresh_devices),
                            8 => keys_page(ui, p, s, st),
                            TAB_RETIRED_DEBUG => false,
                            TAB_CAMERA => camera::page(ui, p, s, &mut r.reset_camera),
                            TAB_COMMANDS => commands::page(ui, p, &mut s.chat_commands),
                            TAB_PRIVACY => privacy::page(
                                ui,
                                p,
                                &mut s.lookat,
                                &mut s.autoresponse.friends_only,
                                &mut s.object_im_filter,
                                st.account.as_ref(),
                                &mut r.account,
                            ),
                            TAB_ACCESSIBILITY => accessibility::page(ui, p, s),
                            TAB_AURORA => {
                                let mut reset = false;
                                let c = aurora::page(ui, p, s, &mut reset);
                                if reset {
                                    st.confirm_reset_section = Some(TAB_AURORA);
                                }
                                c
                            }
                            _ => content(ui, p, s, st, &mut r),
                        };
                    });
            });
        });
    });
    if st.confirm_reset && *open {
        let modal = crate::ui::widgets::Modal::new(egui::Id::new("options_reset")).show(ctx, |ui| {
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
                use super::widgets::{ActionKind, action_button};
                let size = Vec2::new(0.0, 26.0);
                let yes = action_button(ui, p, "restore", aurora_i18n::tr!("common-reset"), ActionKind::Danger, size);
                if yes.clicked() {
                    s.reset_keeping_account();
                    st.capture = None;
                    r.changed = true;
                    r.font_changed = true;
                    r.audio_changed = true;
                    r.reset = true;
                    st.confirm_reset = false;
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    st.confirm_reset = false;
                }
            });
        });
        if modal.should_close() {
            st.confirm_reset = false;
        }
    }
    if let Some(tab) = st.confirm_reset_section.filter(|_| *open) {
        let section = tab_name(tab);
        let modal = crate::ui::widgets::Modal::new(egui::Id::new("options_reset_section")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-options-reset-section-title", section = section))
                    .size(15.0)
                    .strong()
                    .color(p.ink),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-options-reset-section-text"))
                    .size(12.5)
                    .color(p.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                use super::widgets::{ActionKind, action_button};
                let size = Vec2::new(0.0, 26.0);
                if action_button(ui, p, "restore", aurora_i18n::tr!("common-reset"), ActionKind::Danger, size).clicked() {
                    sections::reset(tab, s);
                    st.capture = None;
                    r.changed = true;
                    r.font_changed = true;
                    r.audio_changed = tab == TAB_AUDIO;
                    r.reset_camera = tab == TAB_CAMERA;
                    r.reset_section = Some(tab);
                    st.confirm_reset_section = None;
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    st.confirm_reset_section = None;
                }
            });
        });
        if modal.should_close() {
            st.confirm_reset_section = None;
        }
    }
    if st.confirm_erase_logs && *open {
        let modal = crate::ui::widgets::Modal::new(egui::Id::new("options_erase_chat_logs")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-options-chat-logs-erase-title"))
                    .size(15.0)
                    .strong()
                    .color(p.ink),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-options-chat-logs-erase-text"))
                    .size(12.5)
                    .color(p.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                use super::widgets::{ActionKind, action_button};
                let size = Vec2::new(0.0, 26.0);
                if action_button(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("ui-options-chat-logs-erase-confirm"),
                    ActionKind::Danger,
                    size,
                )
                .clicked()
                {
                    r.erase_chat_logs = true;
                    st.confirm_erase_logs = false;
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    st.confirm_erase_logs = false;
                }
            });
        });
        if modal.should_close() {
            st.confirm_erase_logs = false;
        }
    }
    r
}

/// Preferences › Chat › Conversation history: what is written to
/// the account's transcripts and where they are (Firestorm: Preferences ›
/// Privacy, "Logs & Transcripts").
fn chat_logs_group(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult) -> bool {
    let mut c = false;
    group(ui, p, aurora_i18n::tr!("ui-options-chat-logs"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-chat-logs-im"),
            aurora_i18n::tr!("ui-options-chat-logs-im-help"),
            |ui| toggle(ui, p, &mut s.log_im),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-chat-logs-nearby"),
            aurora_i18n::tr!("ui-options-chat-logs-nearby-help"),
            |ui| toggle(ui, p, &mut s.log_nearby),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-chat-logs-show"),
            aurora_i18n::tr!("ui-options-chat-logs-show-help"),
            |ui| toggle(ui, p, &mut s.log_show_history),
        );
        // Legacy Viewer › Chat (FSRestoreOpenIMs)
        scroll_mark(ui, st);
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-reopen-ims"),
            aurora_i18n::tr!("ui-options-reopen-ims-help"),
            |ui| toggle(ui, p, &mut s.reopen_ims),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-chat-logs-dated"),
            aurora_i18n::tr!("ui-options-chat-logs-dated-help"),
            |ui| toggle(ui, p, &mut s.log_dated_names),
        );
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-options-chat-logs-note"))
                .size(11.5)
                .color(p.muted),
        );
        match &st.chat_log_dir {
            Some(dir) => {
                path_label(ui, p, dir);
                ui.horizontal(|ui| {
                    use super::widgets::{ActionKind, action_button};
                    let size = Vec2::new(0.0, 24.0);
                    if action_button(
                        ui,
                        p,
                        "folder-open-filled",
                        aurora_i18n::tr!("ui-options-chat-logs-open"),
                        ActionKind::Flat,
                        size,
                    )
                    .clicked()
                    {
                        r.open_chat_logs = true;
                    }
                    // Legacy Viewer › Network & Files (InstantMessageLogPath):
                    // the files move with the folder
                    if action_button(
                        ui,
                        p,
                        "folder-filled",
                        aurora_i18n::tr!("ui-options-change-folder"),
                        ActionKind::Flat,
                        size,
                    )
                    .on_hover_text(aurora_i18n::tr!("ui-options-chat-logs-folder-help"))
                    .clicked()
                    {
                        r.folder = Some((Folder::ChatLogs, FolderChoice::Pick));
                    }
                    if st.chat_log_custom
                        && action_button(
                            ui,
                            p,
                            "restore",
                            aurora_i18n::tr!("ui-options-default-folder"),
                            ActionKind::Flat,
                            size,
                        )
                        .clicked()
                    {
                        r.folder = Some((Folder::ChatLogs, FolderChoice::Default));
                    }
                    if action_button(
                        ui,
                        p,
                        "trash-filled",
                        aurora_i18n::tr!("ui-options-chat-logs-erase"),
                        ActionKind::FlatDanger,
                        size,
                    )
                    .clicked()
                    {
                        st.confirm_erase_logs = true;
                    }
                });
                if !st.chat_log_status.is_empty() {
                    ui.label(RichText::new(&st.chat_log_status).size(11.5).color(p.muted));
                }
            }
            None => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-options-chat-logs-no-account"))
                        .size(11.5)
                        .color(p.muted_dim),
                );
            }
        }
    });
    c
}

/// First group of the Chat page; `center`: Aurora's notification center
/// is chosen (Preferences › Aurora), where the friends' toasts have a place.
fn chat_page_head(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, center: bool, scroll_to_attention: bool) -> bool {
    let mut c = false;
    group(ui, p, aurora_i18n::tr!("common-nearby-chat"), |ui| {
        c |= row(ui, p, aurora_i18n::tr!("ui-options-show-time"), "", |ui| {
            toggle(ui, p, &mut s.chat_timestamps)
        });
        // FSShowChatChannel: the "/0" chip of the field (off, as in the
        // Legacy Viewer)
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-show-chat-channel"),
            aurora_i18n::tr!("ui-options-show-chat-channel-hint"),
            |ui| toggle(ui, p, &mut s.show_chat_channel),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-chat-keep-focus"),
            aurora_i18n::tr!("ui-options-chat-keep-focus-hint"),
            |ui| toggle(ui, p, &mut s.chat_keep_focus),
        );
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
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-show-contacts-tab"),
            aurora_i18n::tr!("ui-options-show-contacts-tab-hint"),
            |ui| toggle(ui, p, &mut s.show_contacts_tab),
        );
        // FSSendTypingState: the IM_TYPING notices of a conversation with
        // one resident (LLIMModel::sendTypingState)
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-send-typing"),
            aurora_i18n::tr!("ui-options-send-typing-hint"),
            |ui| toggle(ui, p, &mut s.send_typing_state),
        );
        // PlayTypingAnim and FSTypeDuringEmote, the second greyed while
        // the first is off (panel_preferences_chat.xml)
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-play-typing-anim"),
            aurora_i18n::tr!("ui-options-play-typing-anim-hint"),
            |ui| toggle(ui, p, &mut s.play_typing_anim),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-type-during-emote"),
            aurora_i18n::tr!("ui-options-type-during-emote-hint"),
            |ui| {
                ui.add_enabled_ui(s.play_typing_anim, |ui| toggle(ui, p, &mut s.type_during_emote))
                    .inner
            },
        );
        // FSShowTypingStateInNameTag and FSTypingChevronPrefix
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-typing-in-name-tags"),
            aurora_i18n::tr!("ui-options-typing-in-name-tags-hint"),
            |ui| toggle(ui, p, &mut s.typing_in_name_tags),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-typing-chevron"),
            aurora_i18n::tr!("ui-options-typing-chevron-hint"),
            |ui| toggle(ui, p, &mut s.typing_chevron),
        );
        // the tab of a conversation with news flashes
        // (FSFloaterIMContainer::startFlashingTab), in violet and softly
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-flash-unread"),
            aurora_i18n::tr!("ui-options-flash-unread-hint"),
            |ui| toggle(ui, p, &mut s.flash_unread_conversations),
        );
        // the Legacy Viewer's "Flash chat toolbar button if a new IM
        // arrives" (FSNotifyIMFlash)
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-flash-conversations-button"),
            aurora_i18n::tr!("ui-options-flash-conversations-button-hint"),
            |ui| toggle(ui, p, &mut s.flash_conversations_button),
        );
    });
    // Firestorm's Chat › Notifications (EnableIMChatPopups,
    // EnableGroupChatPopups) and ChatOnlineNotification
    group(ui, p, aurora_i18n::tr!("ui-options-toasts"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-toast-friends-online"),
            aurora_i18n::tr!("ui-options-toast-friends-online-hint"),
            |ui| toggle(ui, p, &mut s.toast_friends_online),
        );
        // OnlineOfflinetoNearbyChat, beside its toast as in the Legacy
        // Viewer's "Notify me when my friends log in or out"
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-friends-online-in-chat"),
            aurora_i18n::tr!("ui-options-friends-online-in-chat-hint"),
            |ui| toggle(ui, p, &mut s.friends_online_in_chat),
        );
        // where they stand with Aurora's center; Firestorm's toasts keep
        // them in their own stack (top right, like its notification channel)
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-toast-friends-position"),
            if center {
                aurora_i18n::tr!("ui-options-toast-friends-position-hint")
            } else {
                aurora_i18n::tr!("ui-options-toast-friends-position-legacy")
            },
            |ui| {
                let mut changed = false;
                ui.add_enabled_ui(center && s.toast_friends_online, |ui| {
                    egui::ComboBox::from_id_salt("presence-position")
                        .selected_text(s.presence_position.label())
                        .show_ui(ui, |ui| {
                            for at in crate::settings::PresencePosition::ALL {
                                changed |= ui.selectable_value(&mut s.presence_position, at, at.label()).changed();
                            }
                        });
                });
                changed
            },
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-toast-im"),
            aurora_i18n::tr!("ui-options-toast-im-hint"),
            |ui| toggle(ui, p, &mut s.toast_im),
        );
        c |= row(ui, p, aurora_i18n::tr!("ui-options-toast-group-im"), "", |ui| {
            toggle(ui, p, &mut s.toast_group_im)
        });
        // only what toasts can stay under the bell
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-ims-in-notification-center"),
            aurora_i18n::tr!("ui-options-ims-in-notification-center-hint"),
            |ui| {
                ui.add_enabled_ui(s.toast_im || s.toast_group_im, |ui| {
                    toggle(ui, p, &mut s.ims_in_notification_center)
                })
                .inner
            },
        );
        c |= row(ui, p, aurora_i18n::tr!("ui-options-toast-duration"), "", |ui| {
            slider(ui, &mut s.im_toast_seconds, 3.0..=60.0, " s")
        });
    });
    // the Legacy Viewer's Notifications › People / Objects & Scripts
    // (FSFlashOnMessage, FSFlashOnObjectIM, FSFlashOnScriptDialog) and
    // World (FSEnableGrowl, FSGrowlWhenActive)
    if scroll_to_attention {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    group(ui, p, aurora_i18n::tr!("ui-options-attention"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-flash-on-message"),
            aurora_i18n::tr!("ui-options-flash-on-message-hint"),
            |ui| toggle(ui, p, &mut s.flash_on_message),
        );
        c |= row(ui, p, aurora_i18n::tr!("ui-options-flash-on-object-im"), "", |ui| {
            ui.add_enabled_ui(s.flash_on_message, |ui| toggle(ui, p, &mut s.flash_on_object_im))
                .inner
        });
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-flash-on-script-dialog"),
            aurora_i18n::tr!("ui-options-flash-on-script-dialog-hint"),
            |ui| toggle(ui, p, &mut s.flash_on_script_dialog),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-system-notifications"),
            aurora_i18n::tr!("ui-options-system-notifications-hint"),
            |ui| toggle(ui, p, &mut s.system_notifications),
        );
        c |= row(ui, p, aurora_i18n::tr!("ui-options-system-notifications-when-active"), "", |ui| {
            ui.add_enabled_ui(s.system_notifications, |ui| toggle(ui, p, &mut s.system_notifications_when_active))
                .inner
        });
    });
    c
}

/// Chat › Keyword alerts (panel_preferences_chat.xml, "ChatKeywordAlerts"):
/// the rows greyed while the alerts are off, the sound's asset while its
/// switch is off, as the Legacy panel's enabled_control.
fn keyword_alerts_group(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut c = false;
    if st.scroll_to_keywords {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
        st.scroll_to_keywords = false;
    }
    group(ui, p, aurora_i18n::tr!("ui-options-keywords"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-keywords-on"),
            aurora_i18n::tr!("ui-options-keywords-on-hint"),
            |ui| toggle(ui, p, &mut s.keywords.on),
        );
        let on = s.keywords.on;
        let switches = [
            (aurora_i18n::tr!("ui-options-keywords-in-chat"), "", &mut s.keywords.in_chat),
            (aurora_i18n::tr!("ui-options-keywords-in-im"), "", &mut s.keywords.in_im),
            (
                aurora_i18n::tr!("ui-options-keywords-speakers-name"),
                aurora_i18n::tr!("ui-options-keywords-speakers-name-hint"),
                &mut s.keywords.speakers_name,
            ),
            (
                aurora_i18n::tr!("ui-options-keywords-case-sensitive"),
                "",
                &mut s.keywords.case_sensitive,
            ),
            (
                aurora_i18n::tr!("ui-options-keywords-whole-words"),
                aurora_i18n::tr!("ui-options-keywords-whole-words-hint"),
                &mut s.keywords.whole_words,
            ),
        ];
        for (label, hint, value) in switches {
            c |= row(ui, p, label, hint, |ui| ui.add_enabled_ui(on, |ui| toggle(ui, p, value)).inner);
        }
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-keywords-words"),
            aurora_i18n::tr!("ui-options-keywords-words-hint"),
            |ui| {
                ui.add_enabled(
                    on,
                    egui::TextEdit::singleline(&mut s.keywords.words)
                        .desired_width(290.0)
                        .hint_text(aurora_i18n::tr!("ui-options-keywords-words-example")),
                )
                .changed()
            },
        );
        c |= row(ui, p, aurora_i18n::tr!("ui-options-keywords-color"), "", |ui| {
            ui.add_enabled_ui(on, |ui| {
                let mut ch = toggle(ui, p, &mut s.keywords.change_color);
                ui.add_enabled_ui(s.keywords.change_color, |ui| {
                    ch |= swatch(ui, p, "keyword-color", &mut s.keywords.color);
                });
                ch
            })
            .inner
        });
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-keywords-sound"),
            aurora_i18n::tr!("ui-options-keywords-sound-hint"),
            |ui| {
                ui.add_enabled_ui(on, |ui| {
                    let k = &mut s.keywords;
                    let mut ch = toggle(ui, p, &mut k.play_sound);
                    ui.add_enabled_ui(k.play_sound, |ui| {
                        // typed for this asset; a reset of the page starts it again
                        let (typed_for, text) = st.keyword_sound_text.get_or_insert_with(|| (k.sound, uuid_text(k.sound)));
                        if *typed_for != k.sound {
                            (*typed_for, *text) = (k.sound, uuid_text(k.sound));
                        }
                        let valid = parse_uuid_text(text).is_some();
                        let edit = ui.add(
                            egui::TextEdit::singleline(text)
                                .desired_width(290.0)
                                .font(egui::TextStyle::Monospace)
                                .text_color(if valid { p.ink } else { p.danger })
                                .hint_text(aurora_i18n::tr!("ui-options-no-sound")),
                        );
                        if edit.changed()
                            && let Some(id) = parse_uuid_text(text)
                        {
                            (k.sound, *typed_for) = (id, id);
                            ch = true;
                        }
                        // force_sound: heard even with the interface sounds off
                        if super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-listen")).clicked() && !k.sound.is_nil() {
                            super::sound_cues::preview(ui.ctx(), k.sound);
                        }
                        let default = crate::keywords::DEFAULT_SOUND;
                        if k.sound != default && super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-default")).clicked() {
                            (k.sound, *typed_for, *text) = (default, default, uuid_text(default));
                            ch = true;
                        }
                    });
                    ch
                })
                .inner
            },
        );
    });
    c
}

fn content(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult) -> bool {
    let mut c = false;
    let (mut font, mut cache) = (false, false);
    let (font_changed, clear_cache) = (&mut font, &mut cache);
    if st.tab == TAB_CHAT {
        // drawn after the first group of the page, below
        let center = s.notifications_style == crate::settings::NotificationsStyle::Aurora;
        c |= chat_page_head(ui, p, s, center, std::mem::take(&mut st.scroll_to_attention));
        c |= keyword_alerts_group(ui, p, s, st);
        c |= chat_logs_group(ui, p, s, st, r);
    }
    match st.tab {
        0 => {
            group(ui, p, aurora_i18n::tr!("ui-options-presets"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-preset-level"),
                    aurora_i18n::tr!("ui-options-preset-level-help"),
                    |ui| preset_choice(ui, p, s),
                );
            });
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
                    aurora_i18n::tr!("ui-options-avatar-physics"),
                    aurora_i18n::tr!("ui-options-avatar-physics-help"),
                    |ui| toggle(ui, p, &mut s.avatar_physics),
                );
                if s.avatar_physics {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-avatar-physics-detail"),
                        aurora_i18n::tr!("ui-options-avatar-physics-detail-help"),
                        |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W;
                            ui.add(
                                egui::Slider::new(&mut s.avatar_physics_detail, 0.0..=1.0)
                                    .custom_parser(aurora_i18n::parse_number)
                                    .step_by(0.05)
                                    .fixed_decimals(2)
                                    .trailing_fill(true),
                            )
                            .changed()
                        },
                    );
                }
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
                    aurora_i18n::tr!("ui-options-projector-shadows"),
                    aurora_i18n::tr!("ui-options-projector-shadows-help"),
                    |ui| {
                        ui.add_enabled_ui(s.shadow_quality > 0, |ui| toggle(ui, p, &mut s.projector_shadows))
                            .inner
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-contact-shadows"),
                    aurora_i18n::tr!("ui-options-contact-shadows-help"),
                    |ui| {
                        ui.add_enabled_ui(s.shadow_quality > 0, |ui| toggle(ui, p, &mut s.contact_shadows))
                            .inner
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
                    aurora_i18n::tr!("ui-options-ao-method"),
                    aurora_i18n::tr!("ui-options-ao-method-help"),
                    |ui| {
                        let names = [aurora_i18n::tr!("ui-options-ao-legacy"), "GTAO"];
                        let mut changed = false;
                        ui.add_enabled_ui(s.ssao > 0, |ui| {
                            egui::ComboBox::from_id_salt("ao_method")
                                .width(SLIDER_W)
                                .selected_text(names[s.ao_method.min(1) as usize])
                                .show_ui(ui, |ui| {
                                    changed |= ui.selectable_value(&mut s.ao_method, 0, names[0]).changed();
                                    changed |= ui.selectable_value(&mut s.ao_method, 1, names[1]).changed();
                                });
                        });
                        changed
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-gtao-quality"),
                    aurora_i18n::tr!("ui-options-gtao-quality-help"),
                    |ui| {
                        ui.add_enabled_ui(s.ssao > 0 && s.ao_method == 1, |ui| {
                            stepped(
                                ui,
                                p,
                                &mut s.gtao_quality,
                                &[
                                    aurora_i18n::tr!("ui-options-low"),
                                    aurora_i18n::tr!("common-medium"),
                                    aurora_i18n::tr!("ui-options-high"),
                                    aurora_i18n::tr!("ui-options-ultra"),
                                ],
                            )
                        })
                        .inner
                    },
                );
                c |= row_experimental(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-screen-indirect-light"),
                    aurora_i18n::tr!("ui-options-screen-indirect-light-help"),
                    |ui| {
                        if std::mem::take(&mut st.scroll_to_occlusion) {
                            ui.scroll_to_cursor(Some(egui::Align::Center));
                        }
                        ui.add_enabled_ui(s.ssao > 0 && s.ao_method == 1, |ui| toggle(ui, p, &mut s.screen_indirect_light))
                            .inner
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-screen-indirect-intensity"),
                    aurora_i18n::tr!("ui-options-screen-indirect-intensity-help"),
                    |ui| {
                        let on = s.ssao > 0 && s.ao_method == 1 && s.screen_indirect_light;
                        ui.add_enabled_ui(on, |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W;
                            ui.add(
                                egui::Slider::new(&mut s.screen_indirect_intensity, 0.0..=4.0)
                                    .custom_parser(aurora_i18n::parse_number)
                                    .step_by(0.1)
                                    .trailing_fill(true),
                            )
                            .changed()
                        })
                        .inner
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
                            .selected_text(names[s.time_of_day.min(crate::world::env::TIME_OF_DAY_PRESETS) as usize])
                            .show_ui(ui, |ui| {
                                for i in std::iter::once(0).chain(crate::world::env::TIME_OF_DAY_ORDER) {
                                    changed |= ui.selectable_value(&mut s.time_of_day, i, names[i as usize]).changed();
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
                c |= row_experimental(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-bloom"),
                    aurora_i18n::tr!("ui-options-bloom-help"),
                    |ui| {
                        if std::mem::take(&mut st.scroll_to_bloom) {
                            ui.scroll_to_cursor(Some(egui::Align::Center));
                        }
                        toggle(ui, p, &mut s.bloom)
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-bloom-strength"),
                    aurora_i18n::tr!("ui-options-bloom-strength-help"),
                    |ui| {
                        ui.add_enabled_ui(s.bloom, |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W;
                            ui.add(
                                egui::Slider::new(&mut s.bloom_strength, 0.05..=1.0)
                                    .custom_parser(aurora_i18n::parse_number)
                                    .step_by(0.05)
                                    .trailing_fill(true),
                            )
                            .changed()
                        })
                        .inner
                    },
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
                    aurora_i18n::tr!("ui-options-specular-aa"),
                    aurora_i18n::tr!("ui-options-specular-aa-help"),
                    |ui| toggle(ui, p, &mut s.specular_aa),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-tone-mapping"),
                    aurora_i18n::tr!("ui-options-tone-mapping-help"),
                    |ui| {
                        let names = [aurora_i18n::tr!("ui-options-khronos-pbr-neutral"), "ACES"];
                        let mut changed = false;
                        if std::mem::take(&mut st.scroll_to_tone_mapping) {
                            ui.scroll_to_cursor(Some(egui::Align::Center));
                        }
                        // a classic sky is not tone mapped (no_post): the
                        // choice would change nothing
                        ui.add_enabled_ui(st.sky_tone_mapped, |ui| {
                            egui::ComboBox::from_id_salt("tonemap")
                                .width(SLIDER_W)
                                .selected_text(names[s.tonemapper.min(1) as usize])
                                .show_ui(ui, |ui| {
                                    changed |= ui.selectable_value(&mut s.tonemapper, 0, names[0]).changed();
                                    changed |= ui.selectable_value(&mut s.tonemapper, 1, names[1]).changed();
                                });
                        })
                        .response
                        .on_disabled_hover_text(aurora_i18n::tr!("ui-options-tone-mapping-classic-sky"));
                        changed
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-tone-mapping-mix"),
                    aurora_i18n::tr!("ui-options-tone-mapping-mix-help"),
                    |ui| {
                        let r = ui.add_enabled_ui(st.sky_tone_mapped, |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W;
                            ui.add(
                                egui::Slider::new(&mut s.tonemap_mix, 0.0..=1.0)
                                    .custom_parser(aurora_i18n::parse_number)
                                    .step_by(0.1)
                                    .fixed_decimals(1)
                                    .trailing_fill(true),
                            )
                            .changed()
                        });
                        r.response
                            .on_disabled_hover_text(aurora_i18n::tr!("ui-options-tone-mapping-classic-sky"));
                        r.inner
                    },
                );
            });
            c |= dof::show(ui, p, &mut s.dof);
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
            if let Some(access) = st.maturity {
                group(ui, p, aurora_i18n::tr!("ui-options-content"), |ui| {
                    row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-content-rated"),
                        aurora_i18n::tr!("ui-options-content-rated-help"),
                        |ui| {
                            if let Some(m) = maturity_choice(ui, p, access) {
                                r.maturity = Some(m);
                            }
                            false
                        },
                    );
                });
            }
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
                // the place island of the Aurora bars (also in its right-click menu)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-place-bar-stretch"),
                    aurora_i18n::tr!("ui-options-place-bar-stretch-help"),
                    |ui| toggle(ui, p, &mut s.place_bar_stretch),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-place-bar-rights-text"),
                    aurora_i18n::tr!("ui-options-place-bar-rights-text-help"),
                    |ui| toggle(ui, p, &mut s.place_bar_rights_text),
                );
                // also in the favourites' own menus
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-favorites-access"),
                    aurora_i18n::tr!("ui-options-favorites-access-help"),
                    |ui| {
                        let mut changed = false;
                        ui.horizontal_wrapped(|ui| {
                            for (value, label) in [
                                (crate::settings::FavoritesAccess::Menu, aurora_i18n::tr!("favorites-access-menu")),
                                (crate::settings::FavoritesAccess::Row, aurora_i18n::tr!("favorites-access-row")),
                                (crate::settings::FavoritesAccess::Dock, aurora_i18n::tr!("favorites-access-dock")),
                            ] {
                                changed |= super::widgets::radio_value(ui, p, &mut s.favorites_access, value, label).changed();
                            }
                        });
                        changed
                    },
                );
                // also in the system island's right click
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-traffic-indicator"),
                    aurora_i18n::tr!("ui-options-traffic-indicator-help"),
                    |ui| toggle(ui, p, &mut s.show_net_stats),
                );
                r.ui_scale_changed |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-interface-scale"),
                    aurora_i18n::tr!("ui-options-interface-scale-help"),
                    |ui| st.interface_scale.show(ui, &mut s.ui_scale),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-snap-windows"),
                    aurora_i18n::tr!("ui-options-snap-windows-help"),
                    |ui| toggle(ui, p, &mut s.snap_windows),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-windows-stop-at-edges"),
                    aurora_i18n::tr!("ui-options-windows-stop-at-edges-help"),
                    |ui| {
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            changed |= super::widgets::checkbox(ui, p, &mut s.windows_stop_left, aurora_i18n::tr!("ui-options-edge-left"))
                                .changed();
                            changed |=
                                super::widgets::checkbox(ui, p, &mut s.windows_stop_right, aurora_i18n::tr!("ui-options-edge-right"))
                                    .changed();
                            changed |=
                                super::widgets::checkbox(ui, p, &mut s.windows_stop_bottom, aurora_i18n::tr!("ui-options-edge-bottom"))
                                    .changed();
                        });
                        changed
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-context-menu"),
                    aurora_i18n::tr!("ui-options-context-menu-help"),
                    |ui| {
                        let mut changed = false;
                        let label = if s.use_pie_menu {
                            aurora_i18n::tr!("ui-options-pie-menu")
                        } else {
                            aurora_i18n::tr!("ui-options-list-menu")
                        };
                        egui::ComboBox::from_id_salt("context-menu-style")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(&mut s.use_pie_menu, false, aurora_i18n::tr!("ui-options-list-menu"))
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut s.use_pie_menu, true, aurora_i18n::tr!("ui-options-pie-menu"))
                                    .changed();
                            });
                        changed
                    },
                );
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
                    aurora_i18n::tr!("ui-options-reduce-motion"),
                    aurora_i18n::tr!("ui-options-reduce-motion-help"),
                    |ui| toggle(ui, p, &mut s.reduce_motion),
                );
                c |= row(ui, p, aurora_i18n::tr!("ui-options-performance-at-startup"), "", |ui| {
                    toggle(ui, p, &mut s.show_perf)
                });
                c |= row(ui, p, aurora_i18n::tr!("ui-options-mini-map-at-startup"), "", |ui| {
                    toggle(ui, p, &mut s.show_minimap)
                });
            });
            // The name tag style and the plates' settings: Preferences › Aurora.
            if std::mem::take(&mut st.scroll_to_name_tags) {
                ui.scroll_to_cursor(Some(egui::Align::TOP));
            }
            group(ui, p, aurora_i18n::tr!("ui-options-name-tags"), |ui| {
                c |= row(ui, p, aurora_i18n::tr!("ui-options-show-own-tag"), "", |ui| {
                    toggle(ui, p, &mut s.show_own_name_tag)
                });
                c |= row(ui, p, aurora_i18n::tr!("ui-options-show-friend-tags"), "", |ui| {
                    toggle(ui, p, &mut s.show_friend_name_tags)
                });
                c |= row(ui, p, aurora_i18n::tr!("ui-options-show-other-tags"), "", |ui| {
                    toggle(ui, p, &mut s.show_other_name_tags)
                });
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-temporary-tags"),
                    aurora_i18n::tr!("ui-options-temporary-tags-help"),
                    |ui| toggle(ui, p, &mut s.temporary_name_tags),
                );
                if s.temporary_name_tags {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-tag-show-time"),
                        aurora_i18n::tr!("ui-options-tag-show-time-help"),
                        |ui| slider_whole(ui, &mut s.name_tag_show_seconds, 1.0..=60.0, " s"),
                    );
                }
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
            });
            // panel_preferences_UI.xml "Use stand-alone windows for:"
            group(ui, p, aurora_i18n::tr!("common-windows"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-landmarks-and-place-profiles"),
                    aurora_i18n::tr!("ui-options-separate-windows-for-landmarks-teleport-history-details-place-profiles"),
                    |ui| toggle(ui, p, &mut s.standalone_place_details),
                );
            });
            // the "ignore" box of TeleportFromLandmark, TeleportToHistoryEntry
            // and TeleportViaSLAPP (Firestorm: Preferences › Notifications ›
            // Alerts, one line each)
            group(ui, p, aurora_i18n::tr!("common-teleport-2"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-confirm-teleports"),
                    aurora_i18n::tr!("ui-options-ask-before-teleporting-to-a-landmark-a-history-entry"),
                    |ui| toggle(ui, p, &mut s.confirm_teleports),
                );
            });
            // the "ignore" box of SettingsMakeNoTrans (Firestorm: Preferences ›
            // Notifications › Alerts)
            group(ui, p, aurora_i18n::tr!("common-environment"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-confirm-no-transfer-settings"),
                    aurora_i18n::tr!("ui-options-confirm-no-transfer-settings-help"),
                    |ui| toggle(ui, p, &mut s.confirm_no_transfer_settings),
                );
            });
            // the "ignore" boxes of ReturnToOwner, AttachmentDrop and
            // ConfirmScriptModify (the object menus)
            group(ui, p, aurora_i18n::tr!("ui-options-objects"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-confirm-return-to-owner"),
                    aurora_i18n::tr!("ui-options-confirm-return-to-owner-help"),
                    |ui| toggle(ui, p, &mut s.confirm_return_to_owner),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-confirm-attachment-drop"),
                    aurora_i18n::tr!("ui-options-confirm-attachment-drop-help"),
                    |ui| toggle(ui, p, &mut s.confirm_attachment_drop),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-confirm-script-modify"),
                    aurora_i18n::tr!("ui-options-confirm-script-modify-help"),
                    |ui| toggle(ui, p, &mut s.confirm_script_modify),
                );
            });
            // the "ignore" box of ChangeObjectBonusFactor and the "Don't ask"
            // of ConfirmTextureHeights (RegionCheckTextureHeights)
            group(ui, p, aurora_i18n::tr!("region-title"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-confirm-object-bonus"),
                    aurora_i18n::tr!("ui-options-confirm-object-bonus-help"),
                    |ui| toggle(ui, p, &mut s.confirm_object_bonus),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-check-texture-heights"),
                    aurora_i18n::tr!("ui-options-check-texture-heights-help"),
                    |ui| toggle(ui, p, &mut s.region_check_texture_heights),
                );
                // Legacy Viewer › Alerts: FSShowServerVersionChangeNotice,
                // FSReportTotalScriptCountChanges and its threshold
                scroll_mark(ui, st);
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-sim-version-alert"),
                    aurora_i18n::tr!("ui-options-sim-version-alert-help"),
                    |ui| toggle(ui, p, &mut s.sim_version_alert),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-script-count-alert"),
                    aurora_i18n::tr!("ui-options-script-count-alert-help"),
                    |ui| {
                        let mut changed = toggle(ui, p, &mut s.script_count_alert);
                        ui.add_enabled_ui(s.script_count_alert, |ui| {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut s.script_count_threshold)
                                        .custom_parser(aurora_i18n::parse_number)
                                        .range(0..=10000)
                                        .speed(10.0),
                                )
                                .on_hover_text(aurora_i18n::tr!("ui-options-script-count-threshold"))
                                .changed();
                        });
                        changed
                    },
                );
            });
            // FSUseNewRegionRestartNotification, FSNoScreenShakeOnRegionRestart,
            // FSReportRegionRestartToChat / FSRegionRestartAnnounceChannel
            // (Firestorm: Preferences › Interface › Toasts)
            group(ui, p, aurora_i18n::tr!("ui-options-region-restart"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-region-restart-alert"),
                    aurora_i18n::tr!("ui-options-region-restart-alert-help"),
                    |ui| toggle(ui, p, &mut s.region_restart.show_alert),
                );
                ui.add_enabled_ui(s.region_restart.show_alert, |ui| {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-region-restart-shake"),
                        aurora_i18n::tr!("ui-options-region-restart-shake-help"),
                        |ui| {
                            use crate::world::region_restart::Shake;
                            let options = [
                                (aurora_i18n::tr!("ui-options-region-restart-shake-alert"), false),
                                (aurora_i18n::tr!("ui-options-region-restart-shake-view"), false),
                                (aurora_i18n::tr!("ui-options-region-restart-shake-none"), false),
                            ];
                            let mut i = match s.region_restart.shake {
                                Shake::Alert => 0,
                                Shake::View => 1,
                                Shake::None => 2,
                            };
                            let changed = super::widgets::segmented(ui, p, &mut i, &options);
                            s.region_restart.shake = [Shake::Alert, Shake::View, Shake::None][i.min(2)];
                            changed
                        },
                    );
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-region-restart-announce"),
                        aurora_i18n::tr!("ui-options-region-restart-announce-help"),
                        |ui| {
                            let mut changed = toggle(ui, p, &mut s.region_restart.announce);
                            ui.add_enabled_ui(s.region_restart.announce, |ui| {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut s.region_restart.channel)
                                            .custom_parser(aurora_i18n::parse_number)
                                            .range(i32::MIN..=i32::MAX)
                                            .speed(1.0),
                                    )
                                    .on_hover_text(aurora_i18n::tr!("ui-options-region-restart-channel"))
                                    .changed();
                            });
                            changed
                        },
                    );
                });
            });
            // FSCollisionMessagesInChat, FSReportCollisionMessages and
            // FSReportCollisionMessagesChannel (Legacy Viewer: Preferences ›
            // Notifications › Alerts)
            scroll_mark(ui, st);
            group(ui, p, aurora_i18n::tr!("bumps-title"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-collisions-in-chat"),
                    aurora_i18n::tr!("ui-options-collisions-in-chat-help"),
                    |ui| toggle(ui, p, &mut s.collisions.in_chat),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-collisions-report"),
                    aurora_i18n::tr!("ui-options-collisions-report-help"),
                    |ui| {
                        let mut changed = toggle(ui, p, &mut s.collisions.report);
                        ui.add_enabled_ui(s.collisions.report, |ui| {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut s.collisions.channel)
                                        .custom_parser(aurora_i18n::parse_number)
                                        .range(i32::MIN..=i32::MAX)
                                        .speed(1.0),
                                )
                                .on_hover_text(aurora_i18n::tr!("ui-options-collisions-channel"))
                                .changed();
                        });
                        changed
                    },
                );
            });
            // FSConfirmPayments / FSPaymentConfirmationThreshold
            // (panel_preferences_firestorm.xml)
            group(ui, p, aurora_i18n::tr!("ui-options-payments"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-confirm-payments"),
                    aurora_i18n::tr!("ui-options-confirm-payments-help"),
                    |ui| toggle(ui, p, &mut s.confirm_payments),
                );
                ui.add_enabled_ui(s.confirm_payments, |ui| {
                    c |= row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-options-payment-threshold"),
                        aurora_i18n::tr!("ui-options-payment-threshold-help"),
                        |ui| {
                            ui.add(
                                egui::DragValue::new(&mut s.payment_confirm_threshold)
                                    .custom_parser(aurora_i18n::parse_number)
                                    .range(0..=i32::MAX)
                                    .speed(1.0)
                                    .suffix(" L$"),
                            )
                            .changed()
                        },
                    );
                });
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
                // Legacy Viewer › General: GroupTitlesTagMode, RenderHideGroupTitle
                scroll_mark(ui, st);
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-group-titles"),
                    aurora_i18n::tr!("ui-options-group-titles-help"),
                    |ui| {
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("group-titles")
                            .selected_text(s.group_titles.label())
                            .show_ui(ui, |ui| {
                                for mode in crate::world::names::GroupTitles::ALL {
                                    changed |= ui.selectable_value(&mut s.group_titles, mode, mode.label()).changed();
                                }
                            });
                        changed
                    },
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-hide-own-group-title"),
                    aurora_i18n::tr!("ui-options-hide-own-group-title-help"),
                    |ui| toggle(ui, p, &mut s.hide_own_group_title),
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
                for builtin in fonts::BUILTINS.iter().map(|b| b.name) {
                    let sel = s.font == builtin || (s.font.is_empty() && builtin == fonts::DEFAULT);
                    let label = if builtin == fonts::DEFAULT {
                        aurora_i18n::tr!("ui-options-default", builtin = builtin.to_string())
                    } else {
                        builtin.to_owned()
                    };
                    if super::widgets::radio(ui, p, sel, RichText::new(label).size(13.0)).clicked() && !sel {
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
                        if super::widgets::radio(ui, p, sel, RichText::new(&f.name).size(13.0)).clicked() && !sel {
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
                // Legacy Viewer › Chat (FSIgnoreAdHocSessions, FSReportIgnoredAdHocSession,
                // FSDontIgnoreAdHocFromFriends): the same switches as Online status ›
                // Ad hoc conferences
                scroll_mark(ui, st);
                let a = &mut s.autoresponse;
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-ignore-conferences"),
                    aurora_i18n::tr!("ui-options-ignore-conferences-hint"),
                    |ui| toggle(ui, p, &mut a.ignore_adhoc),
                );
                let on = a.ignore_adhoc;
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-ignore-conferences-report"),
                    aurora_i18n::tr!("ui-options-ignore-conferences-report-hint"),
                    |ui| ui.add_enabled_ui(on, |ui| toggle(ui, p, &mut a.report_ignored_adhoc)).inner,
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-ignore-conferences-friends"),
                    aurora_i18n::tr!("ui-options-ignore-conferences-friends-hint"),
                    |ui| ui.add_enabled_ui(on, |ui| toggle(ui, p, &mut a.adhoc_from_friends)).inner,
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-report-in-chat"),
                    aurora_i18n::tr!("ui-options-announce-additions-to-and-removals-from-the-block-list"),
                    |ui| toggle(ui, p, &mut s.report_blocks),
                );
                // Legacy Viewer › Privacy (FSShowJoinedGroupInvitations)
                scroll_mark(ui, st);
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-joined-group-invitations"),
                    aurora_i18n::tr!("ui-options-joined-group-invitations-help"),
                    |ui| toggle(ui, p, &mut s.show_joined_group_invitations),
                );
            });
            // Firestorm: Preferences > Privacy > Autoresponse 1 and 2
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
                // Legacy Viewer › General (AvatarSitOnAway, QuitAfterSecondsOfAFK)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-sit-when-away"),
                    aurora_i18n::tr!("ui-options-sit-when-away-help"),
                    |ui| toggle(ui, p, &mut a.sit_when_away),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-quit-after-away"),
                    aurora_i18n::tr!("ui-options-quit-after-away-help"),
                    |ui| slider(ui, &mut a.quit_after_away_minutes, 0..=60, " min"),
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
                // Firestorm › Move & View › Movement: "Double-click on land"
                // (double_click_action_combo, panel_preferences_move.xml)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-double-click-ground"),
                    aurora_i18n::tr!("ui-options-double-click-ground-help"),
                    |ui| {
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("double-click-ground")
                            .selected_text(s.double_click.label())
                            .show_ui(ui, |ui| {
                                for action in crate::autopilot::DoubleClick::ALL {
                                    changed |= ui.selectable_value(&mut s.double_click, action, action.label()).changed();
                                }
                            });
                        changed
                    },
                );
                // Legacy Viewer › Protection (FSBlockClickSit)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-block-click-sit"),
                    aurora_i18n::tr!("ui-options-block-click-sit-help"),
                    |ui| toggle(ui, p, &mut s.block_click_sit),
                );
            });
            group(ui, p, aurora_i18n::tr!("ui-options-keyboard"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-arrows-strafe"),
                    aurora_i18n::tr!("ui-options-left-right-strafe-instead-of-turning-shift-reverses-this"),
                    |ui| toggle(ui, p, &mut s.arrows_strafe),
                );
                // Firestorm › Move & View › Movement (panel_preferences_move.xml)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-arrow-keys-always-move"),
                    aurora_i18n::tr!("ui-options-arrow-keys-always-move-help"),
                    |ui| toggle(ui, p, &mut s.arrow_keys_always_move),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-letter-keys-move"),
                    aurora_i18n::tr!("ui-options-letter-keys-move-help"),
                    |ui| toggle(ui, p, &mut s.letter_keys_move),
                );
                // Legacy Viewer › Move & View › Movement
                // (FSDisableTurningAroundWhenWalkingBackwards)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-walk-backwards-without-turning"),
                    aurora_i18n::tr!("ui-options-walk-backwards-without-turning-help"),
                    |ui| toggle(ui, p, &mut s.walk_backwards_without_turning),
                );
                // Legacy Viewer › Move & View: FSCrouchToggle, FSFlyAfterTeleport
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-crouch-toggle"),
                    aurora_i18n::tr!("ui-options-crouch-toggle-help"),
                    |ui| toggle(ui, p, &mut s.crouch_toggle),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-fly-after-teleport"),
                    aurora_i18n::tr!("ui-options-fly-after-teleport-help"),
                    |ui| toggle(ui, p, &mut s.fly_after_teleport),
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
            // Firestorm › Avatar (panel_preferences_firestorm.xml): the
            // head follows the mouse; the look-at targets are on the
            // Privacy page, as its note says
            group(ui, p, aurora_i18n::tr!("ui-options-head-follows-mouse"), |ui| {
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-head-vertical-range"),
                    aurora_i18n::tr!("ui-options-head-vertical-range-help"),
                    |ui| slider_whole(ui, &mut s.lookat.head_pitch, crate::world::lookat::HEAD_RANGE, "°"),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-head-horizontal-range"),
                    aurora_i18n::tr!("ui-options-head-horizontal-range-help"),
                    |ui| slider_whole(ui, &mut s.lookat.head_yaw, crate::world::lookat::HEAD_RANGE, "°"),
                );
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-static-eyes"),
                    aurora_i18n::tr!("ui-options-static-eyes-help"),
                    |ui| toggle(ui, p, &mut s.lookat.static_eyes),
                );
                row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-look-at-targets"),
                    aurora_i18n::tr!("ui-options-look-at-targets-are-on-the-privacy-page"),
                    |ui| {
                        if super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-privacy")).clicked() {
                            st.tab = TAB_PRIVACY;
                        }
                        false
                    },
                );
            });
        }
        5 => {
            let classic_tags = tag_colors_used(s);
            let k = &mut s.colors;
            c |= tag_colors(ui, p, k, classic_tags);
            group(ui, p, aurora_i18n::tr!("ui-options-mini-map-markers"), |ui| {
                c |= swatches(
                    ui,
                    p,
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
                c |= super::widgets::switch_labeled(
                    ui,
                    p,
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
                                c |= super::widgets::checkbox(ui, p, on, "").changed();
                                c |= swatch(ui, p, ("map_range", label), col);
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
                    p,
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
                    p,
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
                    p,
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
            group(ui, p, aurora_i18n::tr!("ui-options-network"), |ui| {
                // (the step of Firestorm's slider, panel_preferences_setup.xml,
                // which stops at 3,000; ours goes up to what asks the most)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-maximum-bandwidth"),
                    aurora_i18n::tr!("ui-options-maximum-bandwidth-hint"),
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.bandwidth_kbps, 50..=aurora_net::throttle::DEFAULT_KBPS as u32)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(50.0)
                                .suffix(aurora_i18n::tr!("bandwidth-kbps-suffix"))
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                // Legacy Viewer › Move & View (FSDisableNeighbourRegionConnections)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-no-neighbour-regions"),
                    aurora_i18n::tr!("ui-options-no-neighbour-regions-help"),
                    |ui| toggle(ui, p, &mut s.no_neighbour_regions),
                );
            });
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
                        let text = format!("{} / {}", format_bytes(used), format_bytes(limit));
                        super::widgets::progress(ui, p, "cache_usage", frac, SLIDER_W, &text, None);
                        false
                    },
                );
                // Legacy Viewer › Network & Files: CacheLocation, applied at
                // the next start (cache_location.rs)
                c |= row(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-options-cache-folder"),
                    aurora_i18n::tr!("ui-options-cache-folder-help"),
                    |ui| {
                        use super::widgets::{ActionKind, action_button};
                        let mut changed = false;
                        ui.vertical(|ui| {
                            ui.set_max_width(SLIDER_W + 60.0);
                            let next = crate::cache_location::root_for(&s.cache_location);
                            path_label(ui, p, &crate::settings::cache_dir());
                            ui.horizontal(|ui| {
                                let size = Vec2::new(0.0, 24.0);
                                if action_button(
                                    ui,
                                    p,
                                    "folder-filled",
                                    aurora_i18n::tr!("ui-options-change-folder"),
                                    ActionKind::Flat,
                                    size,
                                )
                                .clicked()
                                {
                                    r.folder = Some((Folder::Cache, FolderChoice::Pick));
                                }
                                if !s.cache_location.is_empty()
                                    && action_button(
                                        ui,
                                        p,
                                        "restore",
                                        aurora_i18n::tr!("ui-options-default-folder"),
                                        ActionKind::Flat,
                                        size,
                                    )
                                    .clicked()
                                {
                                    s.cache_location.clear();
                                    changed = true;
                                }
                            });
                            if next.as_deref() != crate::cache_location::chosen() {
                                let shown = next.unwrap_or_else(crate::settings::default_cache_root);
                                ui.label(
                                    RichText::new(aurora_i18n::tr!(
                                        "ui-options-cache-folder-next-start",
                                        path = shown.display().to_string()
                                    ))
                                    .size(11.5)
                                    .color(p.muted),
                                );
                            }
                        });
                        changed
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
            });
        }
    }
    r.font_changed |= font;
    r.clear_cache |= cache;
    c
}

/// Device combo: "" = system default. Returns (a device was chosen, the
/// list is unfolded).
pub(super) fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut String, devices: &[String], width: f32) -> (bool, bool) {
    let mut changed = false;
    let shown = if value.is_empty() {
        aurora_i18n::tr!("ui-options-system-default").to_owned()
    } else {
        value.clone()
    };
    let list = egui::ComboBox::from_id_salt(id)
        .width(width)
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
    (changed, list.inner.is_some())
}

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
    let fill = if capturing { p.violet } else { p.raised };
    // a bound key is drawn as keycaps over an empty button of their width
    let caps = match &b {
        Some(b) if !capturing => keycap::caps(ui, b, fill),
        _ => Vec::new(),
    };
    let (text, col) = if capturing {
        (aurora_i18n::tr!("common-press-a-key"), p.on_accent)
    } else if b.is_some() {
        ("", p.ink)
    } else {
        ("—", p.muted_dim)
    };
    let caps_width = keycap::row_width(&caps);
    let resp = ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(col))
            .fill(fill)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .min_size(Vec2::new((caps_width + 12.0).max(132.0), 20.0)),
    );
    let caps_left = egui::pos2(resp.rect.center().x - caps_width / 2.0, resp.rect.center().y);
    keycap::paint_row(ui, p, &caps, caps_left, fill);
    if resp.clicked() {
        st.capture_modifier = None;
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

fn shortcut_search(ui: &mut egui::Ui, p: &Palette, st: &mut OptionsUi) -> egui::InnerResponse<bool> {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 28.0 - ui.spacing().item_spacing.x).max(80.0);
        let search = super::widgets::search_field(ui, &mut st.shortcut_filter, aurora_i18n::tr!("ui-options-search-shortcuts"), width);
        if search.clicked() || search.gained_focus() {
            st.take_capture();
        }
        let clear = super::widgets::icon_button(
            ui,
            p,
            "x",
            aurora_i18n::tr!("ui-options-clear-shortcut-search"),
            !st.shortcut_filter.is_empty(),
        );
        if clear {
            st.shortcut_filter.clear();
            st.take_capture();
            search.request_focus();
        }
        search.changed() || clear
    })
}

/// Firestorm's LLFloaterPreference::onUpdateFilterTerm folds case. This
/// local filter also searches the shortcut's section and both current bindings.
fn shortcut_matches(query: &str, section: &str, label: &str, kb: &crate::keybinds::KeyBindings, action: crate::keybinds::Action) -> bool {
    query.is_empty()
        || section.to_lowercase().contains(query)
        || label.to_lowercase().contains(query)
        || kb
            .get(action)
            .iter()
            .flatten()
            .any(|binding| binding.label().to_lowercase().contains(query))
}

/// "Shortcuts": every matching rebindable action, two bindings each.
fn keys_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut c = false;
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-options-click-a-shortcut-then-press-the-desired-key-or"))
            .size(12.0)
            .color(p.muted),
    );
    ui.add_space(6.0);
    let kb = s.keybinds.clone();
    let query = st.shortcut_filter.trim().to_lowercase();
    let mut any_matches = false;
    for (section, actions) in crate::keybinds::sections() {
        let actions: Vec<_> = actions
            .iter()
            .filter(|(a, label)| shortcut_matches(&query, section, label, &kb, *a))
            .collect();
        if actions.is_empty() {
            continue;
        }
        any_matches = true;
        ui.push_id(section, |ui| {
            group(ui, p, section, |ui| {
                if actions.iter().any(|(action, _)| *action == crate::keybinds::Action::HudDrag) {
                    ui.label(RichText::new(aurora_i18n::tr!("hud-local-scale-help")).size(12.0).color(p.muted));
                    ui.add_space(4.0);
                }
                for (a, label) in actions {
                    // Keep each binding's widget identity when earlier rows disappear.
                    ui.push_id(a, |ui| {
                        ui.horizontal(|ui| {
                            row_label(ui, p, label);
                            for slot in 0..2 {
                                binding_cell(ui, p, &kb, st, *a, slot);
                            }
                        })
                    });
                }
            })
        });
    }
    if !any_matches {
        ui.label(RichText::new(aurora_i18n::tr!("ui-options-no-matching-shortcuts")).color(p.muted));
        ui.add_space(6.0);
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

/// Preferences › Sound and voice › Interface sounds: each sound on / off, its
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
        let each = aurora_i18n::tr!("ui-options-per-sound-choices");
        super::widgets::collapsing(ui, p, "ui_sounds_each", each, false, |ui| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::{Action, Input};
    use winit::keyboard::KeyCode;

    #[test]
    fn preferences_keep_their_height_when_text_size_changes() {
        let theme = crate::theme::Theme::default();
        let p = theme.palette();
        for small in [false, true] {
            let ctx = egui::Context::default();
            fonts::apply(&ctx, fonts::INTER);
            theme.apply(&ctx, 1.0);
            let mut settings = Settings::default();
            let mut state = OptionsUi {
                tab: 2,
                system_fonts: Some(Vec::new()),
                ..Default::default()
            };
            let mut open = true;
            let mut frame = 0;
            let mut draw = |ctx: &egui::Context, state: &mut OptionsUi, settings: &mut Settings| {
                frame += 1;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |_| {
                        show(ctx, &p, &Icons::default(), settings, state, &mut open);
                    },
                );
                output.textures_delta.clear();
                ctx.memory(|memory| memory.area_rect(egui::Id::new("options")))
                    .expect("preferences")
            };
            for _ in 0..12 {
                draw(&ctx, &mut state, &mut settings);
            }
            if small {
                super::super::widgets::size_floater(&ctx, "options", Vec2::new(780.0, 400.0));
                for _ in 0..12 {
                    draw(&ctx, &mut state, &mut settings);
                }
            }
            let initial = draw(&ctx, &mut state, &mut settings);
            for tab in [2, 8, TAB_AURORA] {
                state.tab = tab;
                for scale in [1.0, 1.2, 1.4, 0.8, 1.0] {
                    settings.font_scale = scale;
                    theme.apply(&ctx, scale);
                    for _ in 0..24 {
                        let rect = draw(&ctx, &mut state, &mut settings);
                        assert!(
                            (rect.height() - initial.height()).abs() < 0.5,
                            "small={small} tab={tab} scale={scale}: {initial:?} -> {rect:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn interface_scale_uses_only_the_zoom_path_and_neighboring_settings_keep_the_general_path() {
        fn label_position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + egui::vec2(0.0, t.galley.size().y * 0.5)),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| label_position(s, label)),
                _ => None,
            }
        }
        fn frame(
            ctx: &egui::Context,
            st: &mut OptionsUi,
            s: &mut Settings,
            label: &str,
            events: Vec<egui::Event>,
        ) -> (OptionsResult, egui::Pos2) {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 2400.0))),
                events,
                ..Default::default()
            };
            st.prepare_scale_input(&raw, ctx.pixels_per_point(), false);
            let p = crate::theme::Theme::default().palette();
            let mut result = OptionsResult::default();
            let mut output = ctx.run_ui(raw, |ui| {
                let changed = content(ui, &p, s, st, &mut result);
                result.changed |= changed;
            });
            output.textures_delta.clear();
            let at = output
                .shapes
                .iter()
                .find_map(|s| label_position(&s.shape, label))
                .expect("preference label visible");
            (result, at)
        }
        for zoom in [true, false] {
            let ctx = egui::Context::default();
            let mut st = OptionsUi {
                tab: 1,
                ..Default::default()
            };
            let mut s = Settings::default();
            let initial_scale = s.ui_scale;
            let initial_snap = s.snap_windows;
            let label = if zoom {
                aurora_i18n::tr!("ui-options-interface-scale")
            } else {
                aurora_i18n::tr!("ui-options-snap-windows")
            };
            let (_, at) = frame(&ctx, &mut st, &mut s, label, vec![]);
            let pos = at + egui::vec2(LABEL_W + 8.0 + if zoom { SLIDER_W * 0.6 } else { 12.0 }, 0.0);
            let mut general_changed = false;
            let mut scale_changed = false;
            for pressed in [true, false] {
                let (result, _) = frame(
                    &ctx,
                    &mut st,
                    &mut s,
                    label,
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
                general_changed |= result.changed;
                scale_changed |= result.ui_scale_changed;
            }
            assert_eq!(scale_changed, zoom, "only interface scale takes the per-frame zoom path");
            assert_eq!(
                general_changed, !zoom,
                "other settings still update rendering/network/crash recovery"
            );
            if zoom {
                assert_ne!(s.ui_scale, initial_scale);
                assert_eq!(s.snap_windows, initial_snap);
            } else {
                assert_eq!(s.ui_scale, initial_scale);
                assert_ne!(s.snap_windows, initial_snap);
            }
        }
    }

    #[test]
    fn the_content_ratings_follow_the_account() {
        use aurora_net::search::{AgentAccess, SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};
        let ratings = |max| {
            maturity_choices(AgentAccess { max, preferred: max })
                .into_iter()
                .map(|(m, _)| m)
                .collect::<Vec<_>>()
        };
        assert_eq!(ratings(SIM_ACCESS_ADULT), [SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG]);
        // not adult: no "Adult"; below Moderate: no choice at all
        assert_eq!(ratings(SIM_ACCESS_MATURE), [SIM_ACCESS_MATURE, SIM_ACCESS_PG]);
        assert!(ratings(SIM_ACCESS_PG).is_empty());
    }

    #[test]
    fn shortcut_search_follows_rebinding_and_keeps_unbound_actions_searchable() {
        use crate::keybinds::{Binding, KeyBindings};
        let mut kb = KeyBindings::default();
        let matches = |query: &str, kb: &KeyBindings| {
            shortcut_matches(&query.trim().to_lowercase(), "Caméra", "Déplacer la vue", kb, Action::CamPanLeft)
        };
        kb.set(Action::CamPanLeft, 0, None);
        kb.set(Action::CamPanLeft, 1, None);
        assert!(matches("  DÉPLACER  ", &kb));
        assert!(matches("CAMÉRA", &kb));
        assert!(matches("   ", &kb));
        assert!(!matches("Ctrl+F12", &kb));
        let binding = Binding {
            input: Input::key(KeyCode::F12),
            ctrl: true,
            shift: false,
            alt: false,
        };
        for slot in 0..2 {
            kb.set(Action::CamPanLeft, slot, Some(binding.clone()));
            assert!(matches(" CTRL+F12 ", &kb));
            kb.set(Action::CamPanLeft, slot, None);
            assert!(!matches("Ctrl+F12", &kb));
        }
    }

    #[test]
    fn hud_modifier_alone_waits_for_release_but_a_combination_captures_its_key() {
        let mut ui = OptionsUi {
            capture: Some(Capture::Binding(Action::HudDrag, 1)),
            ..Default::default()
        };
        assert_eq!(ui.capture_key(KeyCode::ControlRight, true, false), None);
        assert_eq!(
            ui.capture_key(KeyCode::ControlRight, false, false),
            Some(Some(Input::key(KeyCode::ControlRight)))
        );
        ui.take_capture();
        ui.capture = Some(Capture::Binding(Action::HudDrag, 0));
        assert_eq!(ui.capture_key(KeyCode::ControlLeft, true, false), None);
        assert_eq!(ui.capture_key(KeyCode::KeyB, true, false), Some(Some(Input::key(KeyCode::KeyB))));
        ui.take_capture();
        assert_eq!(ui.capture_key(KeyCode::ControlLeft, false, false), None);
    }

    /// One frame of the name colors of the Colors page: whether a color
    /// picker is open, and whether the line about the classic tags is
    /// written.
    fn tag_colors_frame(ctx: &egui::Context, settings: &mut Settings, events: Vec<egui::Event>) -> (bool, bool) {
        let p = crate::theme::Theme::default().palette();
        let note = aurora_i18n::tr!("ui-options-tag-colors-classic-only");
        let picker = aurora_i18n::tr!("color-picker-title");
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(700.0, 500.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let classic = tag_colors_used(settings);
                tag_colors(ui, &p, &mut settings.colors, classic);
            },
        );
        output.textures_delta.clear();
        fn has(shape: &egui::Shape, text: &str) -> bool {
            match shape {
                egui::Shape::Text(t) => t.galley.text() == text,
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| has(s, text)),
                _ => false,
            }
        }
        (
            output.shapes.iter().any(|s| has(&s.shape, picker)),
            output.shapes.iter().any(|s| has(&s.shape, note)),
        )
    }

    /// Click along the column of the first swatches, each click on fresh
    /// settings: whether a click opened a color picker, whether one reached
    /// the switch of the colors by relation, and whether the line about the
    /// classic tags is written.
    fn tag_colors_clicks(settings: &Settings) -> (bool, bool, bool) {
        let (mut opened, mut flipped, mut noted) = (false, false, false);
        for y in (20..260).step_by(4) {
            let ctx = egui::Context::default();
            let mut clicked = settings.clone();
            noted |= tag_colors_frame(&ctx, &mut clicked, vec![]).1;
            let pos = egui::pos2(30.0, y as f32);
            for pressed in [true, false] {
                let click = egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                tag_colors_frame(&ctx, &mut clicked, vec![egui::Event::PointerMoved(pos), click]);
            }
            flipped |= clicked.colors.relation_names != settings.colors.relation_names;
            opened |= tag_colors_frame(&ctx, &mut clicked, vec![]).0;
        }
        (opened, flipped, noted)
    }

    #[test]
    fn the_name_colors_are_greyed_until_switched_on_or_used_by_the_classic_tags() {
        let settings = Settings::default();
        assert!(!settings.classic_name_tags, "the name plates are the default style");
        assert!(!settings.colors.relation_names, "every name is white by default");
        let (opened, flipped, noted) = tag_colors_clicks(&settings);
        assert!(!opened, "a greyed swatch does not open its picker");
        assert!(flipped, "the switch of the colors by relation is live");
        assert!(noted, "the page says the matching pair belongs to the classic tags");
        // the colors by relation, switched on: their swatches are live
        let mut by_relation = settings.clone();
        by_relation.colors.relation_names = true;
        assert!(tag_colors_clicks(&by_relation).0);
        // the classic style: its own pair is live, the note is gone
        let mut classic = settings.clone();
        classic.classic_name_tags = true;
        let (opened, _, noted) = tag_colors_clicks(&classic);
        assert!(opened && !noted);
    }

    #[test]
    fn ordinary_capture_still_ignores_modifier_keys_and_escape_cancels() {
        let mut ui = OptionsUi {
            capture: Some(Capture::Binding(Action::Inventory, 0)),
            ..Default::default()
        };
        assert_eq!(ui.capture_key(KeyCode::ControlLeft, true, false), None);
        assert_eq!(ui.capture_key(KeyCode::ControlLeft, false, false), None);
        assert_eq!(ui.capture_key(KeyCode::KeyI, true, true), None);
        assert_eq!(ui.capture_key(KeyCode::KeyI, true, false), Some(Some(Input::key(KeyCode::KeyI))));
        assert_eq!(ui.capture_key(KeyCode::Escape, true, false), Some(None));
    }
}
