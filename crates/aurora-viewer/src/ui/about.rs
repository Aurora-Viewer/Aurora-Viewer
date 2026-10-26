//! Help › About Aurora Viewer. Port of LLFloaterAbout
//! (indra/newview/llfloaterabout.cpp, floater_about.xml, the About* strings
//! of strings.xml, originally LGPL 2.1): the version, then the system
//! information with "Copy to clipboard", and the licences. Redrawn in the
//! Aurora look: a banner with the logo and the version, the information in
//! cards (a click on a value copies it alone), the licences read from
//! NOTICE.md (`about/notice.rs`). The copy stays in English whatever the
//! interface language, as the Legacy Viewer's (strings.xml: "this info
//! sometimes gets sent to support"). The Legacy Viewer's credits tabs name
//! its own teams: Aurora names the authors of the code it follows in the
//! licences, as NOTICE.md does.

pub mod notice;

use super::sheet::{self, Tone};
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use egui::{Rect, RichText, Sense, Shape, Vec2};
use notice::Block;
use std::time::{Duration, Instant};

/// The window's id.
pub const WINDOW: &str = "about";

/// "Copied" stays this long on the button and beside a value.
const COPIED_FOR: Duration = Duration::from_millis(1600);
/// The logo in the banner, and the width of the information's keys.
const LOGO: f32 = 72.0;
const KEY_W: f32 = 138.0;

/// A value as read, or a key of the interface (translated on screen,
/// English in the copy).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Key(&'static str),
}

impl Value {
    fn shown(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Key(k) => aurora_i18n::text(k).to_owned(),
        }
    }

    fn english(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Key(k) => aurora_i18n::text_in("en", k).to_owned(),
        }
    }
}

/// One line of information: the key of its label, and its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: &'static str,
    pub value: Value,
}

impl Row {
    pub fn text(label: &'static str, value: impl Into<String>) -> Row {
        Row {
            label,
            value: Value::Text(value.into()),
        }
    }

    pub fn key(label: &'static str, value: &'static str) -> Row {
        Row {
            label,
            value: Value::Key(value),
        }
    }
}

/// A card of the Info tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub icon: &'static str,
    /// The key of its title.
    pub title: &'static str,
    pub rows: Vec<Row>,
}

/// What the window shows, built by the app while it is open.
pub struct AboutView<'a> {
    pub sections: Vec<Section>,
    pub logo: Option<&'a egui::TextureHandle>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Info,
    Licences,
}

#[derive(Default)]
pub struct AboutUi {
    pub tab: Tab,
    copied_at: Option<Instant>,
    /// The value copied alone last (its section and row).
    value_copied: Option<(usize, usize, Instant)>,
    /// Scroll the tab to its end at the next frame (AURORA_DEMO_ABOUT's
    /// `:end`, to capture the last cards).
    pub scroll_to_end: bool,
}

/// The release page of this version, for a published version only (a
/// build between two tags has none).
pub fn release_notes(version: &str) -> Option<String> {
    let published = !version.is_empty() && !version.contains('+') && version != "0.0.0";
    published.then(|| format!("{}/releases/tag/v{version}", crate::links::GITHUB))
}

/// The text "Copy to clipboard" gives: the version, its release page, then
/// a block per card, a line per value (LLFloaterAbout::getInfo's order,
/// its English labels).
pub fn copy_text(sections: &[Section]) -> String {
    let version = aurora_version::version();
    let mut out = format!("Aurora Viewer {version}\n");
    if let Some(url) = release_notes(version) {
        out.push_str(&format!("Release notes: {url}\n"));
    }
    for section in sections {
        out.push('\n');
        for row in &section.rows {
            out.push_str(&format!("{}: {}\n", aurora_i18n::text_in("en", row.label), row.value.english()));
        }
    }
    out
}

impl AboutUi {
    /// Open the window on `tab`.
    pub fn open(&mut self, ctx: &egui::Context, tab: Tab, open: &mut bool) {
        self.tab = tab;
        widgets::open_floater(ctx, WINDOW, open);
    }

