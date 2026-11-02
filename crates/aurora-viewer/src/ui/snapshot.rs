//! The photo window, anchored on the right of the world (Firestorm's
//! snapshot floater, floater_snapshot.xml / panel_snapshot_local.xml, laid
//! out for photographers: preview with its guide, proportions and sizes in
//! one click, file, what the image holds, focal length, latest photos), and
//! the full-frame view: the interface gone but a small command strip, the
//! world outside the photo's proportions darkened, a composition guide.

use super::widgets::{self, ActionKind, Floater};
use crate::snapshot::inventory::TextureSize;
use crate::snapshot::profile::ProfileSize;
use crate::snapshot::{Aspect, FileFormat, Guide, Resolution, Settings};
use crate::theme::Palette;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::path::{Path, PathBuf};

pub const WINDOW: &str = "photo";
/// Firestorm's floater (Preferences › Aurora › Photo window): its own
/// place and size, kept apart from the panel's.
pub const CLASSIC_WINDOW: &str = "photo_classic";
/// Side of the classic window's preview (floater_snapshot.xml's
/// thumbnail_placeholder, 420 x 420).
const CLASSIC_PREVIEW: f32 = 420.0;
const WIDTH: f32 = 340.0;
/// Focal lengths of the slider and its presets (mm, full frame).
pub(crate) const FOCAL_RANGE: std::ops::RangeInclusive<f32> = 10.0..=135.0;
const FOCAL_PRESETS: [f32; 5] = [24.0, 35.0, 50.0, 85.0, 135.0];

#[derive(Default)]
pub struct SnapshotUi {
    /// The full-frame view instead of the interface.
    pub full_frame: bool,
    /// The photo is taken and shown still until saved or released.
    pub freeze: bool,
    /// The world holds still while the camera moves (never saved: the
    /// photo mode's end thaws it).
    pub freeze_world: bool,
    /// Where "Take the photo" sends it (the disk each time the viewer
    /// starts: a photo that costs L$ is never one by surprise).
    pub destination: Destination,
    /// The caption of a profile post (not kept, like Firestorm's editor).
    pub caption: String,
    /// The e-mail of a postcard (not kept either).
    pub postcard: PostcardForm,
    /// The quick light, for the photo only (never saved).
    pub light: crate::snapshot::light::PhotoLight,
    /// Scroll the panel to the Light section on the next frame (demo).
    pub reveal_light: bool,
    /// The free camera's switch and flight (never saved).
    pub free: crate::camera::free::FreeSettings,
    /// The looks editor's state (`ui::looks_editor`).
    pub looks: super::looks_editor::LooksUi,
    /// The user's 3D tables, by file name (`snapshot::luts`).
    pub lut_names: Vec<String>,
}

/// Where a photo goes (Firestorm's snapshot panels; the profile feed
/// comes later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Destination {
    #[default]
    Disk,
    Inventory,
    Profile,
    /// A postcard (LLPanelSnapshotPostcard).
    Email,
}

/// The e-mail of a postcard (panel_postcard_message.xml).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostcardForm {
    /// The recipients, separated by commas.
    pub to: String,
    /// The sender's name; the app writes ours while it is empty.
    pub name: String,
    pub subject: String,
    pub message: String,
}

/// panel_postcard_message.xml's max_length_bytes / max_length.
const POSTCARD_NAME_MAX: usize = 100;
const POSTCARD_SUBJECT_MAX: usize = 100;
const POSTCARD_MESSAGE_MAX: usize = 700;

/// A photo sent by e-mail.
pub struct EmailView {
    /// The region takes postcards (SendPostcard), or the demo.
    pub available: bool,
    pub size: (u32, u32),
    /// Our name, sent while "From" is empty (shown as its hint).
    pub sender: String,
}

/// A photo for the profile feed.
pub struct ProfileView {
    /// Logged in (or in demo).
    pub connected: bool,
    /// The web profile knows who we are (the login's OpenID cookie came).
    pub ready: bool,
    pub size: (u32, u32),
}

/// The environment under the photo's light, for its Light section.
pub struct EnvView<'a> {
    /// The sky, water and day cycle lists (the environment window's).
    pub catalog: &'a crate::world::env_select::SettingsCatalog,
    /// What the user's own environment shows in each list.
    pub own: crate::world::eep_env::LocalView,
    /// The sun shown: azimuth and elevation (degrees).
    pub sun: (f32, f32),
    /// RLVa holds the environment (@setenv): nothing to change.
    pub locked: bool,
    /// An item the photo picked is still loading.
    pub loading: bool,
}

/// A photo for the inventory: the texture it becomes and its price.
pub struct InventoryView {
    pub texture: (u32, u32),
    /// The upload cost in L$; None while it is not known.
    pub cost: Option<i32>,
    /// Logged in (or in demo): the upload is possible.
    pub connected: bool,
}

/// What the window shows besides the settings.
pub struct View<'a> {
    pub window: (u32, u32),
    pub photo: (u32, u32),
    /// Parts the photo is drawn in (1: one pass).
    pub tiles: usize,
    pub preview: Option<&'a egui::TextureHandle>,
    /// Work in progress: (0..1, what).
    pub busy: Option<(f32, String)>,
    /// Last outcome: (text, is an error).
    pub message: Option<&'a (String, bool)>,
    pub last_saved: Option<&'a Path>,
    pub gallery: &'a [(PathBuf, Option<egui::TextureHandle>)],
    pub folder: PathBuf,
    /// The destination is the inventory.
    pub inventory: Option<InventoryView>,
    /// The message tells of a photo saved to the inventory ("Show").
    pub item_saved: bool,
    /// The destination is the profile feed.
    pub profile: Option<ProfileView>,
    /// The message tells of a post to the profile feed ("View").
    pub profile_posted: bool,
    /// The destination is a postcard.
    pub email: Option<EmailView>,
    /// Preferences' exposure, under the photo's own.
    pub exposure: f32,
    /// HDR and Emissive is on: without it no exposure applies, the row is
    /// greyed.
    pub hdr: bool,
    pub env: EnvView<'a>,
    /// The key that brings the free camera back (Escape by default).
    pub back_key: Option<crate::keybinds::Binding>,
    /// The photo quality is lowered to stay within the video memory.
    pub quality_reduced: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Refresh,
    Take,
    SaveAs,
    ChooseFolder,
    OpenFolder,
    OpenPhoto(PathBuf),
    /// Freeze frame switched (on: take it now).
    Freeze(bool),
    /// A setting that changes the photo changed: the preview follows.
    Changed,
    /// The settings changed (saved, the preview unchanged).
    Saved,
    /// Show the photo saved to the inventory.
    ShowItem,
    /// Open our profile (its feed shows the post).
    ShowProfile,
    /// Open the personal lighting window.
    PersonalLighting,
    /// "Keep this light": the photo's environment becomes the user's.
    KeepLight,
    /// Open the Controls window (it drives the free camera).
    Controls,
    /// The upload's price was confirmed (true) or not.
    Confirm(bool),
    /// The "cannot afford" message was closed.
    CloseAfford,
    /// Choose a .cube file to add to the tables of the photo's look.
    ImportLut,
    /// Delete one of the tables (its file name).
    RemoveLut(String),
    /// Show the folder of the tables.
    OpenLutFolder,
}

