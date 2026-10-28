//! Login screen: animated aurora background, brand block and the login
//! card in the middle, version and news panels on the right, community
//! links at the bottom left. Cards and buttons use the surfaces of the
//! in-world windows (`widgets::overlay_frame`, `widgets::action_button`);
//! their blur needs a world behind, the aurora is soft enough without it.

use super::icons::Icons;
use super::news::{LoginInfo, VersionState};
use super::widgets::{self, ActionKind};
use crate::settings::{GridChoice, Settings};
use crate::theme::Palette;
use egui::{Color32, Frame, Margin, RichText, Sense, Stroke, TextEdit, Vec2};
mod carousel;
mod chrome;
mod credentials_form;
mod derez;
mod driver_notice;
mod fade;
mod layout;
pub(crate) mod manager;
mod manager_drag;
pub(crate) mod motion;
pub mod safe_graphics_notice;

const REMEMBERED_PASSWORD_MASK: &str = "••••••••••••••••";

#[derive(Default)]
pub struct LoginForm {
    pub accounts: Vec<crate::login_accounts::Account>,
    pub accounts_loaded: bool,
    pub remember_checked: bool,
    pub remember_mfa: bool,
    pub mfa_authorization: Option<crate::credentials::MfaAuthorization>,
    pub mfa_for: Option<(String, String)>,
    pub content_height: f32,
    pub editing: bool,
    pub adding: bool,
    pub add_return: Option<(String, String)>,
    pub places: crate::login_places::Cache,
    pub managing: bool,
    pub show_info: bool,
    pub page: usize,
    pub selection_shown_for: Option<(String, String)>,
    pub pending_connect: Option<usize>,
    pub pending_since: Option<f64>,
    pub wheel_scroll: f32,
    pub wheel_time: Option<f64>,
    pub motion: motion::State,
    pub manager: manager::State,
    pub transition: Option<derez::Transition>,
    pub portraits: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    pub portrait_rx: Option<crossbeam_channel::Receiver<(uuid::Uuid, egui::ColorImage)>>,
    pub password: String,
    pub mfa_token: String,
    pub show_mfa: bool,
    pub tos_message: Option<String>,
    pub agree_tos: bool,
    pub error: Option<String>,
    /// The session ended with its region's restart: the login button reads
    /// "Reconnect" (region_restart.rs).
    pub reconnect: bool,
    pub busy: bool,
    /// A visible reason keeps the button and all login paths disabled together.
    pub block_reason: Option<&'static str>,
    /// Password shown in clear (eye button).
    pub show_password: bool,
    /// The first field to fill already has the keyboard focus (set again
    /// when the login screen comes back).
    pub focused: bool,
    /// Remembered login hash for the shown user (never displayed).
    pub stored: Option<String>,
    /// Grid and user name `stored` was read for.
    pub stored_for: Option<(String, String)>,
    /// A place given from outside (command line, a link of a web page):
    /// where the next login starts, whatever the account's choice, shown in
    /// the start menus (FSPanelLogin::onUpdateStartSLURL). Not saved.
    pub start_slurl: Option<crate::slurl::Location>,
    /// The graphics driver is out of date (`gpu_driver::check`), and the
    /// user has not dismissed the warning for this driver.
    pub driver_notice: Option<crate::gpu_driver::Notice>,
    /// The safe graphics offered after the GPU stopped answering, or the
    /// way back from them (`app/safe_graphics.rs`).
    pub safe_graphics: Option<safe_graphics_notice::SafeGraphics>,
}

#[derive(Default)]
pub enum LoginAction {
    ReportBug,
    /// The bug button's right click, as in the top bar.
    CopyVersion,
    /// "Ignore for this driver" on the out of date driver warning.
    DismissDriverNotice,
    /// A button of the safe graphics card.
    SafeGraphics(safe_graphics_notice::Choice),
    #[default]
    None,
    Login,
    Quit,
    CheckUpdates,
    /// The cogwheel: the preferences window over the login screen.
    Preferences,
    /// The accessibility button beside it: the preferences on their
    /// Accessibility page.
    Accessibility,
}

/// Animated aurora background (also behind the loading screens without a
/// last view).
pub fn draw_background(ui: &egui::Ui, p: &Palette) {
    let ctx = ui.ctx();
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let t = ctx.input(|i| i.time) as f32;
    super::aurora_bg::paint(&painter, p, rect, t, 1.0);
    // slow animation: ~30 images/s are plenty
    ctx.request_repaint_after(std::time::Duration::from_millis(33));
}

