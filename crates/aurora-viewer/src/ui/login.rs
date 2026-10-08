//! Login screen: animated aurora background, brand block and a glass login
//! card in the middle, version and news panels on the right, community
//! links at the bottom left.

use super::icons::Icons;
use super::news::{LoginInfo, VersionState};
use crate::settings::{GridChoice, Settings};
use crate::theme::Palette;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, TextEdit, Vec2};

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
    #[default]
    None,
    Login,
    Quit,
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
        Stroke::new(1.0, p.raised)
    };
    Frame::new()
        .fill(p.field)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(6))
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
                    img(ui, icons, if *show && !remembered { "eye-slash" } else { "eye" }, r, col);
                    let tip = if remembered {
                        "Mot de passe retenu. Saisissez-en un nouveau pour le remplacer."
                    } else if *show {
                        "Masquer"
                    } else {
                        "Afficher"
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

/// Round icon button (community links); `url` empty = not configured.
fn link_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, name: &str, url: &str) {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
    let enabled = !url.is_empty();
    let hot = enabled && resp.hovered();
    ui.painter().circle_filled(
        r.center(),
        19.0,
        if hot {
            p.violet.gamma_multiply(0.85)
        } else {
            Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), 170)
        },
    );
    ui.painter()
        .circle_stroke(r.center(), 19.0, Stroke::new(1.0, if hot { p.violet_light } else { p.raised }));
    let tint = if !enabled {
        p.muted
    } else if hot {
        Color32::WHITE
    } else {
        p.ink
    };
    img(ui, icons, icon, egui::Rect::from_center_size(r.center(), Vec2::splat(18.0)), tint);
    let tip = if enabled {
        name.to_owned()
    } else {
        format!("{name} (lien à venir)")
    };
    if resp.on_hover_text(tip).clicked() && enabled {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

/// Glass panel used by the card and the side panels.
fn glass(p: &Palette, radius: u8, margin: i8) -> Frame {
    Frame::new()
        .fill({
            // night-blue glass: the panel color tinted with the sky
            let m = |a: u8, b: u8| ((a as u16 + b as u16 * 2) / 3) as u8;
            Color32::from_rgba_unmultiplied(
                m(p.panel.r(), p.navy.r()),
                m(p.panel.g(), p.navy.g()),
                m(p.panel.b(), p.navy.b()),
                205,
            )
        })
        .stroke(Stroke::new(1.0, p.violet.gamma_multiply(0.28)))
        .corner_radius(CornerRadius::same(radius))
        .inner_margin(Margin::same(margin))
        .shadow(egui::Shadow {
            offset: [0, 10],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha(110),
        })
}

fn panel_title(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        img(ui, icons, icon, r, p.violet_light);
        ui.label(RichText::new(title.to_uppercase()).size(11.5).strong().color(p.violet_light));
    });
    ui.add_space(4.0);
}

fn version_panel(ui: &mut egui::Ui, p: &Palette, icons: &Icons, info: &LoginInfo) {
    glass(p, 10, 16).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel_title(ui, p, icons, "tag", "Version");
        let current = env!("CARGO_PKG_VERSION");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Installée").size(12.5).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("v{current}")).size(13.0).strong().color(p.ink));
            });
        });
        ui.add_space(2.0);
        let (icon, text, col) = match &info.version {
            VersionState::UpToDate => ("check-circle", "À jour".to_owned(), p.success),
            VersionState::Available { version, .. } => ("sparkle", format!("v{version} disponible"), p.amber),
            VersionState::Checking => ("arrows-clockwise", "Vérification…".to_owned(), p.muted),
            VersionState::Failed => ("arrows-clockwise", "Vérification impossible".to_owned(), p.muted),
            VersionState::NotConfigured => ("check-circle", "Version de développement".to_owned(), p.muted),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Dernière").size(12.5).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(text).size(12.5).color(col));
                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                img(ui, icons, icon, r, col);
            });
        });
        if let VersionState::Available { url, .. } = &info.version {
            ui.add_space(8.0);
            let b = egui::Button::new(RichText::new("Télécharger la mise à jour").size(13.0).color(Color32::WHITE))
                .fill(p.violet)
                .corner_radius(CornerRadius::same(6))
                .min_size(Vec2::new(ui.available_width(), 30.0));
            if ui.add(b).clicked() && !url.is_empty() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
    });
}