impl SnapshotUi {
    /// The window; `fov` is the camera's vertical field of view (radians),
    /// `default_fov` the viewer's.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        open: &mut bool,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        classic: bool,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        self.looks.hover = None;
        let screen = ctx.content_rect();
        let mut full = false;
        let full_tip = aurora_i18n::tr!("photo-full-frame");
        if classic {
            // a window of its own, placed and moved like Firestorm's floater
            let size = Vec2::new(WIDTH + CLASSIC_PREVIEW + 8.0, CLASSIC_PREVIEW + 160.0);
            Floater::new(CLASSIC_WINDOW, aurora_i18n::tr!("photo-title"), screen.center() - size * 0.5, size)
                .auto_height()
                .action("maximize", full_tip, &mut full)
                .show(ctx, p, open, |ui| self.body_classic(ui, p, s, fov, default_fov, view, &mut out));
        } else {
            Floater::new(
                WINDOW,
                aurora_i18n::tr!("photo-title"),
                Pos2::new(screen.right() - WIDTH - 24.0, screen.top() + 64.0),
                Vec2::new(WIDTH, 640.0),
            )
            .fixed()
            .auto_height()
            .action("maximize", full_tip, &mut full)
            .show(ctx, p, open, |ui| self.body(ui, p, s, fov, default_fov, view, &mut out));
        }
        if full {
            self.full_frame = true;
        }
        if !*open {
            self.full_frame = false;
            if self.freeze {
                self.freeze = false;
                out.push(Action::Freeze(false));
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn body(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        out: &mut Vec<Action>,
    ) {
        ui.set_width(WIDTH - 24.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        if preview(ui, p, s, view) {
            out.push(Action::Refresh);
        }

        let before = s.clone();
        let light_before = self.light;
        // the settings scroll between the preview and the shutter, which
        // stay in sight on a short screen
        let room = (ui.ctx().content_rect().height() - 470.0).max(150.0);
        egui::ScrollArea::vertical()
            .id_salt("photo_settings")
            .max_height(room)
            .show(ui, |ui| self.settings(ui, p, s, fov, default_fov, view, out));

        self.actions(ui, p, view, out);

        changed(s, &before, out);
        if self.light != light_before && !out.contains(&Action::Changed) {
            out.push(Action::Changed);
        }
    }

    /// The viewfinder's drawer: every setting of the panel, its shutter,
    /// progress, last outcome and latest photos, without the preview (the
    /// viewfinder is one); the settings scroll in `room` points.
    #[allow(clippy::too_many_arguments)]
    pub fn drawer(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        room: f32,
        out: &mut Vec<Action>,
    ) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let before = s.clone();
        let light_before = self.light;
        let freeze = self.freeze;
        egui::ScrollArea::vertical()
            .id_salt("photo_drawer")
            .max_height(room)
            .show(ui, |ui| self.settings(ui, p, s, fov, default_fov, view, out));
        self.actions(ui, p, view, out);
        changed(s, &before, out);
        if (self.light != light_before || self.freeze != freeze) && !out.contains(&Action::Changed) {
            out.push(Action::Changed);
        }
    }

    /// The viewfinder's light panel: the Light section alone, the dock's
    /// shortcut to the drawer's section of the same name.
    pub fn light_only(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let before = self.light;
        light_rows(ui, p, &mut self.light, view.exposure, view.hdr, &view.env, out);
        if self.light != before && !out.contains(&Action::Changed) {
            out.push(Action::Changed);
        }
    }

    /// The shutter (or the destination's button), the progress or the last
    /// outcome, and the latest photos.
    fn actions(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Vec<Action>) {
        ui.add_space(4.0);
        let busy = view.busy.is_some();
        ui.horizontal(|ui| match (&view.email, &view.inventory, &view.profile) {
            (Some(mail), _, _) => {
                let ready = !busy && mail.available;
                if widgets::action_button_if(
                    ui,
                    p,
                    "mail-filled",
                    aurora_i18n::tr!("photo-email-send"),
                    ActionKind::Primary,
                    Vec2::new(ui.available_width(), 32.0),
                    ready,
                )
                .clicked()
                {
                    out.push(Action::Take);
                }
            }
            (None, _, Some(prof)) => {
                let ready = !busy && prof.connected && prof.ready;
                if widgets::action_button_if(
                    ui,
                    p,
                    "send-filled",
                    aurora_i18n::tr!("photo-profile-post"),
                    ActionKind::Primary,
                    Vec2::new(ui.available_width(), 32.0),
                    ready,
                )
                .clicked()
                {
                    out.push(Action::Take);
                }
            }
            (None, Some(inv), None) => {
                let ready = !busy && inv.connected && inv.cost.is_some();
                if widgets::action_button_if(
                    ui,
                    p,
                    "file-upload-filled",
                    aurora_i18n::tr!("photo-save-inventory"),
                    ActionKind::Primary,
                    Vec2::new(ui.available_width(), 32.0),
                    ready,
                )
                .clicked()
                {
                    out.push(Action::Take);
                }
            }
            (None, None, None) => {
                let take_w = ui.available_width() - 34.0;
                ui.add_enabled_ui(!busy, |ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "camera-filled",
                        aurora_i18n::tr!("photo-take"),
                        ActionKind::Primary,
                        Vec2::new(take_w, 32.0),
                    )
                    .clicked()
                    {
                        out.push(Action::Take);
                    }
                    if widgets::action_button(ui, p, "folder-filled", "", ActionKind::Flat, Vec2::new(30.0, 32.0))
                        .on_hover_text(aurora_i18n::tr!("photo-save-as"))
                        .clicked()
                    {
                        out.push(Action::SaveAs);
                    }
                });
            }
        });
        if let Some((fraction, what)) = &view.busy {
            widgets::progress(ui, p, "photo_progress", *fraction, ui.available_width(), what, None);
        } else if let Some((text, error)) = view.message {
            // the button on the right, the text cut before it (a long file
            // name never widens the window)
            let line = Vec2::new(ui.available_width(), 26.0);
            ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = match () {
                    _ if *error => None,
                    _ if view.item_saved => Some((aurora_i18n::tr!("photo-show-in-inventory"), Action::ShowItem)),
                    _ if view.profile_posted => Some((aurora_i18n::tr!("photo-profile-show"), Action::ShowProfile)),
                    _ => view
                        .last_saved
                        .map(|path| (aurora_i18n::tr!("photo-open"), Action::OpenPhoto(path.to_path_buf()))),
                };
                if let Some((label, action)) = button
                    && widgets::flat_button(ui, p, label).clicked()
                {
                    out.push(action);
                }
                let color = if *error { p.danger } else { p.success };
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).truncate());
                });
            });
        }
        gallery(ui, p, view, out);
    }

    /// The postcard's e-mail (panel_postcard_message.xml) and its JPEG
    /// quality (SnapshotQuality, shared with the disk's JPEG). An empty
    /// subject or message is sent with the default shown in the field
    /// (PromptMissingSubjMsg's choice, without the question).
    fn postcard_form(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, mail: &EmailView) {
        if !mail.available {
            ui.label(RichText::new(aurora_i18n::tr!("photo-email-unavailable")).color(p.muted));
        }
        let form = &mut self.postcard;
        let fields: [(&str, &mut String, &str, usize); 3] = [
            (
                aurora_i18n::tr!("photo-email-to"),
                &mut form.to,
                aurora_i18n::tr!("photo-email-to-hint"),
                usize::MAX,
            ),
            (
                aurora_i18n::tr!("photo-email-from"),
                &mut form.name,
                &mail.sender,
                POSTCARD_NAME_MAX,
            ),
            (
                aurora_i18n::tr!("photo-email-subject"),
                &mut form.subject,
                aurora_i18n::tr!("photo-email-default-subject"),
                POSTCARD_SUBJECT_MAX,
            ),
        ];
        for (label, text, hint, max) in fields {
            ui.label(RichText::new(label).color(p.ink));
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .char_limit(max)
                    .desired_width(f32::INFINITY),
            );
        }
        ui.label(RichText::new(aurora_i18n::tr!("photo-email-message")).color(p.ink));
        let mut message = egui::TextEdit::multiline(&mut form.message)
            .hint_text(aurora_i18n::tr!("photo-email-default-message"))
            .char_limit(POSTCARD_MESSAGE_MAX)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .show(ui);
        // spell checked, as panel_postcard_message.xml
        crate::ui::spelling::field(ui, p, &mut message, &mut form.message, false);
        row(ui, p, aurora_i18n::tr!("photo-jpeg-quality"), None, |ui| {
            ui.spacing_mut().slider_width = 130.0;
            ui.add(egui::Slider::new(&mut s.jpeg_quality, 1..=100));
        });
    }

    /// Firestorm's snapshot floater (floater_snapshot.xml, Preferences ›
    /// Aurora › Photo window › Classic window): a floating window, the
    /// controls in a column on the left (Refresh first, then the options and
    /// the destination), the preview on the right; the same controls.
    #[allow(clippy::too_many_arguments)]
    fn body_classic(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        out: &mut Vec<Action>,
    ) {
        let before = s.clone();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(WIDTH - 24.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                // new_snapshot_btn
                if widgets::action_button(
                    ui,
                    p,
                    "refresh",
                    aurora_i18n::tr!("photo-refresh"),
                    ActionKind::Flat,
                    Vec2::new(ui.available_width(), 28.0),
                )
                .clicked()
                {
                    out.push(Action::Refresh);
                }
                egui::ScrollArea::vertical()
                    .id_salt("photo_settings_classic")
                    .max_height(CLASSIC_PREVIEW + 40.0)
                    .show(ui, |ui| self.settings(ui, p, s, fov, default_fov, view, out));
                self.actions(ui, p, view, out);
            });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_width(CLASSIC_PREVIEW);
                ui.spacing_mut().item_spacing.y = 6.0;
                if preview(ui, p, s, view) {
                    out.push(Action::Refresh);
                }
            });
        });
        changed(s, &before, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn settings(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        out: &mut Vec<Action>,
    ) {
        ui.set_width(WIDTH - 36.0);
        section(ui, p, "aspect-ratio-filled", aurora_i18n::tr!("photo-section-format"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for a in Aspect::ALL {
                let label = a.short().unwrap_or(aurora_i18n::tr!("photo-aspect-window"));
                if chip(ui, p, label, None, s.aspect == a, true).clicked() {
                    s.set_aspect(a, view.window);
                }
            }
        });

        section(ui, p, "dimensions", aurora_i18n::tr!("photo-section-resolution"));
        match (&view.email, &view.inventory, &view.profile) {
            (Some(mail), _, _) => long_sides(ui, p, &mut s.postcard_size, mail.size),
            (None, _, Some(prof)) => long_sides(ui, p, &mut s.profile_size, prof.size),
            (None, Some(inv), None) => texture_sizes(ui, p, s, inv),
            (None, None, None) => resolutions(ui, p, s, view),
        }
        // the photo quality lowered for the video memory (the switch is
        // further down, often out of sight): with how the photo is drawn
        if view.quality_reduced && s.quality {
            ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-quality-reduced")).size(11.5).color(p.warn)).wrap());
        }

        section(ui, p, "send-filled", aurora_i18n::tr!("photo-section-destination"));
        let mut dest = match self.destination {
            Destination::Disk => 0,
            Destination::Inventory => 1,
            Destination::Profile => 2,
            Destination::Email => 3,
        };
        if widgets::segmented(
            ui,
            p,
            &mut dest,
            &[
                (aurora_i18n::tr!("photo-destination-disk"), true),
                (aurora_i18n::tr!("photo-destination-inventory"), true),
                (aurora_i18n::tr!("photo-destination-profile"), true),
                (aurora_i18n::tr!("photo-destination-email"), true),
            ],
        ) {
            self.destination = match dest {
                1 => Destination::Inventory,
                2 => Destination::Profile,
                3 => Destination::Email,
                _ => Destination::Disk,
            };
            out.push(Action::Changed);
        }
        match (&view.email, &view.inventory, &view.profile) {
            (Some(mail), _, _) => self.postcard_form(ui, p, s, mail),
            (None, _, Some(prof)) => {
                if !prof.connected {
                    ui.label(RichText::new(aurora_i18n::tr!("photo-profile-offline")).color(p.muted));
                } else if !prof.ready {
                    ui.label(RichText::new(aurora_i18n::tr!("photo-profile-not-ready")).color(p.muted));
                }
                ui.label(RichText::new(aurora_i18n::tr!("photo-profile-caption")).color(p.ink));
                let mut caption = egui::TextEdit::multiline(&mut self.caption)
                    .hint_text(aurora_i18n::tr!("photo-profile-caption-hint"))
                    .char_limit(crate::snapshot::profile::MAX_CAPTION)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .show(ui);
                // spell checked, as panel_snapshot_profile.xml
                crate::ui::spelling::field(ui, p, &mut caption, &mut self.caption, false);
                row(ui, p, aurora_i18n::tr!("photo-profile-location"), None, |ui| {
                    widgets::switch(ui, p, &mut s.profile_location);
                });
            }
            (None, Some(inv), None) => {
                let cost = match (inv.connected, inv.cost) {
                    (false, _) => aurora_i18n::tr!("photo-inventory-offline").to_string(),
                    (true, Some(cost)) => aurora_i18n::tr!("photo-upload-cost", cost = cost).to_string(),
                    (true, None) => aurora_i18n::tr!("photo-upload-cost-unknown").to_string(),
                };
                ui.label(RichText::new(cost).color(p.ink));
            }
            (None, None, None) => {
                row(ui, p, aurora_i18n::tr!("photo-file-format"), None, |ui| {
                    let mut i = FileFormat::ALL.iter().position(|f| *f == s.format).unwrap_or(0);
                    let options: Vec<(&str, bool)> = FileFormat::ALL.iter().map(|f| (f.label(), true)).collect();
                    if widgets::segmented(ui, p, &mut i, &options) {
                        s.format = FileFormat::ALL[i];
                    }
                });
                if s.format == FileFormat::Jpeg {
                    row(ui, p, aurora_i18n::tr!("photo-jpeg-quality"), None, |ui| {
                        ui.spacing_mut().slider_width = 130.0;
                        ui.add(egui::Slider::new(&mut s.jpeg_quality, 1..=100));
                    });
                }
            }
        }

        section(ui, p, "adjustments-horizontal-filled", aurora_i18n::tr!("photo-section-in-image"));
        let interface_possible = crate::snapshot::same_proportions(view.photo, view.window);
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-interface"),
            (!interface_possible).then(|| aurora_i18n::tr!("photo-interface-hint")),
            |ui| {
                let mut on = s.interface && interface_possible;
                ui.add_enabled_ui(interface_possible, |ui| {
                    if widgets::switch(ui, p, &mut on).changed() {
                        s.interface = on;
                    }
                });
            },
        );
        row(ui, p, aurora_i18n::tr!("photo-huds"), None, |ui| {
            widgets::switch(ui, p, &mut s.huds);
        });
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-quality"),
            Some(aurora_i18n::tr!("photo-quality-hint")),
            |ui| {
                widgets::switch(ui, p, &mut s.quality);
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-freeze"),
            Some(aurora_i18n::tr!("photo-freeze-hint")),
            |ui| {
                let mut on = self.freeze;
                if widgets::switch(ui, p, &mut on).changed() {
                    self.freeze = on;
                    out.push(Action::Freeze(on));
                }
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-freeze-world"),
            Some(aurora_i18n::tr!("photo-freeze-world-hint")),
            |ui| {
                widgets::switch(ui, p, &mut self.freeze_world);
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-auto-refresh"),
            Some(aurora_i18n::tr!("photo-auto-refresh-hint")),
            |ui| {
                widgets::switch(ui, p, &mut s.auto_refresh);
            },
        );

        section(ui, p, "camera-filled", aurora_i18n::tr!("photo-section-camera"));
        if focal(ui, p, fov, default_fov) {
            out.push(Action::Saved);
        }
        row(ui, p, aurora_i18n::tr!("photo-free-camera"), None, |ui| {
            widgets::switch(ui, p, &mut self.free.on);
        });
        free_camera_hint(ui, p, view.back_key.as_ref());
        if widgets::action_button(
            ui,
            p,
            "video-filled",
            aurora_i18n::tr!("photo-free-camera-controls"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
        )
        .on_hover_text(aurora_i18n::tr!("photo-free-camera-controls-tip"))
        .clicked()
        {
            out.push(Action::Controls);
        }

        section(ui, p, "focus-centered", aurora_i18n::tr!("photo-section-dof"));
        dof_rows(ui, p, &mut s.dof);

        let light = ui.cursor();
        section(ui, p, "sun-filled", aurora_i18n::tr!("photo-section-light"));
        light_rows(ui, p, &mut self.light, view.exposure, view.hdr, &view.env, out);
        if std::mem::take(&mut self.reveal_light) {
            ui.scroll_to_rect(light.with_max_y(ui.cursor().top()), Some(egui::Align::Center));
        }

        let filter = ui.cursor();
        section(ui, p, "color-filter", aurora_i18n::tr!("photo-looks"));
        let opts = super::looks_editor::Options {
            list_height: 220.0,
            compare: false,
            luts: &self.lut_names,
        };
        self.looks.show(ui, p, s, opts, out);
        if std::mem::take(&mut self.looks.reveal) {
            ui.scroll_to_rect(filter.with_max_y(ui.cursor().top()), Some(egui::Align::Center));
        }

        ui.add_space(2.0);
        widgets::collapsing(ui, p, "photo_file", aurora_i18n::tr!("photo-section-file"), false, |ui| {
            row(ui, p, aurora_i18n::tr!("photo-base-name"), None, |ui| {
                ui.add(egui::TextEdit::singleline(&mut s.base_name).desired_width(150.0));
            });
            row(ui, p, aurora_i18n::tr!("photo-timestamp"), None, |ui| {
                widgets::switch(ui, p, &mut s.timestamp_names);
            });
            row(
                ui,
                p,
                aurora_i18n::tr!("photo-folder"),
                Some(&view.folder.display().to_string()),
                |ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("photo-folder-change")).clicked() {
                        out.push(Action::ChooseFolder);
                    }
                },
            );
            row(ui, p, aurora_i18n::tr!("photo-remember-folder"), None, |ui| {
                widgets::switch(ui, p, &mut s.remember_folder);
            });
            row(ui, p, aurora_i18n::tr!("photo-sound"), None, |ui| {
                widgets::switch(ui, p, &mut s.sound);
            });
        });
    }

    /// The price to confirm before a paid upload (`confirm`), or the price
    /// the balance cannot pay (`short`: LLPanelSnapshotInventory::onSend
    /// and its ErrorPhotoCannotAfford).
    pub fn dialogs(&self, ctx: &egui::Context, p: &Palette, confirm: Option<i32>, short: Option<i32>) -> Vec<Action> {
        use widgets::{Modal, action_button};
        let mut out = Vec::new();
        let size = Vec2::new(0.0, 26.0);
        if let Some(cost) = confirm {
            let mut answer = None;
            let response = Modal::new(egui::Id::unique("photo_upload_confirm")).show(ctx, |ui| {
                ui.set_width(350.0);
                ui.label(RichText::new(aurora_i18n::tr!("photo-confirm-title")).size(17.0).color(p.ink));
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("photo-confirm-text", cost = cost));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = aurora_i18n::tr!("photo-confirm-ok", cost = cost);
                    if action_button(ui, p, "file-upload-filled", &ok, ActionKind::Primary, size).clicked() {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if answer.is_none() && response.should_close() {
                answer = Some(false);
            }
            out.extend(answer.map(Action::Confirm));
        }
        if let Some(cost) = short {
            let mut close = false;
            let response = Modal::new(egui::Id::unique("photo_cannot_afford")).show(ctx, |ui| {
                ui.set_width(350.0);
                ui.label(aurora_i18n::tr!("photo-cannot-afford", cost = cost));
                ui.add_space(8.0);
                if action_button(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size).clicked() {
                    close = true;
                }
            });
            if close || response.should_close() {
                out.push(Action::CloseAfford);
            }
        }
        out
    }

    /// The full-frame view (instead of the interface); `photo` the photo's
    /// size. Escape or the exit button leaves it.
    pub fn full_frame(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        photo: (u32, u32),
    ) -> Vec<Action> {
        let mut out = Vec::new();
        let screen = ctx.content_rect();
        let frame = fit(screen, photo.0 as f32 / photo.1.max(1) as f32);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("photo_full_frame")));
        let mask = p.navy.gamma_multiply(0.62);
        for r in outside(screen, frame) {
            painter.rect_filled(r, 0.0, mask);
        }
        painter.rect_stroke(frame, 0.0, Stroke::new(1.0, p.ink.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        guide_lines(&painter, frame, s.guide, p.ink.gamma_multiply(0.55));

        let tag = |id: &str, pos: Pos2, align: egui::Align2, icon: &str, text: String| {
            egui::Area::new(egui::Id::unique(id))
                .order(egui::Order::Foreground)
                .pivot(align)
                .fixed_pos(pos)
                .interactable(false)
                .show(ctx, |ui| {
                    widgets::overlay(ui, p, egui::Margin::symmetric(8, 4), |ui| {
                        ui.horizontal(|ui| {
                            icon_image(ui, icon, 14.0, p.teal);
                            ui.label(RichText::new(text).size(12.0).color(p.ink));
                        });
                    });
                });
        };
        let label = s.aspect.short().unwrap_or(aurora_i18n::tr!("photo-aspect-window"));
        tag(
            "photo_full_size",
            screen.left_top() + Vec2::new(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            "aspect-ratio-filled",
            format!("{label} · {} × {}", photo.0, photo.1),
        );
        tag(
            "photo_full_exit",
            screen.right_top() + Vec2::new(-12.0, 12.0),
            egui::Align2::RIGHT_TOP,
            "x",
            aurora_i18n::tr!("photo-exit-full-frame").to_string(),
        );

        let before = s.clone();
        egui::Area::new(egui::Id::unique("photo_full_strip"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - 14.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::symmetric(10, 8), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for a in [
                            Aspect::Window,
                            Aspect::R16x9,
                            Aspect::R3x2,
                            Aspect::R1x1,
                            Aspect::R4x5,
                            Aspect::R9x16,
                        ] {
                            let label = a.short().unwrap_or(aurora_i18n::tr!("photo-aspect-window"));
                            if chip(ui, p, label, None, s.aspect == a, true).clicked() {
                                s.aspect = a;
                            }
                        }
                        ui.add_space(8.0);
                        for g in Guide::ALL {
                            let (icon, tip) = guide_face(g);
                            if widgets::icon_toggle(ui, p, icon, tip, s.guide == g) {
                                s.guide = if s.guide == g { Guide::None } else { g };
                            }
                        }
                        ui.add_space(8.0);
                        if shutter(ui, p).clicked() {
                            out.push(Action::Take);
                        }
                        ui.add_space(8.0);
                        if focal(ui, p, fov, default_fov) {
                            out.push(Action::Saved);
                        }
                        ui.add_space(4.0);
                        strip_aperture(ui, p, &mut s.dof);
                        ui.add_space(4.0);
                        if widgets::icon_button(ui, p, "minimize", aurora_i18n::tr!("photo-exit-full-frame"), true) {
                            self.full_frame = false;
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.full_frame = false;
        }
        if *s != before {
            out.push(if s.aspect != before.aspect || s.dof != before.dof {
                Action::Changed
            } else {
                Action::Saved
            });
        }
        out
    }
}

/// The disk's sizes: presets, the window's, or typed, with how the photo
/// is drawn.
fn resolutions(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, view: &View) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for r in Resolution::ALL {
            let (label, sub) = match r {
                Resolution::Window => (aurora_i18n::tr!("photo-resolution-window"), None),
                Resolution::Custom => (aurora_i18n::tr!("photo-resolution-custom"), None),
                _ => (r.short().unwrap_or(""), r.long_side().map(|v| v.to_string())),
            };
            if chip(ui, p, label, sub.as_deref(), s.resolution == r, true).clicked() {
                if r == Resolution::Custom && s.resolution != Resolution::Custom {
                    // the typed size starts from the photo of the moment
                    let (w, h) = s.size(view.window);
                    s.custom = [w, h];
                }
                s.resolution = r;
            }
        }
    });
    ui.horizontal(|ui| {
        if s.resolution == Resolution::Custom {
            let (mut w, mut h) = (s.custom[0], s.custom[1]);
            let range = crate::snapshot::MIN_SIDE..=crate::snapshot::MAX_SIDE;
            let wc = ui.add(egui::DragValue::new(&mut w).range(range.clone()).speed(4.0)).changed();
            let lock = if s.keep_ratio { "lock-filled" } else { "lock-open" };
            if widgets::icon_toggle(ui, p, lock, aurora_i18n::tr!("photo-keep-ratio"), s.keep_ratio) {
                s.keep_ratio = !s.keep_ratio;
            }
            let hc = ui.add(egui::DragValue::new(&mut h).range(range).speed(4.0)).changed();
            if wc || hc {
                s.set_custom(wc.then_some(w), hc.then_some(h), view.window);
            }
        } else {
            let (w, h) = view.photo;
            ui.label(RichText::new(aurora_i18n::tr!("photo-size", width = w, height = h)).color(p.ink));
        }
        let passes = if view.tiles <= 1 {
            aurora_i18n::tr!("photo-one-pass").to_string()
        } else {
            aurora_i18n::tr!("photo-tiles", count = view.tiles).to_string()
        };
        ui.label(RichText::new(passes).size(11.5).color(p.muted));
    });
}

/// The window's depth of field (the mockup's card): on, aperture, focus by
/// click, pointer or a distance set by hand. It drives the view's effect
/// while the window is open; Preferences › Graphics keeps its own.
fn dof_rows(ui: &mut egui::Ui, p: &Palette, d: &mut crate::dof::PhotoDof) {
    use crate::dof::PhotoFocus;
    row(
        ui,
        p,
        aurora_i18n::tr!("photo-dof-enable"),
        Some(aurora_i18n::tr!("photo-dof-enable-hint")),
        |ui| {
            widgets::switch(ui, p, &mut d.enabled);
        },
    );
    ui.add_enabled_ui(d.enabled, |ui| {
        row(ui, p, aurora_i18n::tr!("photo-dof-aperture"), None, |ui| {
            // the slider, then its value, like the focal length's
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(60.0);
                ui.add(
                    egui::Slider::new(&mut d.f_number, 1.0..=22.0)
                        .logarithmic(true)
                        .prefix("f/")
                        .fixed_decimals(1)
                        .clamping(egui::SliderClamping::Edits)
                        .custom_parser(aurora_i18n::parse_number),
                );
            });
        });
        row(ui, p, aurora_i18n::tr!("photo-dof-focus"), None, |ui| {
            let modes = [PhotoFocus::Click, PhotoFocus::Pointer, PhotoFocus::Manual];
            let mut i = modes.iter().position(|m| *m == d.focus).unwrap_or(0);
            let options = [
                (aurora_i18n::tr!("photo-dof-focus-click"), true),
                (aurora_i18n::tr!("photo-dof-focus-pointer"), true),
                (aurora_i18n::tr!("photo-dof-focus-manual"), true),
            ];
            if widgets::segmented(ui, p, &mut i, &options) {
                d.focus = modes[i];
            }
        });
        if d.focus == PhotoFocus::Manual {
            row(ui, p, aurora_i18n::tr!("photo-dof-distance"), None, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(60.0);
                    ui.add(
                        egui::Slider::new(&mut d.distance, 0.3..=200.0)
                            .logarithmic(true)
                            .suffix(" m")
                            .fixed_decimals(1)
                            .clamping(egui::SliderClamping::Edits)
                            .custom_parser(aurora_i18n::parse_number),
                    );
                });
            });
        }
    });
}