fn img(ui: &egui::Ui, icons: &Icons, name: &str, rect: egui::Rect, tint: Color32) {
    if let Some(t) = icons.get(name) {
        ui.painter().image(
            t.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// Text field with a leading icon (and an optional eye toggle for passwords).
#[allow(clippy::too_many_arguments)]
fn field(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    id: &str,
    icon: &str,
    hint: &str,
    text: &mut String,
    password: Option<&mut bool>,
) -> egui::Response {
    let id = egui::Id::unique(id);
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = if focused {
        Stroke::new(1.0, p.violet)
    } else {
        Stroke::new(1.0, p.muted_dim.gamma_multiply(0.65))
    };
    Frame::new()
        .fill(p.field)
        .stroke(stroke)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                img(ui, icons, icon, r, if focused { p.violet_light } else { p.muted });
                let eye_w = if password.is_some() { 26.0 } else { 0.0 };
                let w = ui.available_width() - eye_w;
                let remembered = password.is_some() && text.is_empty() && !hint.is_empty();
                if remembered {
                    // The saved-password marker should look filled, not like a placeholder.
                    ui.visuals_mut().weak_text_color = Some(p.ink);
                }
                let hidden = remembered || password.as_deref().is_some_and(|show| !*show);
                let resp = ui.add(
                    TextEdit::singleline(text)
                        .id(id)
                        .interactive(ui.is_enabled())
                        .frame(
                            egui::Frame::NONE
                                .inner_margin(crate::ui::widgets::field_text_margin(ui.ctx(), &egui::FontId::proportional(14.0))),
                        )
                        .password(hidden)
                        .hint_text(RichText::new(hint).color(p.muted_dim))
                        .font(egui::FontId::proportional(14.0))
                        .text_color(p.ink)
                        .desired_width(w),
                );
                if let Some(show) = password {
                    let (r, eye) = ui.allocate_exact_size(Vec2::splat(18.0), if remembered { Sense::hover() } else { Sense::click() });
                    let col = if !remembered && eye.hovered() { p.ink } else { p.muted };
                    img(ui, icons, if *show && !remembered { "eye-off" } else { "eye-filled" }, r, col);
                    let tip = if remembered {
                        aurora_i18n::tr!("ui-login-password-remembered-enter-a-new-one-to-replace-it")
                    } else if *show {
                        aurora_i18n::tr!("ui-login-hide")
                    } else {
                        aurora_i18n::tr!("common-show")
                    };
                    if eye.on_hover_text(tip).clicked() && !remembered {
                        *show = !*show;
                    }
                }
                resp
            })
            .inner
        })
        .inner
}

fn caption(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(p.muted));
    ui.add_space(-2.0);
}

/// The start location menu: last location, home or a region; a place given
/// from outside (`slurl`) comes first and stays chosen until another entry
/// is, as the Legacy Viewer's location combo shows the start SLURL
/// (FSPanelLogin::onUpdateStartSLURL).
pub(super) fn start_menu(ui: &mut egui::Ui, combo: egui::ComboBox, start: &mut String, slurl: &mut Option<crate::slurl::Location>) {
    let place = slurl.as_ref().map(crate::external_url::place_label);
    let selected = match (&place, start.as_str()) {
        (Some(place), _) => place.clone(),
        (None, "home") => aurora_i18n::tr!("common-home").to_owned(),
        (None, "region") => aurora_i18n::tr!("ui-login-region").to_owned(),
        _ => aurora_i18n::tr!("ui-login-last-location").to_owned(),
    };
    let shown = combo.selected_text(selected).show_ui(ui, |ui| {
        if let Some(place) = &place {
            let _ = ui.selectable_label(true, place);
        }
        for (value, text) in [
            ("last", aurora_i18n::tr!("ui-login-last-location")),
            ("home", aurora_i18n::tr!("common-home")),
            ("region", aurora_i18n::tr!("ui-login-region")),
        ] {
            if ui.selectable_label(place.is_none() && start == value, text).clicked() {
                *start = value.into();
                *slurl = None;
            }
        }
    });
    // a card's narrow menu cuts the place short
    if let Some(place) = place {
        shown.response.on_hover_text(place);
    }
}

/// Height of the buttons along the bottom of the screen.
const BAR_BUTTON: f32 = 30.0;
/// From the bottom of the screen to these buttons.
const BAR_MARGIN: f32 = 28.0;
/// Kept free between the cards and the line of these buttons, and between
/// the brand and the top of the window.
const BAR_GAP: f32 = 10.0;
/// The brand and the login card stand a little above the middle.
const CENTER_OFFSET: f32 = -10.0;
/// Side of the logo over the login card, and what is left of it in a window
/// too low for the whole block.
const LOGO: f32 = 150.0;
const LOGO_MIN: f32 = 64.0;

