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

const REMEMBERED_PASSWORD_MASK: &str = "••••••••••••••••";

#[derive(Default)]
pub struct LoginForm {
    pub password: String,
    pub mfa_token: String,
    pub show_mfa: bool,
    pub tos_message: Option<String>,
    pub agree_tos: bool,
    pub error: Option<String>,
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
}

#[derive(Default)]
pub enum LoginAction {
    ReportBug,
    #[default]
    None,
    Login,
    Quit,
    CheckUpdates,
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
    let id = egui::Id::new(id);
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
                        .frame(egui::Frame::NONE)
                        .margin(Margin::ZERO)
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

/// Height of the buttons along the bottom of the screen.
const BAR_BUTTON: f32 = 30.0;

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

/// Card of the login screen: the surface of the in-world windows.
fn card(ui: &egui::Ui, p: &Palette, margin: i8) -> Frame {
    widgets::overlay_frame(ui.ctx(), p).inner_margin(Margin::same(margin))
}

fn version_panel(ui: &mut egui::Ui, p: &Palette, icons: &Icons, info: &LoginInfo) -> bool {
    let mut check_updates = false;
    card(ui, p, 16).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_header(ui, p, "tag-filled", aurora_i18n::tr!("ui-login-version"));
        let current = aurora_version::version();
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("ui-login-installed")).size(12.5).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("v{current}")).size(13.0).strong().color(p.ink));
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