/// The window's quick light (Phototools' Quick Environments, the sky, water
/// and day cycle lists of its Env tab, Pause Clouds, Attached Lights and
/// Exposure, and the sun of "Personal lighting"), for the photo only: the
/// window gives the user's own back when it closes, unless "Keep this
/// light".
fn light_rows(
    ui: &mut egui::Ui,
    p: &Palette,
    l: &mut crate::snapshot::light::PhotoLight,
    exposure: f32,
    hdr: bool,
    env: &EnvView,
    out: &mut Vec<Action>,
) {
    use crate::snapshot::light::EXPOSURE;
    ui.add_enabled_ui(!env.locked, |ui| env_rows(ui, p, l, env))
        .response
        .on_disabled_hover_text(aurora_i18n::tr!("rlva-blocked"));
    row(ui, p, aurora_i18n::tr!("photo-light-exposure"), None, |ui| {
        // (no exposure without HDR: LLPipeline::gammaCorrect)
        ui.add_enabled_ui(hdr, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // (the value and the reset button after it)
                ui.spacing_mut().slider_width = (ui.available_width() - 130.0).max(40.0);
                let mut value = l.exposure.unwrap_or(exposure);
                // (shown, the sky's exposure must not turn into a fixed one)
                let slider = egui::Slider::new(&mut value, EXPOSURE)
                    .logarithmic(true)
                    .fixed_decimals(2)
                    .clamping(egui::SliderClamping::Edits)
                    .custom_parser(aurora_i18n::parse_number);
                if ui.add(slider).changed() {
                    l.exposure = Some(value);
                }
                if widgets::icon_button(
                    ui,
                    p,
                    "restore",
                    aurora_i18n::tr!("photo-light-exposure-reset"),
                    l.exposure.is_some(),
                ) {
                    l.exposure = None;
                }
            })
        })
        .response
        .on_disabled_hover_text(aurora_i18n::tr!("ui-options-needs-hdr-emissive"));
    });
    row(
        ui,
        p,
        aurora_i18n::tr!("photo-light-pause-clouds"),
        Some(aurora_i18n::tr!("photo-light-pause-clouds-hint")),
        |ui| {
            widgets::switch(ui, p, &mut l.clouds_paused);
        },
    );
    row(
        ui,
        p,
        aurora_i18n::tr!("photo-light-attached"),
        Some(aurora_i18n::tr!("photo-light-attached-hint")),
        |ui| {
            widgets::switch(ui, p, &mut l.attached_lights);
        },
    );
    ui.horizontal(|ui| {
        let keep = !l.env.is_empty() && !env.loading && !env.locked;
        if ui
            .add_enabled_ui(keep, |ui| widgets::flat_button(ui, p, aurora_i18n::tr!("photo-light-keep")))
            .inner
            .on_hover_text(aurora_i18n::tr!("photo-light-keep-tip"))
            .on_disabled_hover_text(aurora_i18n::tr!("photo-light-keep-tip"))
            .clicked()
        {
            out.push(Action::KeepLight);
        }
        if widgets::flat_button(ui, p, aurora_i18n::tr!("common-personal-lighting")).clicked() {
            out.push(Action::PersonalLighting);
        }
    });
}