/// Community link with its icon and name; `url` empty = not configured.
fn link_button(ui: &mut egui::Ui, p: &Palette, icon: &str, name: &str, url: &str) {
    let enabled = !url.is_empty();
    let resp = ui
        .add_enabled_ui(enabled, |ui| {
            widgets::action_button(ui, p, icon, name, ActionKind::Flat, Vec2::new(0.0, BAR_BUTTON))
        })
        .inner;
    let resp = resp
        .on_hover_text(url)
        .on_disabled_hover_text(aurora_i18n::tr!("ui-login-pending-link", name = name.to_string()));
    if resp.clicked() && enabled {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

/// A strip whose color runs through `stops` (position 0..1, color) from left
/// to right.
fn strip(r: egui::Rect, stops: &[(f32, Color32)]) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    for (at, color) in stops {
        let x = egui::lerp(r.left()..=r.right(), *at);
        mesh.colored_vertex(egui::pos2(x, r.top()), *color);
        mesh.colored_vertex(egui::pos2(x, r.bottom()), *color);
    }
    for i in 0..stops.len().saturating_sub(1) as u32 {
        let n = i * 2;
        mesh.add_triangle(n, n + 2, n + 1);
        mesh.add_triangle(n + 1, n + 2, n + 3);
    }
    egui::Shape::mesh(mesh)
}

/// Card of the login screen: a pane of frosted glass. Under the tint of the
/// in-world windows it shows the sky behind it out of focus
/// (`aurora_bg::frosted`) and two lights of the aurora, which also spill
/// around the card; its top edge catches the light, and the card the screen
/// is for (`accent`) wears the brand line. An opaque skin (panel_alpha =
/// 255) keeps its plain surface.
fn card<R>(ui: &mut egui::Ui, p: &Palette, margin: i8, accent: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    card_hover(ui, p, margin, accent, 0.0, body)
}

fn card_hover<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    margin: i8,
    accent: bool,
    hover: f32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    glass_card(ui, p, margin, accent, hover, None, body)
}

/// A card edged in the caution colour: a warning about the machine itself
/// (`driver_notice.rs`).
fn card_caution<R>(ui: &mut egui::Ui, p: &Palette, margin: i8, body: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    glass_card(ui, p, margin, false, 0.0, Some(p.amber), body)
}

fn glass_card<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    margin: i8,
    accent: bool,
    hover: f32,
    edge: Option<Color32>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let behind = (ui.painter().clone(), ui.painter().add(egui::Shape::Noop));
    let mut pane = None;
    let mut frame = widgets::overlay_frame(ui.ctx(), p);
    frame.stroke.color = edge.unwrap_or_else(|| frame.stroke.color.lerp_to_gamma(p.violet_pale, hover * 0.7));
    let response = frame.inner_margin(Margin::same(margin)).show(ui, |ui| {
        pane = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
        body(ui)
    });
    if let Some((painter, slot)) = pane.filter(|_| p.panel_alpha < 255) {
        let ctx = ui.ctx();
        let outer = response.response.rect;
        // inside the border of the frame
        let rect = outer.shrink(1.0);
        let strength = if accent { 1.0 } else { 0.6 } + hover * 0.45;
        let lights = vec![
            super::aurora_bg::radial_glow_shape(
                outer.left_top() + Vec2::new(outer.width() * 0.18, outer.height() * 0.12),
                outer.width() * 0.95,
                p.violet,
                0.24 * strength,
            ),
            super::aurora_bg::radial_glow_shape(
                outer.right_bottom() - Vec2::new(outer.width() * 0.12, outer.height() * 0.14),
                outer.width() * 0.8,
                p.teal,
                0.13 * strength,
            ),
        ];
        behind.0.set(behind.1, egui::Shape::Vec(lights.clone()));
        let mut shapes = super::aurora_bg::frosted_pane(p, ctx.content_rect(), ctx.input(|i| i.time) as f32, rect, lights);
        let top = |height: f32| egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), height));
        if accent {
            shapes.push(strip(top(2.0), &[(0.0, p.violet), (1.0, p.teal)]));
        } else {
            let lit = p.ink.gamma_multiply(0.30);
            shapes.push(strip(
                top(1.0),
                &[(0.0, Color32::TRANSPARENT), (0.3, lit), (1.0, Color32::TRANSPARENT)],
            ));
        }
        painter
            .with_clip_rect(rect.intersect(painter.clip_rect()))
            .set(slot, egui::Shape::Vec(shapes));
    }
    response
}