    /// Draw the window while `open`.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, view: &AboutView, open: &mut bool) {
        if !*open {
            return;
        }
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("ui-bars-about"),
            egui::pos2(360.0, 110.0),
            Vec2::new(600.0, 720.0),
        );
        floater.min_size = Vec2::new(440.0, 420.0);
        floater.show(ctx, p, open, |ui| {
            hero(ui, p, view.logo);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let tabs = [
                    (Tab::Info, "info-circle-filled", aurora_i18n::tr!("about-tab-info")),
                    (Tab::Licences, "license", aurora_i18n::tr!("about-tab-licences")),
                ];
                for (tab, icon, label) in tabs {
                    if widgets::pill_tab(ui, p, icon, label, None, false, self.tab == tab, true).clicked() {
                        self.tab = tab;
                    }
                }
            });
            ui.add_space(8.0);
            match self.tab {
                Tab::Info => self.info(ui, p, &view.sections),
                Tab::Licences => licences(ui, p, std::mem::take(&mut self.scroll_to_end)),
            }
        });
    }

    fn info(&mut self, ui: &mut egui::Ui, p: &Palette, sections: &[Section]) {
        let now = Instant::now();
        let (mut body, foot) = widgets::above_footer(ui);
        egui::ScrollArea::vertical()
            .id_salt("about_info")
            .auto_shrink([false, false])
            .show(&mut body, |ui| {
                for (s, section) in sections.iter().enumerate() {
                    let mut card = sheet::card_begin(ui);
                    let c = &mut card.child;
                    sheet::card_title(c, p, section.icon, aurora_i18n::text(section.title), None);
                    c.add_space(4.0);
                    for (r, row) in section.rows.iter().enumerate() {
                        let copied = self
                            .value_copied
                            .is_some_and(|(cs, cr, at)| cs == s && cr == r && now.duration_since(at) < COPIED_FOR);
                        if value_row(c, p, aurora_i18n::text(row.label), &row.value.shown(), copied) {
                            c.ctx().copy_text(row.value.english());
                            self.value_copied = Some((s, r, now));
                        }
                        c.add_space(5.0);
                    }
                    sheet::card_end(ui, p, card);
                }
                if std::mem::take(&mut self.scroll_to_end) {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        foot.show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let copied = self.copied_at.is_some_and(|at| now.duration_since(at) < COPIED_FOR);
                let (icon, label) = if copied {
                    ("check", aurora_i18n::tr!("about-copied"))
                } else {
                    ("copy-filled", aurora_i18n::tr!("about-copy"))
                };
                let button = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = widgets::action_button(ui, p, icon, label, ActionKind::Primary, Vec2::new(0.0, 30.0));
                    ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("about-copy-hint")).size(11.5).color(p.muted)).wrap());
                    button
                });
                if button.inner.clicked() {
                    ui.ctx().copy_text(copy_text(sections));
                    self.copied_at = Some(now);
                }
            });
        });
        // the "Copied" marks go away on their own
        let marks = [self.copied_at, self.value_copied.map(|(_, _, at)| at)];
        if let Some(left) = marks
            .into_iter()
            .flatten()
            .filter_map(|at| COPIED_FOR.checked_sub(now.duration_since(at)))
            .min()
        {
            ui.ctx().request_repaint_after(left);
        }
    }
}

/// The banner: the logo in its violet light, the name, the version and the
/// build, and the project's links.
fn hero(ui: &mut egui::Ui, p: &Palette, logo: Option<&egui::TextureHandle>) {
    const PAD: Vec2 = Vec2::new(18.0, 16.0);
    let width = ui.available_width();
    let top = ui.cursor().min;
    // the face and the light go under what is laid out first
    let face = ui.painter().add(Shape::Noop);
    let light = ui.painter().add(Shape::Noop);
    let inner = Rect::from_min_size(top + PAD, Vec2::new(width - PAD.x * 2.0, 10_000.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let mut logo_rect = None;
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        if let Some(logo) = logo {
            logo_rect = Some(ui.add(egui::Image::new(logo).fit_to_exact_size(Vec2::splat(LOGO))).rect);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(sheet::semibold(ui, "Aurora Viewer", 22.0, p.ink));
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-login-second-life-client-rust-vulkan"))
                    .size(12.0)
                    .color(p.violet_pale),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                sheet::pill(ui, p, Tone::Group, "tag-filled", aurora_version::version());
                let build = aurora_i18n::tr!("about-build-pill", profile = build_label());
                sheet::pill(ui, p, Tone::Neutral, "", &build);
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                if let Some(url) = release_notes(aurora_version::version()) {
                    link(ui, p, "file-text-filled", aurora_i18n::tr!("about-release-notes"), &url);
                }
                link(ui, p, "brand-github-filled", "GitHub", crate::links::GITHUB);
                link(ui, p, "world-filled", aurora_i18n::tr!("ui-login-website"), crate::links::WEBSITE);
            });
        });
    });
    let rect = Rect::from_min_size(top, Vec2::new(width, child.min_rect().height() + PAD.y * 2.0));
    ui.painter()
        .set(face, Shape::rect_filled(rect, ui.visuals().window_corner_radius, p.field));
    if let Some(logo) = logo_rect {
        let glow = crate::ui::aurora_bg::radial_glow_shape(logo.center(), LOGO * 1.6, p.violet, 0.38);
        // the light stays in the banner: a shape set keeps the clip of the
        // painter that sets it
        ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).set(light, glow);
    }
    ui.allocate_rect(rect, Sense::hover());
}

/// The build's profile as the window title names it ("Dev"), else its
/// target folder.
pub fn build_label() -> &'static str {
    let dir = env!("AURORA_BUILD_PROFILE");
    crate::cli::profile_label(dir).unwrap_or(dir)
}