/// Width of a list's label in the Light section.
const ENV_LABEL_W: f32 = 86.0;

/// The photo's time of day, sky, water, day cycle and sun over the
/// environment shown (`world::eep_photo`); each row's restore button gives
/// the user's own back.
fn env_rows(ui: &mut egui::Ui, p: &Palette, l: &mut crate::snapshot::light::PhotoLight, env: &EnvView) {
    use crate::world::env_select::SettingsKind;
    ui.label(RichText::new(aurora_i18n::tr!("photo-light-time")).color(p.ink));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let times = [
            (None, aurora_i18n::tr!("photo-light-time-current")),
            (Some(1), aurora_i18n::tr!("ui-environment-sunrise")),
            (Some(2), aurora_i18n::tr!("common-midday")),
            (Some(3), aurora_i18n::tr!("ui-environment-lie-down")),
            (Some(4), aurora_i18n::tr!("common-midnight")),
        ];
        for (time, label) in times {
            if chip(ui, p, label, None, l.env.time == time, true).clicked() {
                l.env.set_time(time);
            }
        }
    });
    ui.add_space(2.0);
    let shown = l.env.shown(env.own);
    let lists = [
        (
            SettingsKind::Sky,
            aurora_i18n::tr!("photo-light-sky"),
            aurora_i18n::tr!("ui-environment-sky-settings"),
            shown.sky,
        ),
        (
            SettingsKind::Water,
            aurora_i18n::tr!("common-water"),
            aurora_i18n::tr!("ui-environment-water-settings"),
            shown.water,
        ),
        (
            SettingsKind::Day,
            aurora_i18n::tr!("photo-light-day"),
            aurora_i18n::tr!("ui-environment-day-cycle-settings"),
            shown.day,
        ),
    ];
    for (kind, label, tip, shown) in lists {
        ui.horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ENV_LABEL_W, 20.0), Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(12.5),
                p.ink,
            );
            resp.on_hover_text(tip);
            // the two arrows and the restore button after the list
            let gap = ui.spacing().item_spacing.x;
            let width = (ui.available_width() - 3.0 * (20.0 + gap) - 12.0).max(60.0);
            if let Some(id) = super::environment::settings_list(ui, p, env.catalog, kind, shown, ("photo_env_list", kind as u8), width) {
                l.env.pick(kind, Some(id));
            }
            let own = l.env.of(kind).is_some() || (kind == SettingsKind::Sky && l.env.time.is_some());
            if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("photo-light-own"), own) {
                if kind == SettingsKind::Sky {
                    l.env.set_time(None);
                }
                l.env.pick(kind, None);
            }
        });
    }
    // the sun: LLFloaterEnvironmentAdjust's azimuth and elevation, over any sky
    let (mut az, mut el) = l.env.sun.map_or(env.sun, super::environment::azimuth_elevation);
    let mut moved = false;
    for (label, value, range) in [
        (aurora_i18n::tr!("photo-light-sun-azimuth"), &mut az, 0.0..=359.99),
        (aurora_i18n::tr!("photo-light-sun-elevation"), &mut el, -90.0..=90.0),
    ] {
        let reset = range.start() < &0.0;
        row(ui, p, label, None, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 100.0).max(40.0);
                // (shown, the sky's sun must not turn into a fixed one)
                let slider = egui::Slider::new(value, range)
                    .fixed_decimals(0)
                    .suffix("°")
                    .clamping(egui::SliderClamping::Edits)
                    .custom_parser(aurora_i18n::parse_number);
                moved |= ui.add(slider).changed();
                // one restore button for the sun, on its second row
                if reset && widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("photo-light-sun-reset"), l.env.sun.is_some()) {
                    l.env.sun = None;
                }
            });
        });
    }
    if moved {
        l.env.sun = Some(super::environment::rotation_from(az, el));
    }
}