fn version_panel(ui: &mut egui::Ui, p: &Palette, icons: &Icons, info: &LoginInfo) -> bool {
    let mut check_updates = false;
    card(ui, p, 16, false, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_header(ui, p, "tag-filled", aurora_i18n::tr!("ui-login-version"));
        let current = aurora_version::version();
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("ui-login-installed")).size(12.5).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // a beta's long number elides beside a longer translated label
                ui.add(egui::Label::new(RichText::new(format!("v{current}")).size(13.0).strong().color(p.ink)).truncate());
            });
        });
        ui.add_space(2.0);
        let (icon, text, col) = match &info.version {
            VersionState::UpToDate => ("circle-check-filled", aurora_i18n::tr!("ui-login-up-to-date").to_owned(), p.success),
            VersionState::Available { version, .. } => (
                "sparkles-filled",
                aurora_i18n::tr!("status-v-version-available", version = version.to_string()),
                p.amber,
            ),
            VersionState::Checking => ("refresh", aurora_i18n::tr!("ui-login-checking").to_owned(), p.muted),
            VersionState::Failed => ("refresh", aurora_i18n::tr!("ui-login-unable-to-check").to_owned(), p.muted),
            VersionState::NotConfigured => (
                "circle-check-filled",
                aurora_i18n::tr!("ui-login-development-version").to_owned(),
                p.muted,
            ),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("ui-login-latest")).size(12.5).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(text).size(12.5).color(col));
                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                img(ui, icons, icon, r, col);
            });
        });
        if info.can_check_updates {
            ui.add_space(8.0);
            check_updates =
                super::widgets::flat_button_sized(ui, p, aurora_i18n::tr!("update-check"), Vec2::new(ui.available_width(), 30.0))
                    .on_hover_text(aurora_i18n::tr!("update-check-hint"))
                    .clicked();
        }
        if let VersionState::Available { url, .. } = &info.version
            && !url.is_empty()
        {
            ui.add_space(8.0);
            let size = Vec2::new(ui.available_width(), 30.0);
            let label = aurora_i18n::tr!("ui-login-download-update");
            if widgets::action_button(ui, p, "download-filled", label, ActionKind::Primary, size).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
    });
    check_updates
}

/// Side of the mark after the title of a news that opens a link.
const LINK_MARK: f32 = 13.0;