/// A link of the banner: a flat button that opens `url` in the browser.
fn link(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, url: &str) {
    let resp = widgets::action_button(ui, p, icon, label, ActionKind::Flat, Vec2::new(0.0, 26.0)).on_hover_text(url);
    if resp.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

/// A line of information; true when its value was clicked (to copy it).
fn value_row(ui: &mut egui::Ui, p: &Palette, key: &str, value: &str, copied: bool) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.allocate_ui_with_layout(Vec2::new(KEY_W, 16.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.set_min_width(KEY_W);
            ui.add(egui::Label::new(RichText::new(key).size(12.5).color(p.muted)).truncate());
        });
        let room = (ui.available_width() - 20.0).max(40.0);
        let resp = ui
            .allocate_ui_with_layout(Vec2::new(room, 16.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(value).size(12.5).color(p.ink))
                        .wrap()
                        .sense(Sense::click()),
                )
            })
            .inner;
        if copied {
            widgets::glyph(ui, "check", p.success, 13.0);
        } else if resp.hovered() {
            widgets::glyph(ui, "copy-filled", p.muted_dim, 13.0);
        }
        let tip = if copied {
            aurora_i18n::tr!("about-copied")
        } else {
            aurora_i18n::tr!("about-click-to-copy")
        };
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
    })
    .inner
}

/// The Licences tab: Aurora's own licence and its links, then NOTICE.md, a
/// card per heading.
fn licences(ui: &mut egui::Ui, p: &Palette, to_end: bool) {
    egui::ScrollArea::vertical()
        .id_salt("about_licences")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut card = sheet::card_begin(ui);
            let c = &mut card.child;
            sheet::card_title(c, p, "license", aurora_i18n::tr!("about-licence-title"), None);
            c.add_space(6.0);
            c.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                link(ui, p, "file-text-filled", aurora_i18n::tr!("about-licence-gpl"), GPL);
                link(
                    ui,
                    p,
                    "brand-github-filled",
                    aurora_i18n::tr!("about-source-code"),
                    crate::links::GITHUB,
                );
            });
            c.add_space(8.0);
            let mut first = true;
            for block in notice::blocks() {
                match block {
                    Block::Heading(title) => {
                        sheet::card_end(ui, p, card);
                        card = sheet::card_begin(ui);
                        sheet::card_title(&mut card.child, p, heading_icon(title), title, None);
                        card.child.add_space(4.0);
                        first = true;
                    }
                    Block::Paragraph(text) | Block::Item(text) => {
                        let c = &mut card.child;
                        if !first {
                            c.add_space(6.0);
                        }
                        let bullet = matches!(block, Block::Item(_));
                        c.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if bullet {
                                widgets::glyph(ui, "point-filled", p.violet_light, 10.0);
                            }
                            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.ink)).wrap());
                        });
                        first = false;
                    }
                    Block::Row { work, licence } => {
                        let c = &mut card.child;
                        c.add_space(if first { 2.0 } else { 9.0 });
                        c.add(egui::Label::new(RichText::new(work).size(12.0).color(p.ink)).wrap());
                        c.add_space(2.0);
                        c.add(egui::Label::new(sheet::semibold(c, licence, 11.5, p.violet_light)).wrap());
                        first = false;
                    }
                }
            }
            sheet::card_end(ui, p, card);
            if to_end {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });
}

/// The GNU GPL version 3, Aurora's licence.
const GPL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

/// The icon of a NOTICE.md heading's card.
fn heading_icon(title: &str) -> &'static str {
    let t = title.to_ascii_lowercase();
    if t.contains("plugin") {
        "player-play-filled"
    } else if t.contains("asset") {
        "package"
    } else {
        "code"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_notes_only_for_a_published_version() {
        assert_eq!(
            release_notes("0.7.0").as_deref(),
            Some("https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.7.0")
        );
        assert_eq!(release_notes("0.7.0+3.gabcdef123"), None);
        assert_eq!(release_notes("0.0.0"), None);
    }

    #[test]
    fn the_copy_is_in_english_a_block_per_card() {
        let sections = vec![
            Section {
                icon: "device-desktop-filled",
                title: "about-section-computer",
                rows: vec![Row::text("about-cpu", "Some CPU"), Row::key("about-memory", "about-reading")],
            },
            Section {
                icon: "map-pin-filled",
                title: "about-section-location",
                rows: vec![Row::key("about-region", "about-hidden-rlva")],
            },
        ];
        let text = copy_text(&sections);
        let blocks: Vec<&str> = text.split("\n\n").collect();
        assert!(blocks[0].starts_with(&format!("Aurora Viewer {}", aurora_version::version())));
        assert_eq!(
            blocks[1],
            format!(
                "{}: Some CPU\n{}: {}",
                aurora_i18n::text_in("en", "about-cpu"),
                aurora_i18n::text_in("en", "about-memory"),
                aurora_i18n::text_in("en", "about-reading")
            )
        );
        assert_eq!(
            blocks[2],
            format!(
                "{}: {}\n",
                aurora_i18n::text_in("en", "about-region"),
                aurora_i18n::text_in("en", "about-hidden-rlva")
            )
        );
    }
}