/// The full-frame strip's depth of field: its switch and the aperture.
fn strip_aperture(ui: &mut egui::Ui, p: &Palette, d: &mut crate::dof::PhotoDof) {
    if widgets::icon_toggle(ui, p, "focus-centered", aurora_i18n::tr!("photo-section-dof"), d.enabled) {
        d.enabled = !d.enabled;
    }
    ui.add_enabled_ui(d.enabled, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("photo-dof-aperture")).size(12.0).color(p.muted));
        ui.add(
            egui::DragValue::new(&mut d.f_number)
                .range(1.0..=22.0)
                .speed(0.05)
                .prefix("f/")
                .fixed_decimals(1),
        );
    });
}

/// The profile feed's and the postcard's sizes (panel_snapshot_profile.xml's
/// and panel_snapshot_postcard.xml's combos): long sides, or the window.
fn long_sides(ui: &mut egui::Ui, p: &Palette, current: &mut ProfileSize, shown: (u32, u32)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for size in ProfileSize::ALL {
            let label = match size.side() {
                Some(side) => side.to_string(),
                None => aurora_i18n::tr!("photo-resolution-window").to_string(),
            };
            if chip(ui, p, &label, None, *current == size, true).clicked() {
                *current = size;
            }
        }
    });
    let (w, h) = shown;
    ui.label(RichText::new(aurora_i18n::tr!("photo-size", width = w, height = h)).color(p.ink));
}