/// News card, `height` tall whatever the number of news (the list scrolls).
fn news_panel(ui: &mut egui::Ui, p: &Palette, icons: &Icons, info: &LoginInfo, height: f32) {
    const MARGIN: i8 = 16;
    card(ui, p, MARGIN, false, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(height - 2.0 * f32::from(MARGIN));
        widgets::section_header(ui, p, "article-filled", aurora_i18n::tr!("ui-login-news"));
        let rest = ui.available_height();
        egui::ScrollArea::vertical()
            .max_height(rest)
            .min_scrolled_height(rest)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for (i, n) in info.news.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(4.0);
                        let r = ui.available_rect_before_wrap();
                        // lighter than in a window: the card is glass over the sky
                        let mut line = widgets::divider_stroke(p);
                        line.color = line.color.gamma_multiply(0.7);
                        ui.painter().hline(r.x_range(), r.top(), line);
                        ui.add_space(6.0);
                    }
                    let has_url = !n.url.is_empty();
                    let resp = ui
                        .scope(|ui| {
                            if !n.date.is_empty() {
                                ui.label(RichText::new(&n.date).size(11.0).color(p.muted_dim));
                            }
                            // a title of several lines leaves the link mark its
                            // place: it stood beyond the card, which grew wider
                            let mark = if has_url { LINK_MARK + ui.spacing().item_spacing.x } else { 0.0 };
                            let room = ui.available_width() - mark;
                            ui.horizontal(|ui| {
                                ui.scope(|ui| {
                                    ui.set_max_width(room);
                                    ui.add(egui::Label::new(RichText::new(&n.title).size(14.0).strong().color(p.ink)).wrap());
                                });
                                if has_url {
                                    let (r, _) = ui.allocate_exact_size(Vec2::splat(LINK_MARK), Sense::hover());
                                    img(ui, icons, "external-link-filled", r, p.muted);
                                }
                            });
                            if !n.summary.is_empty() {
                                ui.add(egui::Label::new(RichText::new(&n.summary).size(12.5).color(p.muted)).wrap());
                            }
                        })
                        .response;
                    if has_url {
                        let r = ui.interact(resp.rect, ui.scope_id().with(("news", i)), Sense::click());
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if r.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&n.url));
                        }
                    }
                }
            });
    });
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    logo: Option<&egui::TextureHandle>,
    settings: &mut Settings,
    form: &mut LoginForm,
    info: &LoginInfo,
    progress: Option<(&str, f32)>,
) -> LoginAction {
    if form.accounts_loaded {
        return carousel::show(ui, p, icons, logo, settings, form, info, progress);
    }
    draw_background(ui, p);
    let mut action = LoginAction::None;
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let wide = screen.width() >= 1100.0;
    let card_w = 380.0f32.min(screen.width() - 32.0);

    // ---- brand + login card (center)
    let mut login_card = egui::Rect::NOTHING;
    let area = egui::Id::unique("login_area");
    // In a low window (1280 × 720) the centred block would reach the line of
    // buttons along the bottom: the news, as tall as the card, then stood
    // over the version and Quit. The block moves up by what it needs, and
    // the logo gives up the height still missing (the height of the block
    // is the one of the frame before).
    let buttons_top = screen.bottom() - BAR_MARGIN - BAR_BUTTON;
    let lowest = buttons_top - BAR_GAP;
    let logo_drawn = area.with("logo");
    let last = ctx
        .memory(|m| m.area_rect(area))
        .map(|rect| (rect.height(), ctx.data(|d| d.get_temp::<f32>(logo_drawn)).unwrap_or(0.0)));
    let logo_size = last.map_or(LOGO, |(height, before)| {
        (lowest - BAR_GAP - (height - before)).clamp(LOGO_MIN, LOGO).round()
    });
    let drawn = if logo.is_some() { logo_size } else { 0.0 };
    let lift = last.map_or(0.0, |(height, before)| {
        let height = height - before + drawn;
        let top = screen.center().y + CENTER_OFFSET - height / 2.0;
        (top + height - lowest).min(top - BAR_GAP).max(0.0)
    });
    ctx.data_mut(|d| d.insert_temp(logo_drawn, drawn));
    egui::Area::new(area)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, CENTER_OFFSET - lift))
        .show(&ctx, |ui| {
            ui.set_width(card_w);
            ui.vertical_centered(|ui| {
                if let Some(l) = logo {
                    let r = ui.add(egui::Image::new(l).fit_to_exact_size(Vec2::splat(logo_size))).rect;
                    // soft light behind the logo (additive)
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("halo")));
                    super::aurora_bg::radial_glow(&painter, r.center(), 150.0, p.violet, 0.42);
                }
                ui.add_space(2.0);
                ui.label(RichText::new("Aurora Viewer").size(34.0).strong().color(p.ink));
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-login-second-life-client-rust-vulkan"))
                        .size(13.0)
                        .color(p.violet_pale),
                );
                ui.add_space(18.0);
            });
            login_card = card(ui, p, 24, true, |ui| {
                ui.set_width(card_w - 48.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                let enabled = !form.busy && !super::grids::is_open(&ctx);
                ui.add_enabled_ui(enabled, |ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-login-login")).size(18.0).strong().color(p.ink));
                    ui.add_space(6.0);
                    caption(ui, p, aurora_i18n::tr!("common-username"));
                    let user = field(
                        ui,
                        p,
                        icons,
                        "login_user",
                        "user-filled",
                        aurora_i18n::tr!("extra-first-last-or-username"),
                        &mut settings.username,
                        None,
                    );
                    ui.add_space(4.0);
                    caption(ui, p, aurora_i18n::tr!("ui-login-password"));
                    // LLPanelLogin::setFields (indra/newview/llpanellogin.cpp,
                    // originally LGPL 2.1): show 16 masked filler characters.
                    // Keep the marker out of the input so login still uses the
                    // saved hash, and typing replaces it without editing filler.
                    let hint = if form.stored.is_some() { REMEMBERED_PASSWORD_MASK } else { "" };
                    let pass = field(
                        ui,
                        p,
                        icons,
                        "login_pass",
                        "lock-filled",
                        hint,
                        &mut form.password,
                        Some(&mut form.show_password),
                    );
                    let mut mfa = None;
                    if form.show_mfa {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-login-authentication-code-2fa"))
                                .size(11.5)
                                .color(p.amber),
                        );
                        mfa = Some(field(
                            ui,
                            p,
                            icons,
                            "login_mfa",
                            "shield-lock-filled",
                            "000000",
                            &mut form.mfa_token,
                            None,
                        ));
                        widgets::checkbox(ui, p, &mut form.remember_mfa, aurora_i18n::tr!("login-accounts-remember-mfa"));
                    }

                    ui.add_space(6.0);
                    // grid and start location side by side
                    ui.columns(2, |cols| {
                        caption(&mut cols[0], p, aurora_i18n::tr!("ui-login-grid"));
                        let mut uri = settings.login_uri();
                        super::grids::selector(&mut cols[0], &settings.grids, &mut uri);
                        settings.grid = crate::login_accounts::grid(&uri);
                        if settings.grid == GridChoice::Custom && !uri.is_empty() {
                            settings.custom_login_uri = uri;
                        }
                        caption(&mut cols[1], p, aurora_i18n::tr!("ui-login-start"));
                        let width = cols[1].available_width();
                        start_menu(
                            &mut cols[1],
                            egui::ComboBox::from_id_salt("start").width(width),
                            &mut settings.start_location,
                            &mut form.start_slurl,
                        );
                    });
                    if settings.grid == GridChoice::Custom {
                        field(
                            ui,
                            p,
                            icons,
                            "login_grid",
                            "world-filled",
                            "http://grille.exemple:8002/",
                            &mut settings.custom_login_uri,
                            None,
                        );
                    }
                    if settings.start_location == "region" && form.start_slurl.is_none() {
                        field(
                            ui,
                            p,
                            icons,
                            "login_region",
                            "map-pin-filled",
                            aurora_i18n::tr!("ui-login-region-name"),
                            &mut settings.start_region,
                            None,
                        );
                    }
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        super::widgets::switch(ui, p, &mut settings.remember_username);
                        ui.label(RichText::new(aurora_i18n::tr!("ui-login-remember-me")).size(12.5).color(p.muted));
                    });
                    ui.horizontal(|ui| {
                        super::widgets::switch(ui, p, &mut settings.remember_password);
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-login-remember-password"))
                                .size(12.5)
                                .color(p.muted),
                        )
                        .on_hover_text(aurora_i18n::tr!("ui-login-stored-in-windows-credential-manager-as-the-hash-sent"));
                    });
                    ui.add_space(10.0);
                    let can = form.block_reason.is_none()
                        && !settings.username.trim().is_empty()
                        && (!form.password.is_empty() || form.stored.is_some());
                    let (r, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 38.0),
                        if can { Sense::click() } else { Sense::hover() },
                    );
                    // Requested UX: focus the first missing field, or Connect
                    // when credentials are ready. Firestorm giveFocus otherwise
                    // returns to the user-name field when both are present.
                    if !form.focused && enabled {
                        form.focused = true;
                        match mfa {
                            Some(m) if form.mfa_token.is_empty() => m.request_focus(),
                            _ if settings.username.trim().is_empty() => user.request_focus(),
                            _ if can => resp.request_focus(),
                            _ => pass.request_focus(),
                        }
                    }
                    let hot = can && resp.hovered();
                    let fill = if !can {
                        p.violet.gamma_multiply(0.35)
                    } else if hot {
                        p.violet_light
                    } else {
                        p.violet
                    };
                    ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, fill);
                    let txt_col = if can { p.on_accent } else { p.on_accent.gamma_multiply(0.5) };
                    let galley = ui.painter().layout_no_wrap(
                        aurora_i18n::tr!("ui-login-log-in").to_owned(),
                        egui::FontId::proportional(15.0),
                        txt_col,
                    );
                    let total = galley.size().x + 26.0;
                    let x0 = r.center().x - total * 0.5;
                    img(
                        ui,
                        icons,
                        if form.block_reason.is_some() { "lock-filled" } else { "login-2" },
                        egui::Rect::from_center_size(egui::pos2(x0 + 9.0, r.center().y), Vec2::splat(18.0)),
                        txt_col,
                    );
                    super::widgets::paint_ink_centered(
                        ui.painter(),
                        galley,
                        egui::pos2(x0 + 26.0 + (total - 26.0) * 0.5, r.center().y),
                        txt_col,
                    );
                    if hot {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let enter =
                        (user.lost_focus() || pass.lost_focus() || resp.has_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (resp.clicked() || enter) && can && enabled {
                        action = LoginAction::Login;
                    }
                });
                if let Some(reason) = form.block_reason {
                    ui.add_space(8.0);
                    ui.label(RichText::new(reason).size(12.0).color(p.amber));
                }
                if let Some(msg) = form.tos_message.clone() {
                    ui.add_space(8.0);
                    widgets::divider(ui, p);
                    ui.label(RichText::new(aurora_i18n::tr!("ui-login-terms-of-service")).strong().color(p.amber));
                    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                        ui.label(RichText::new(msg).size(12.0).color(p.muted));
                    });
                    if ui
                        .add_enabled(
                            form.block_reason.is_none() && !form.busy,
                            egui::Button::new(aurora_i18n::tr!("ui-login-i-accept-the-terms-and-want-to-log-in")),
                        )
                        .clicked()
                    {
                        form.agree_tos = true;
                        form.tos_message = None;
                        action = LoginAction::Login;
                    }
                }
                if let Some((msg, frac)) = progress {
                    ui.add_space(10.0);
                    widgets::progress(ui, p, "login_progress", frac, ui.available_width(), "", None);
                    ui.label(RichText::new(msg).size(12.0).color(p.violet_pale));
                }
                if let Some(err) = &form.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(err).size(12.5).color(p.danger));
                }
            })
            .response
            .rect;
        });

    // ---- version + news (right)
    if wide {
        if chrome::sidebar(&ctx, p, icons, info, login_card, 330.0, lowest) {
            action = LoginAction::CheckUpdates;
        }
    } else if info.can_check_updates {
        egui::Area::new(egui::Id::unique("login_check_updates"))
            // left of the accessibility button and the preferences cogwheel
            // of `chrome::footer`
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-101.0, 20.0))
            .show(&ctx, |ui| {
                if super::widgets::flat_button_sized(ui, p, aurora_i18n::tr!("update-check"), Vec2::new(190.0, 30.0))
                    .on_hover_text(aurora_i18n::tr!("update-check-hint"))
                    .clicked()
                {
                    action = LoginAction::CheckUpdates;
                }
            });
    }

    let chrome_action = chrome::footer(&ctx, p, icons, info, wide, form);
    if !matches!(chrome_action, LoginAction::None) {
        action = chrome_action;
    }
    action
}