fn news_panel(ui: &mut egui::Ui, p: &Palette, icons: &Icons, info: &LoginInfo, max_h: f32) {
    glass(p, 10, 16).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel_title(ui, p, icons, "newspaper", "Actualités");
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
                                    img(ui, icons, "arrow-square-out", r, p.muted);
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
                ui.label(RichText::new("Client Second Life  ·  Rust  ·  Vulkan").size(13.0).color(p.violet_pale));
                ui.add_space(18.0);
            });
            glass(p, 12, 24).show(ui, |ui| {
                ui.set_width(card_w - 48.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                let enabled = !form.busy;
                ui.add_enabled_ui(enabled, |ui| {
                    ui.label(RichText::new("Connexion").size(18.0).strong().color(p.ink));
                    ui.add_space(6.0);
                    caption(ui, p, "Nom d'utilisateur");
                    let user = field(ui, p, icons, "login_user", "user", "prenom.nom ou nom d'utilisateur", &mut settings.username, None);
                    ui.add_space(4.0);
                    caption(ui, p, "Mot de passe");
                    // LLPanelLogin::setFields (indra/newview/llpanellogin.cpp,
                    // originally LGPL 2.1): show 16 masked filler characters.
                    // Keep the marker out of the input so login still uses the
                    // saved hash, and typing replaces it without editing filler.
                    let hint = if form.stored.is_some() { REMEMBERED_PASSWORD_MASK } else { "" };
                    let pass = field(ui, p, icons, "login_pass", "lock-key", hint, &mut form.password, Some(&mut form.show_password));
                    let mut mfa = None;
                    if form.show_mfa {
                        ui.add_space(4.0);
                        ui.label(RichText::new("Code d'authentification (2FA)").size(11.5).color(p.amber));
                        mfa = Some(field(ui, p, icons, "login_mfa", "lock-key", "000000", &mut form.mfa_token, None));
                    }

                    ui.add_space(6.0);
                    // grid and start location side by side
                    ui.columns(2, |cols| {
                        caption(&mut cols[0], p, "Grille");
                        egui::ComboBox::from_id_salt("grid")
                            .width(cols[0].available_width())
                            .selected_text(match settings.grid {
                                GridChoice::SecondLife => "Second Life",
                                GridChoice::SecondLifeBeta => "Aditi (bêta)",
                                GridChoice::Custom => "Autre grille…",
                            })
                            .show_ui(&mut cols[0], |ui| {
                                ui.selectable_value(&mut settings.grid, GridChoice::SecondLife, "Second Life");
                                ui.selectable_value(&mut settings.grid, GridChoice::SecondLifeBeta, "Second Life Beta (Aditi)");
                                ui.selectable_value(&mut settings.grid, GridChoice::Custom, "Autre grille…");
                            });
                        caption(&mut cols[1], p, "Départ");
                        egui::ComboBox::from_id_salt("start")
                            .width(cols[1].available_width())
                            .selected_text(match settings.start_location.as_str() {
                                "home" => "Domicile",
                                "region" => "Région…",
                                _ => "Dernière position",
                            })
                            .show_ui(&mut cols[1], |ui| {
                                ui.selectable_value(&mut settings.start_location, "last".into(), "Dernière position");
                                ui.selectable_value(&mut settings.start_location, "home".into(), "Domicile");
                                ui.selectable_value(&mut settings.start_location, "region".into(), "Région…");
                            });
                    });
                    if settings.grid == GridChoice::Custom {
                        field(ui, p, icons, "login_grid", "globe-simple", "http://grille.exemple:8002/", &mut settings.custom_login_uri, None);
                    }
                    if settings.start_location == "region" {
                        field(ui, p, icons, "login_region", "map-pin", "Nom de la région", &mut settings.start_region, None);
                    }
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        super::widgets::switch(ui, p, &mut settings.remember_username);
                        ui.label(RichText::new("Se souvenir de moi").size(12.5).color(p.muted));
                    });
                    ui.horizontal(|ui| {
                        super::widgets::switch(ui, p, &mut settings.remember_password);
                        ui.label(RichText::new("Retenir le mot de passe").size(12.5).color(p.muted)).on_hover_text(
                            "Gardé dans le Gestionnaire d'identification de Windows, sous la forme chiffrée envoyée à la connexion (jamais en clair) ; \
                             toute personne ouvrant votre session Windows pourra se connecter",
                        );
                    });
                    ui.add_space(10.0);
                    let can = !settings.username.trim().is_empty() && (!form.password.is_empty() || form.stored.is_some());
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
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
                    ui.painter().rect_filled(r, CornerRadius::same(8), fill);
                    if hot {
                        ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(10), Stroke::new(2.0, p.violet.gamma_multiply(0.35)), egui::StrokeKind::Outside);
                    }
                    let txt_col = if can { Color32::WHITE } else { Color32::WHITE.gamma_multiply(0.5) };
                    let galley = ui.painter().layout_no_wrap("Se connecter".to_owned(), egui::FontId::proportional(16.0), txt_col);
                    let total = galley.size().x + 26.0;
                    let x0 = r.center().x - total * 0.5;
                    img(ui, icons, "sign-in", egui::Rect::from_center_size(egui::pos2(x0 + 9.0, r.center().y), Vec2::splat(18.0)), txt_col);
                    super::widgets::paint_ink_centered(ui.painter(), galley, egui::pos2(x0 + 26.0 + (total - 26.0) * 0.5, r.center().y), txt_col);
                    if hot {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let enter = (user.lost_focus() || pass.lost_focus() || resp.has_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (resp.clicked() || enter) && can && !form.busy {
                        action = LoginAction::Login;
                    }
                });
                if let Some(msg) = form.tos_message.clone() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(RichText::new("Conditions d'utilisation").strong().color(p.amber));
                    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                        ui.label(RichText::new(msg).size(12.0).color(p.muted));
                    });
                    if ui.button("J'accepte les conditions et je me connecte").clicked() {
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
                version_panel(ui, p, icons, info);
                ui.add_space(14.0);
                news_panel(ui, p, icons, info, (screen.height() * 0.42).clamp(160.0, 420.0));
            });
    }

    // ---- community links (bottom left)
    egui::Area::new(egui::Id::new("login_links"))
        .anchor(egui::Align2::LEFT_BOTTOM, Vec2::new(32.0, -28.0))
        .show(&ctx, |ui| {
            ui.label(RichText::new("Rejoignez la communauté").size(11.5).color(p.muted));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                link_button(ui, p, icons, "github-logo", "GitHub", crate::links::GITHUB);
                link_button(ui, p, icons, "discord-logo", "Discord", crate::links::DISCORD);
                link_button(ui, p, icons, "globe-simple", "Site web", crate::links::WEBSITE);
            });
        });

    // ---- version + quit (bottom right)
    egui::Area::new(egui::Id::new("login_quit"))
        .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-32.0, -30.0))
        .show(&ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                ui.label(
                    RichText::new(format!("Aurora Viewer v{}", env!("CARGO_PKG_VERSION")))
                        .size(11.5)
                        .color(p.muted_dim),
                );
                let (r, resp) = ui.allocate_exact_size(Vec2::new(92.0, 30.0), Sense::click());
                let hot = resp.hovered();
                ui.painter().rect_filled(
                    r,
                    CornerRadius::same(6),
                    if hot {
                        p.danger.gamma_multiply(0.8)
                    } else {
                        Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), 170)
                    },
                );
                ui.painter().rect_stroke(
                    r,
                    CornerRadius::same(6),
                    Stroke::new(1.0, if hot { p.danger } else { p.raised }),
                    egui::StrokeKind::Inside,
                );
                let col = if hot { Color32::WHITE } else { p.muted };
                img(
                    ui,
                    icons,
                    "power",
                    egui::Rect::from_center_size(egui::pos2(r.left() + 20.0, r.center().y), Vec2::splat(15.0)),
                    col,
                );
                let galley = ui
                    .painter()
                    .layout_no_wrap("Quitter".to_owned(), egui::FontId::proportional(13.0), col);
                super::widgets::paint_ink_centered(ui.painter(), galley, egui::pos2(r.left() + 56.0, r.center().y), col);
                if resp.clicked() {
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
    fn saved_password_marker_survives_focus_without_becoming_input() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let mut password = String::new();
        let mut show_password = false;
        let mut draw = |hint: &str, events: Vec<egui::Event>| {
            ctx.run_ui(
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
                        "lock-key",
                        hint,
                        &mut password,
                        Some(&mut show_password),
                    );
                },
            )
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