/// The inventory's sizes (panel_snapshot_inventory.xml's combo) and the
/// texture the photo becomes.
fn texture_sizes(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, inv: &InventoryView) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for t in TextureSize::ALL {
            let label = match t.side() {
                Some(side) => side.to_string(),
                None => aurora_i18n::tr!("photo-resolution-window").to_string(),
            };
            if chip(ui, p, &label, None, s.inventory_size == t, true).clicked() {
                s.inventory_size = t;
            }
        }
    });
    let (w, h) = inv.texture;
    ui.label(RichText::new(aurora_i18n::tr!("photo-texture-size", width = w, height = h)).color(p.ink));
    if w != h {
        ui.label(RichText::new(aurora_i18n::tr!("photo-texture-hint")).size(11.5).color(p.muted));
    }
}

/// What a change of the settings asks: a new preview when the photo
/// changes, a save otherwise.
fn changed(s: &Settings, before: &Settings, out: &mut Vec<Action>) {
    if s != before {
        let photo_changed = s.aspect != before.aspect
            || s.resolution != before.resolution
            || s.custom != before.custom
            || s.inventory_size != before.inventory_size
            || s.profile_size != before.profile_size
            || s.dof != before.dof
            || s.interface != before.interface
            || s.huds != before.huds
            || s.looks != before.looks
            || s.look_settings != before.look_settings;
        out.push(if photo_changed { Action::Changed } else { Action::Saved });
    }
}