#[cfg(test)]
pub(crate) fn test_output(mut output: egui::FullOutput) -> egui::FullOutput {
    // Headless interaction tests inspect shapes and actions without a GPU uploader.
    output.textures_delta.clear();
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_update_disables_mouse_enter_and_terms_with_synthetic_credentials() {
        fn position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + egui::vec2(3.0, 5.0)),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| position(s, label)),
                _ => None,
            }
        }
        let ctx = egui::Context::default();
        let mut settings = Settings {
            username: "Demo Resident".into(),
            ..Default::default()
        };
        let mut form = LoginForm {
            password: "synthetic".into(),
            block_reason: Some(aurora_i18n::tr!("update-login-required")),
            ..Default::default()
        };
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let info = LoginInfo::preview_release("0.3.0");
        let draw = |events, form: &mut LoginForm, settings: &mut Settings| {
            let mut action = LoginAction::None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    action = show(ui, &p, &icons, None, settings, form, &info, None);
                },
            );
            output.textures_delta.clear();
            (action, output)
        };
        draw(vec![], &mut form, &mut settings);
        let (_, output) = draw(vec![], &mut form, &mut settings);
        assert!(
            output
                .shapes
                .iter()
                .any(|s| position(&s.shape, aurora_i18n::tr!("update-login-required")).is_some())
        );
        let pos = output
            .shapes
            .iter()
            .find_map(|s| position(&s.shape, aurora_i18n::tr!("ui-login-log-in")))
            .expect("button");
        for pressed in [true, false] {
            let (action, _) = draw(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                &mut form,
                &mut settings,
            );
            assert!(matches!(action, LoginAction::None));
        }
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique("login_pass")));
        let (action, _) = draw(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            &mut form,
            &mut settings,
        );
        assert!(matches!(action, LoginAction::None));
        form.tos_message = Some("Synthetic terms".into());
        let (_, output) = draw(vec![], &mut form, &mut settings);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| position(&s.shape, aurora_i18n::tr!("ui-login-i-accept-the-terms-and-want-to-log-in")))
            .expect("terms");
        for pressed in [true, false] {
            let (action, _) = draw(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                &mut form,
                &mut settings,
            );
            assert!(matches!(action, LoginAction::None));
        }
        assert!(!form.agree_tos);
        // Verify the same fixture can log in once the policy permits it.
        form.tos_message = None;
        form.block_reason = None;
        draw(vec![], &mut form, &mut settings);
        let (_, output) = draw(vec![], &mut form, &mut settings);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| position(&s.shape, aurora_i18n::tr!("ui-login-log-in")))
            .expect("enabled button");
        draw(
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            &mut form,
            &mut settings,
        );
        let (action, _) = draw(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            &mut form,
            &mut settings,
        );
        assert!(matches!(action, LoginAction::Login));
    }

    /// At 1280 × 720 the news, as tall as the login card, stood over the
    /// version and the Quit button of the line along the bottom.
    #[test]
    fn the_cards_stay_above_the_line_of_buttons_in_a_low_window() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let info = LoginInfo::preview_release("0.3.0");
        let logo = ctx.load_texture("logo", egui::ColorImage::filled([4, 4], Color32::WHITE), Default::default());
        let rect = |name: &str| {
            ctx.memory(|m| m.area_rect(egui::Id::unique(name)))
                .expect("an area of the login screen")
        };
        // (the fonts of a test make the block some 60 px lower than in the
        // viewer: 680 here is what 740 is there, the block only moves up; at
        // 640 the logo shrinks too)
        for (size, room) in [
            (Vec2::new(1280.0, 680.0), false),
            (Vec2::new(1280.0, 640.0), false),
            (Vec2::new(1600.0, 900.0), true),
            (Vec2::new(1920.0, 1080.0), true),
        ] {
            let mut settings = Settings::default();
            let mut form = LoginForm::default();
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui, &p, &icons, Some(&logo), &mut settings, &mut form, &info, None);
                    },
                );
                output.textures_delta.clear();
            }
            let (block, news, quit) = (rect("login_area"), rect("login_news"), rect("login_quit"));
            assert!(block.top() >= BAR_GAP - 0.5, "{size:?}: the brand stays in the window ({block:?})");
            assert!(
                block.bottom() <= quit.top(),
                "{size:?}: login card {block:?} over the buttons {quit:?}"
            );
            assert!(news.bottom() <= quit.top(), "{size:?}: news {news:?} over the buttons {quit:?}");
            // the news keep the height of the login card, on its line (their
            // frame is drawn around the same height: two pixels more)
            assert!((news.bottom() - block.bottom()).abs() <= 2.5, "{size:?}: {news:?} / {block:?}");
            if room {
                let middle = size.y / 2.0 + CENTER_OFFSET;
                assert!(
                    (block.center().y - middle).abs() < 1.0,
                    "{size:?}: the block is centred ({block:?})"
                );
            } else {
                assert!(
                    block.center().y < size.y / 2.0 + CENTER_OFFSET - 1.0,
                    "{size:?}: the block moved up"
                );
            }
        }
    }

    /// The cogwheel opens the preferences before login, from the top right
    /// corner, beside the narrow window's update button without covering it.
    #[test]
    fn the_cogwheel_asks_for_the_preferences_from_the_top_right_corner() {
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let info = LoginInfo::preview_release("0.3.0");
        for size in [Vec2::new(1000.0, 700.0), Vec2::new(1600.0, 900.0)] {
            let ctx = egui::Context::default();
            let mut settings = Settings::default();
            let mut form = LoginForm::default();
            let mut frame = |events: Vec<egui::Event>| {
                let mut action = LoginAction::None;
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ui| action = show(ui, &p, &icons, None, &mut settings, &mut form, &info, None),
                );
                test_output(output);
                action
            };
            for _ in 0..3 {
                frame(vec![]);
            }
            let rect = |name: &str| ctx.memory(|m| m.area_rect(egui::Id::unique(name)));
            let cog = rect("login_settings").expect("the cogwheel");
            assert!(
                cog.right() <= size.x - 20.0 && cog.right() >= size.x - 30.0 && cog.top() <= 25.0,
                "{size:?}: top right corner ({cog:?})"
            );
            for other in ["login_check_updates", "login_news", "login_version", "login_area"] {
                if let Some(r) = rect(other) {
                    assert!(!r.intersects(cog), "{size:?}: {other} {r:?} under the cogwheel {cog:?}");
                }
            }
            let pos = cog.center();
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]);
            let action = frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
            assert!(matches!(action, LoginAction::Preferences), "{size:?}");

            // the accessibility button, just left of it, opens their
            // Accessibility page
            let access = rect("login_accessibility").expect("the accessibility button");
            assert!(
                access.right() < cog.left() && cog.left() - access.right() <= 12.0 && (access.center().y - cog.center().y).abs() < 1.0,
                "{size:?}: left of the cogwheel ({access:?}, {cog:?})"
            );
            assert_eq!(access.size(), cog.size(), "{size:?}: the cogwheel's size");
            for other in ["login_check_updates", "login_news", "login_version", "login_area"] {
                if let Some(r) = rect(other) {
                    assert!(
                        !r.intersects(access),
                        "{size:?}: {other} {r:?} under the accessibility button {access:?}"
                    );
                }
            }
            let pos = access.center();
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]);
            let action = frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
            assert!(matches!(action, LoginAction::Accessibility), "{size:?}");
        }
    }

    /// A news with a link and a title of several lines made its card wider
    /// than the version card above it: the link mark stood after a title
    /// that already took the whole width.
    #[test]
    fn a_long_news_title_with_a_link_keeps_the_card_width() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let mut info = LoginInfo::preview_release("0.3.0");
        let plain = info.news.clone();
        info.news.insert(
            0,
            super::super::news::NewsItem {
                date: "2026-10-13".into(),
                title: "A title long enough to run over several lines of the news card of the login screen".into(),
                summary: "Text.".into(),
                url: "https://example.org/news".into(),
            },
        );
        let width = |info: &LoginInfo| {
            let mut used = 0.0;
            for _ in 0..2 {
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    ui.set_width(330.0);
                    used = ui.scope(|ui| news_panel(ui, &p, &icons, info, 400.0)).response.rect.width();
                });
                output.textures_delta.clear();
            }
            used
        };
        let with_link = width(&info);
        info.news = plain;
        assert_eq!(with_link, width(&info), "the card is as wide with the link as without");
    }

    #[test]
    fn saved_password_marker_survives_focus_without_becoming_input() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let mut password = String::new();
        let mut show_password = false;
        let mut draw = |hint: &str, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(300.0);
                    field(
                        ui,
                        &p,
                        &icons,
                        "test_pass",
                        "lock-filled",
                        hint,
                        &mut password,
                        Some(&mut show_password),
                    );
                },
            );
            // This headless test has no renderer to consume texture updates.
            output.textures_delta.clear();
            output
        };
        let has_mask = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == REMEMBERED_PASSWORD_MASK))
        };
        assert!(!has_mask(&draw("", vec![])));
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique("test_pass")));
        assert!(has_mask(&draw(REMEMBERED_PASSWORD_MASK, vec![])));
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique("test_pass"))));
        draw(REMEMBERED_PASSWORD_MASK, vec![egui::Event::Text("replacement".into())]);
        // egui paints the empty-field hint for the entry frame; the next frame
        // must show the newly typed value rather than the remembered marker.
        assert!(!has_mask(&draw(REMEMBERED_PASSWORD_MASK, vec![])));
        assert_eq!(password, "replacement");
        assert!(!show_password);
    }
}