fn news_panel(ui: &mut egui::Ui, p: &Palette, icons: &Icons, info: &LoginInfo, max_h: f32) {
    card(ui, p, 16).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_header(ui, p, "article-filled", aurora_i18n::tr!("ui-login-news"));
        egui::ScrollArea::vertical()
            .max_height(max_h)
            .min_scrolled_height(max_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for (i, n) in info.news.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(4.0);
                        let r = ui.available_rect_before_wrap();
                        ui.painter()
                            .hline(r.x_range(), r.top(), Stroke::new(1.0, p.raised.gamma_multiply(0.7)));
                        ui.add_space(6.0);
                    }
                    let has_url = !n.url.is_empty();
                    let resp = ui
                        .scope(|ui| {
                            if !n.date.is_empty() {
                                ui.label(RichText::new(&n.date).size(11.0).color(p.muted_dim));
                            }
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new(RichText::new(&n.title).size(14.0).strong().color(p.ink)).wrap());
                                if has_url {
                                    let (r, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                                    img(ui, icons, "external-link-filled", r, p.muted);
                                }
                            });
                            if !n.summary.is_empty() {
                                ui.add(egui::Label::new(RichText::new(&n.summary).size(12.5).color(p.muted)).wrap());
                            }
                        })
                        .response;
                    if has_url {
                        let r = ui.interact(resp.rect, ui.id().with(("news", i)), Sense::click());
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
    draw_background(ui, p);
    let mut action = LoginAction::None;
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let wide = screen.width() >= 1100.0;
    let card_w = 380.0f32.min(screen.width() - 32.0);

    // ---- brand + login card (center)
    egui::Area::new(egui::Id::new("login_area"))
        .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -10.0))
        .show(&ctx, |ui| {
            ui.set_width(card_w);
            ui.vertical_centered(|ui| {
                if let Some(l) = logo {
                    let r = ui.add(egui::Image::new(l).fit_to_exact_size(Vec2::splat(150.0))).rect;
                    // soft light behind the logo (additive)
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("halo")));
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
            card(ui, p, 24).show(ui, |ui| {
                ui.set_width(card_w - 48.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                let enabled = !form.busy;
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
                    }

                    ui.add_space(6.0);
                    // grid and start location side by side
                    ui.columns(2, |cols| {
                        caption(&mut cols[0], p, aurora_i18n::tr!("ui-login-grid"));
                        egui::ComboBox::from_id_salt("grid")
                            .width(cols[0].available_width())
                            .selected_text(match settings.grid {
                                GridChoice::SecondLife => "Second Life",
                                GridChoice::SecondLifeBeta => aurora_i18n::tr!("ui-login-aditi-beta"),
                                GridChoice::Custom => aurora_i18n::tr!("ui-login-other-grid"),
                            })
                            .show_ui(&mut cols[0], |ui| {
                                ui.selectable_value(&mut settings.grid, GridChoice::SecondLife, "Second Life");
                                ui.selectable_value(
                                    &mut settings.grid,
                                    GridChoice::SecondLifeBeta,
                                    aurora_i18n::tr!("status-second-life-beta-aditi"),
                                );
                                ui.selectable_value(&mut settings.grid, GridChoice::Custom, aurora_i18n::tr!("ui-login-other-grid"));
                            });
                        caption(&mut cols[1], p, aurora_i18n::tr!("ui-login-start"));
                        egui::ComboBox::from_id_salt("start")
                            .width(cols[1].available_width())
                            .selected_text(match settings.start_location.as_str() {
                                "home" => aurora_i18n::tr!("common-home"),
                                "region" => aurora_i18n::tr!("ui-login-region"),
                                _ => aurora_i18n::tr!("ui-login-last-location"),
                            })
                            .show_ui(&mut cols[1], |ui| {
                                ui.selectable_value(
                                    &mut settings.start_location,
                                    "last".into(),
                                    aurora_i18n::tr!("ui-login-last-location"),
                                );
                                ui.selectable_value(&mut settings.start_location, "home".into(), aurora_i18n::tr!("common-home"));
                                ui.selectable_value(&mut settings.start_location, "region".into(), aurora_i18n::tr!("ui-login-region"));
                            });
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
                    if settings.start_location == "region" {
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
                    let txt_col = if can { Color32::WHITE } else { Color32::WHITE.gamma_multiply(0.5) };
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
                    if (resp.clicked() || enter) && can && !form.busy {
                        action = LoginAction::Login;
                    }
                });
                if let Some(reason) = form.block_reason {
                    ui.add_space(8.0);
                    ui.label(RichText::new(reason).size(12.0).color(p.amber));
                }
                if let Some(msg) = form.tos_message.clone() {
                    ui.add_space(8.0);
                    ui.separator();
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
                    super::loading::progress_bar(ui, p, ui.available_width(), Some(frac), ctx.input(|i| i.time) as f32);
                    ui.label(RichText::new(msg).size(12.0).color(p.violet_pale));
                }
                if let Some(err) = &form.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(err).size(12.5).color(p.danger));
                }
            });
        });

    // ---- version + news (right)
    if wide {
        let col_w = 330.0;
        egui::Area::new(egui::Id::new("login_side"))
            .anchor(egui::Align2::RIGHT_CENTER, Vec2::new(-48.0, -10.0))
            .show(&ctx, |ui| {
                ui.set_width(col_w);
                if version_panel(ui, p, icons, info) {
                    action = LoginAction::CheckUpdates;
                }
                ui.add_space(14.0);
                news_panel(ui, p, icons, info, (screen.height() * 0.42).clamp(160.0, 420.0));
            });
    } else if info.can_check_updates {
        egui::Area::new(egui::Id::new("login_check_updates"))
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-20.0, 20.0))
            .show(&ctx, |ui| {
                if super::widgets::flat_button_sized(ui, p, aurora_i18n::tr!("update-check"), Vec2::new(190.0, 30.0))
                    .on_hover_text(aurora_i18n::tr!("update-check-hint"))
                    .clicked()
                {
                    action = LoginAction::CheckUpdates;
                }
            });
    }

    // ---- community links (bottom left)
    egui::Area::new(egui::Id::new("login_links"))
        .anchor(egui::Align2::LEFT_BOTTOM, Vec2::new(32.0, -28.0))
        .show(&ctx, |ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-login-join-the-community"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                link_button(ui, p, "brand-discord-filled", "Discord", crate::links::DISCORD);
                link_button(ui, p, "brand-github-filled", "GitHub", crate::links::GITHUB);
                link_button(ui, p, "world-filled", aurora_i18n::tr!("ui-login-website"), crate::links::WEBSITE);
                let label = aurora_i18n::tr!("bug-title");
                if widgets::action_button(ui, p, "bug-filled", label, ActionKind::Flat, Vec2::new(0.0, BAR_BUTTON)).clicked() {
                    action = LoginAction::ReportBug;
                }
            });
        });

    // ---- version + quit (bottom right)
    egui::Area::new(egui::Id::new("login_quit"))
        .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-32.0, -30.0))
        .show(&ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                ui.label(
                    RichText::new(aurora_i18n::tr!(
                        "ui-login-aurora-viewer-v",
                        value0 = aurora_version::version().to_string()
                    ))
                    .size(11.5)
                    .color(p.muted_dim),
                );
                let label = aurora_i18n::tr!("common-quit");
                if widgets::action_button(ui, p, "power", label, ActionKind::FlatDanger, Vec2::new(0.0, BAR_BUTTON)).clicked() {
                    action = LoginAction::Quit;
                }
            });
        });
    action
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
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("login_pass")));
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
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("test_pass")));
        assert!(has_mask(&draw(REMEMBERED_PASSWORD_MASK, vec![])));
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("test_pass"))));
        draw(REMEMBERED_PASSWORD_MASK, vec![egui::Event::Text("replacement".into())]);
        // egui paints the empty-field hint for the entry frame; the next frame
        // must show the newly typed value rather than the remembered marker.
        assert!(!has_mask(&draw(REMEMBERED_PASSWORD_MASK, vec![])));
        assert_eq!(password, "replacement");
        assert!(!show_password);
    }
}