/// The photo held by "Freeze frame", over the whole world, in its
/// proportions.
pub fn frozen(ctx: &egui::Context, p: &Palette, texture: &egui::TextureHandle) {
    let screen = ctx.content_rect();
    let size = texture.size_vec2();
    let rect = fit(screen, size.x / size.y.max(1.0));
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("photo_frozen")));
    painter.rect_filled(screen, 0.0, p.navy);
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        widgets::PICTURE_TINT,
    );
}

/// The largest rectangle of `ratio` (width / height) centred in `area`.
pub fn fit(area: Rect, ratio: f32) -> Rect {
    let ratio = ratio.max(0.01);
    let size = if area.width() / area.height().max(1.0) > ratio {
        Vec2::new(area.height() * ratio, area.height())
    } else {
        Vec2::new(area.width(), area.width() / ratio)
    };
    Rect::from_center_size(area.center(), size)
}

/// The parts of `screen` outside `frame` (at most four bands).
pub fn outside(screen: Rect, frame: Rect) -> Vec<Rect> {
    let mut v = Vec::new();
    if frame.left() > screen.left() {
        v.push(Rect::from_min_max(screen.left_top(), Pos2::new(frame.left(), screen.bottom())));
    }
    if frame.right() < screen.right() {
        v.push(Rect::from_min_max(Pos2::new(frame.right(), screen.top()), screen.right_bottom()));
    }
    if frame.top() > screen.top() {
        v.push(Rect::from_min_max(
            Pos2::new(frame.left(), screen.top()),
            Pos2::new(frame.right(), frame.top()),
        ));
    }
    if frame.bottom() < screen.bottom() {
        v.push(Rect::from_min_max(
            Pos2::new(frame.left(), frame.bottom()),
            Pos2::new(frame.right(), screen.bottom()),
        ));
    }
    v
}

/// Icon and name of a guide.
fn guide_face(g: Guide) -> (&'static str, &'static str) {
    match g {
        Guide::Thirds => ("grid-3x3", aurora_i18n::tr!("photo-guide-thirds")),
        Guide::Golden => ("spiral", aurora_i18n::tr!("photo-guide-golden")),
        Guide::Diagonals => ("slash", aurora_i18n::tr!("photo-guide-diagonals")),
        Guide::Center => ("focus-centered", aurora_i18n::tr!("photo-guide-center")),
        Guide::None => ("x", aurora_i18n::tr!("photo-guide-off")),
    }
}

pub(crate) fn guide_lines(painter: &egui::Painter, frame: Rect, guide: Guide, color: Color32) {
    let at = |v: [f32; 2]| Pos2::new(frame.left() + v[0] * frame.width(), frame.top() + v[1] * frame.height());
    for [a, b] in guide.lines() {
        painter.line_segment([at(a), at(b)], Stroke::new(1.0, color));
    }
}

/// Preview with its guide, size badge and refresh button; true when the
/// refresh was clicked.
fn preview(ui: &mut egui::Ui, p: &Palette, s: &Settings, view: &View) -> bool {
    let width = ui.available_width();
    let ratio = view.photo.0 as f32 / view.photo.1.max(1) as f32;
    let height = (width / ratio).min(300.0);
    let (outer, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let rect = fit(outer, ratio);
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(outer, radius, p.field);
    match view.preview {
        Some(t) => {
            ui.painter().image(
                t.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                widgets::PICTURE_TINT,
            );
        }
        None => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                aurora_i18n::tr!("photo-preview-pending"),
                egui::FontId::proportional(12.0),
                p.muted,
            );
        }
    }
    guide_lines(ui.painter(), rect, s.guide, p.ink.gamma_multiply(0.55));
    // the size badge
    let badge = match (&view.email, &view.inventory, &view.profile) {
        (Some(mail), _, _) => format!("{} × {} · JPEG", mail.size.0, mail.size.1),
        (None, _, Some(prof)) => format!("{} × {} · PNG", prof.size.0, prof.size.1),
        (None, Some(inv), None) => aurora_i18n::tr!("photo-texture-size", width = inv.texture.0, height = inv.texture.1).to_string(),
        (None, None, None) => format!("{} × {} · {}", view.photo.0, view.photo.1, s.format.label()),
    };
    let galley = ui.painter().layout_no_wrap(badge, egui::FontId::proportional(11.0), p.ink);
    let badge_rect = Rect::from_min_size(
        rect.right_bottom() - galley.size() - Vec2::new(14.0, 10.0),
        galley.size() + Vec2::new(10.0, 4.0),
    );
    ui.painter().rect_filled(badge_rect, radius, p.navy.gamma_multiply(0.72));
    ui.painter().galley(badge_rect.min + Vec2::new(5.0, 2.0), galley, p.ink);
    // the refresh button
    let button = Rect::from_min_size(rect.left_bottom() + Vec2::new(6.0, -28.0), Vec2::splat(22.0));
    let resp = ui.interact(button, ui.scope_id().with("photo_refresh"), Sense::click());
    let fill = if resp.hovered() { p.raised } else { p.navy.gamma_multiply(0.72) };
    ui.painter().rect_filled(button, radius, fill);
    if let Some(t) = super::icons::global("refresh") {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(button.center(), Vec2::splat(14.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            p.ink,
        );
    }
    resp.on_hover_text(aurora_i18n::tr!("photo-refresh")).clicked()
}

/// The latest photos and "Open the folder".
fn gallery(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Vec<Action>) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let radius = ui.visuals().window_corner_radius;
        for (path, tex) in view.gallery {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(54.0, 34.0), Sense::click());
            ui.painter().rect_filled(rect, radius, p.field);
            if let Some(t) = tex {
                let size = t.size_vec2();
                // the middle of the photo, filling the thumbnail
                let r = size.x / size.y.max(1.0);
                let want = rect.width() / rect.height();
                let uv = if r > want {
                    let w = want / r;
                    Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0))
                } else {
                    let h = r / want;
                    Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0))
                };
                ui.painter().image(t.id(), rect, uv, widgets::PICTURE_TINT);
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if resp.on_hover_text(name).clicked() {
                out.push(Action::OpenPhoto(path.clone()));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("photo-open-folder"),
                ActionKind::Flat,
                Vec2::new(0.0, 26.0),
            )
            .clicked()
            {
                out.push(Action::OpenFolder);
            }
        });
    });
}

/// The focal length slider and presets (mm, full frame) on the camera's
/// field of view; true when it changed.
fn focal(ui: &mut egui::Ui, p: &Palette, fov: &mut f32, default_fov: f32) -> bool {
    let mut changed = false;
    let mut mm = crate::snapshot::focal_of_fov(*fov).clamp(*FOCAL_RANGE.start(), *FOCAL_RANGE.end());
    ui.horizontal(|ui| {
        ui.label(RichText::new(aurora_i18n::tr!("photo-focal")).color(p.ink))
            .on_hover_text(aurora_i18n::tr!("photo-focal-hint"));
        ui.spacing_mut().slider_width = 110.0;
        if ui
            .add(
                // clamped and rounded only when the user moves it: egui's
                // default rounds the shown value back into `mm` (60° is
                // 20.8 mm, shown 21), which changed the view on opening
                egui::Slider::new(&mut mm, FOCAL_RANGE)
                    .logarithmic(true)
                    .suffix(" mm")
                    .fixed_decimals(0)
                    .clamping(egui::SliderClamping::Edits),
            )
            .changed()
        {
            *fov = crate::snapshot::fov_of_focal(mm);
            changed = true;
        }
        if widgets::icon_button(
            ui,
            p,
            "restore",
            aurora_i18n::tr!("photo-focal-reset"),
            (*fov - default_fov).abs() > 1e-3,
        ) {
            *fov = default_fov;
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for preset in FOCAL_PRESETS {
            let on = (mm - preset).abs() < 0.5;
            if chip(ui, p, &format!("{preset:.0}"), None, on, true).clicked() {
                *fov = crate::snapshot::fov_of_focal(preset);
                changed = true;
            }
        }
    });
    changed
}

/// Under the free camera's switch: the avatar stays, and what brings the
/// camera back, its key drawn as a keycap in the sentence.
fn free_camera_hint(ui: &mut egui::Ui, p: &Palette, key: Option<&crate::keybinds::Binding>) {
    const MARK: &str = "\u{1}";
    let text = aurora_i18n::tr!("photo-free-camera-hint", key = MARK);
    let (before, after) = text.split_once(MARK).unwrap_or((text.as_str(), ""));
    let style = |t: &str| RichText::new(t).size(11.0).color(p.muted);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(style(before.trim_end()));
        if let Some(b) = key {
            let caps = widgets::keycap::caps(ui, b, p.panel);
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(widgets::keycap::row_width(&caps), widgets::keycap::HEIGHT),
                Sense::hover(),
            );
            widgets::keycap::paint_row(ui, p, &caps, egui::pos2(rect.left(), rect.center().y), p.panel);
        }
        ui.label(style(after.trim_start()));
    });
}

fn section(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icon_image(ui, icon, 14.0, p.violet_light);
        ui.label(RichText::new(title).size(12.5).strong().color(p.ink));
    });
}

fn icon_image(ui: &mut egui::Ui, icon: &str, side: f32, tint: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    if let Some(t) = super::icons::global(icon) {
        ui.painter()
            .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
    }
}

/// A setting: its label (and a hint under it) on the left, its control on
/// the right.
fn row(ui: &mut egui::Ui, p: &Palette, label: &str, hint: Option<&str>, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let width = ui.available_width();
        ui.vertical(|ui| {
            ui.set_width(width * 0.55);
            ui.label(RichText::new(label).color(p.ink));
            if let Some(h) = hint {
                ui.add(egui::Label::new(RichText::new(h).size(11.0).color(p.muted)).wrap());
            }
        });
        // (a long hint: the control stays next to its label)
        let align = if hint.is_some() { egui::Align::Min } else { egui::Align::Center };
        ui.with_layout(egui::Layout::right_to_left(align), control);
    });
}

/// A choice among several shown side by side: the accent when chosen.
pub(crate) fn chip(ui: &mut egui::Ui, p: &Palette, label: &str, sub: Option<&str>, on: bool, enabled: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let main = ui.painter().layout_no_wrap(label.to_owned(), font, p.ink);
    let sub = sub.map(|t| ui.painter().layout_no_wrap(t.to_owned(), egui::FontId::proportional(10.0), p.ink));
    let width = main.size().x + sub.as_ref().map_or(0.0, |g| g.size().x + 4.0) + 16.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = match (on, resp.hovered() && enabled) {
        (true, _) => p.violet,
        (false, true) => p.raised,
        (false, false) => p.bar,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let text = if on {
        p.on_accent
    } else if enabled {
        p.muted
    } else {
        p.muted_dim
    };
    let mut x = rect.left() + 8.0;
    let main_w = main.size().x;
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &main, x, rect.center().y);
    ui.painter().galley(at, main, if on { text } else { p.ink });
    x += main_w + 4.0;
    if let Some(g) = sub {
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, x, rect.center().y);
        ui.painter().galley(at, g, text.gamma_multiply(0.8));
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, on, label));
    resp
}

/// The round shutter button of the full-frame strip.
fn shutter(ui: &mut egui::Ui, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
    let c = rect.center();
    ui.painter().circle_filled(c, 21.0, p.violet.gamma_multiply(0.35));
    ui.painter()
        .circle_filled(c, 18.0, if resp.hovered() { p.violet_light } else { p.violet });
    if let Some(t) = super::icons::global("camera-filled") {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(c, Vec2::splat(20.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            p.on_accent,
        );
    }
    resp.on_hover_text(aurora_i18n::tr!("photo-take"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_photo_fits_the_screen_and_the_rest_is_masked() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        let frame = fit(screen, 4.0 / 5.0);
        assert!((frame.height() - 900.0).abs() < 0.01);
        assert!((frame.width() - 720.0).abs() < 0.01);
        assert!((frame.center().x - 800.0).abs() < 0.01);
        let bands = outside(screen, frame);
        assert_eq!(bands.len(), 2);
        let masked: f32 = bands.iter().map(|r| r.area()).sum();
        assert!((masked + frame.area() - screen.area()).abs() < 1.0);
        // the screen's own proportions: nothing masked
        assert!(outside(screen, fit(screen, 16.0 / 9.0)).is_empty());
        // wider than the screen: bands above and below
        let wide = fit(screen, 3.0);
        assert_eq!(outside(screen, wide).len(), 2);
    }

    #[test]
    fn showing_the_focal_length_leaves_the_field_of_view_alone() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        // 60° is 20.8 mm: the slider shows 21 mm, and must not write it back
        for start in [crate::camera::DEFAULT_FOV, 0.7, 1.31] {
            let mut fov = start;
            let mut changed = false;
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    changed |= focal(ui, &p, &mut fov, crate::camera::DEFAULT_FOV)
                });
                output.textures_delta.clear();
            }
            assert!(!changed, "opening the window changed the focal length");
            assert_eq!(fov, start);
        }
        // the sky's exposure (1.234 shown 1.23) and its sun (shown in whole
        // degrees) are not taken over either
        let mut light = crate::snapshot::light::PhotoLight::default();
        let catalog = crate::world::env_select::SettingsCatalog::default();
        let env = EnvView {
            catalog: &catalog,
            own: crate::world::eep_env::LocalView::SHARED,
            sun: (123.456, 21.7),
            locked: false,
            loading: false,
        };
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                light_rows(ui, &p, &mut light, 1.234, true, &env, &mut Vec::new())
            });
            output.textures_delta.clear();
        }
        assert_eq!(light.exposure, None);
        assert!(light.env.is_empty(), "showing the Light section picked {:?}", light.env);
    }
}
