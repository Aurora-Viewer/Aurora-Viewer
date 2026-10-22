//! Account cards and separate Add button: user-selected Aurora design.
//! Account selection follows LLPanelLogin::onUserListCommit (Firestorm).
use super::{LoginAction, LoginForm, derez, img};
use crate::login_accounts::Account;
use crate::settings::{GridChoice, Settings};
use crate::theme::Palette;
use crate::ui::{
    icons::Icons,
    news::LoginInfo,
    widgets::{self, ActionKind},
};
use egui::{Context, Id, LayerId, Rect, RichText, Sense, Vec2};

pub(super) fn select(settings: &mut Settings, form: &mut LoginForm, index: usize) {
    let Some(account) = form.accounts.get(index) else {
        return;
    };
    settings.username = account.username.clone();
    account.apply(settings);
    settings.remember_username = true;
    settings.remember_password = account.remember_password;
    settings.mfa_hash.clear();
    form.password.clear();
    form.mfa_token.clear();
    form.show_mfa = false;
    form.remember_mfa = account.remember_mfa;
    form.mfa_authorization = None;
    form.mfa_for = None;
    form.agree_tos = false;
    form.stored = None;
    form.stored_for = None;
    form.show_password = false;
    form.focused = false;
    form.error = None;
    form.tos_message = None;
}

fn snapshot(ctx: &Context, layer: LayerId, rect: Rect, action: Rect) -> derez::Snapshot {
    let shapes = ctx.graphics(|graphics| {
        graphics
            .get(layer)
            .map(|list| list.all_entries().cloned().collect())
            .unwrap_or_default()
    });
    derez::Snapshot { rect, action, shapes }
}

/// The card's grid and start menus; `slurl` is the login screen's start
/// SLURL, shown by every card's start menu.
pub(super) fn controls(
    ui: &mut egui::Ui,
    account: &mut Account,
    slurl: &mut Option<crate::slurl::Location>,
    grids: &[crate::grids::Grid],
) -> bool {
    let previous = (account.uri.clone(), account.start_location.clone(), account.start_region.clone());
    let label_size = Vec2::new(
        (ui.text_style_height(&egui::TextStyle::Body) * 3.2).max(44.0),
        ui.spacing().interact_size.y.max(24.0),
    );
    ui.horizontal(|ui| {
        ui.add_sized(label_size, egui::Label::new(aurora_i18n::tr!("ui-login-grid")));
        crate::ui::grids::selector(ui, grids, &mut account.uri);
    });
    match crate::login_accounts::grid(&account.uri) {
        GridChoice::SecondLife => account.uri = aurora_net::login::SL_MAIN_GRID.into(),
        GridChoice::SecondLifeBeta => account.uri = aurora_net::login::SL_BETA_GRID.into(),
        GridChoice::Custom => {
            if crate::login_accounts::grid(&account.uri) != GridChoice::Custom {
                account.uri.clear();
            }
            ui.add(
                egui::TextEdit::singleline(&mut account.uri)
                    .interactive(ui.is_enabled())
                    .desired_width(ui.available_width()),
            );
        }
    }
    ui.horizontal(|ui| {
        ui.add_sized(label_size, egui::Label::new(aurora_i18n::tr!("ui-login-start")));
        let combo = egui::ComboBox::from_id_salt("accounts_start")
            .width(ui.available_width())
            .truncate();
        super::start_menu(ui, combo, &mut account.start_location, slurl);
    });
    if account.start_location == "region" && slurl.is_none() {
        ui.add(
            egui::TextEdit::singleline(&mut account.start_region)
                .interactive(ui.is_enabled())
                .desired_width(ui.available_width())
                .hint_text(aurora_i18n::tr!("ui-login-region-name")),
        );
    }
    previous != (account.uri.clone(), account.start_location.clone(), account.start_region.clone())
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
    super::draw_background(ui, p);
    let ctx = ui.ctx().clone();
    let grids_open = crate::ui::grids::is_open(&ctx);
    let screen = ctx.content_rect();
    if form.block_reason.is_some() && !form.busy {
        form.transition = None;
    }
    if let Some(transition) = &mut form.transition {
        if transition.draw(&ctx, p, progress) && !form.busy {
            return LoginAction::Login;
        }
        return LoginAction::None;
    }
    if form.error.is_some() || form.show_mfa || form.tos_message.is_some() {
        form.editing = true;
    }
    let first_account = form.accounts.is_empty();
    if first_account {
        form.editing = true;
    }
    let visible: Vec<usize> = (0..form.accounts.len()).collect();
    let layout = super::layout::Layout::new(screen, visible.len());
    let per_page = layout.per_page;
    let paginated = layout.paginated;
    form.page %= visible.len().max(1);
    if !paginated {
        form.page = 0;
        form.motion.stop();
    }
    let mut current_key = (settings.login_uri(), settings.username.trim().to_lowercase());
    // An account being added is not a card yet: from its submit to its
    // answer (success, error, MFA or terms) the identity typed in the Add
    // form is the one sent, challenged and saved, never a saved card's.
    if !form.editing && !form.adding && !form.busy && form.selection_shown_for.as_ref() != Some(&current_key) {
        let index = if !paginated && form.selection_shown_for.is_none() && form.pending_connect.is_none() && !form.busy {
            (!visible.is_empty()).then_some(visible.len() / 2)
        } else {
            form.accounts.iter().position(|account| account.key() == current_key)
        };
        if let Some(index) = index.or_else(|| (!visible.is_empty()).then_some(visible.len() / 2)) {
            if index != index_of(settings, form).unwrap_or(usize::MAX) {
                select(settings, form, index);
                current_key = form.accounts[index].key();
            }
            if paginated && form.pending_connect.is_none() {
                centre(form, index, per_page);
            }
        }
        form.selection_shown_for = Some(current_key.clone());
    }
    let card_width = layout.card_width;
    // The overlay frame includes a one-point stroke on each side.
    let inner_width = card_width - 2.0 * f32::from(layout.margin) - 2.0;
    let count = visible.len().min(per_page);
    let width = count as f32 * card_width + count.saturating_sub(1) as f32 * layout.gap;
    let stride = card_width + layout.gap;
    let track_width = width + 2.0 * layout.peek;
    let row_width = track_width + if paginated { layout.arrow_space } else { 0.0 };
    let mut cards = Vec::new();
    let mut clicked = None;
    let mut edited = None;
    let mut action = LoginAction::None;
    let mut card_rect = Rect::NOTHING;
    let mut arrow_direction = 0;
    let mut centred = None;
    let enter_shortcut = !grids_open
        && !egui::Popup::is_any_open(&ctx)
        && ctx.input(|i| i.modifiers == egui::Modifiers::NONE && i.key_pressed(egui::Key::Enter));
    let available_height = (screen.height() - if first_account { 84.0 } else { 148.0 }).max(100.0);
    let content_height = if form.content_height > 0.0 { form.content_height } else { 650.0 };
    let top = ((screen.height() - 112.0 - content_height.min(available_height)) / 2.0).max(12.0);
    let portrait_height =
        ((screen.height() - 560.0) * 0.15 + 88.0).clamp(88.0, if screen.height() < 800.0 { 100.0 } else { 160.0 }) * layout.scale;
    let place_height = (if screen.height() < 800.0 { 84.0 } else { 112.0 }) * layout.scale;
    let arrow_top = (portrait_height + place_height + 190.0 * layout.text_scale) / 2.0 - 19.0;
    wheel(&ctx, settings, form, &layout);
    current_key = (settings.login_uri(), settings.username.trim().to_lowercase());
    egui::Area::new(Id::unique("accounts_content"))
        .fixed_pos(egui::pos2(screen.center().x - layout.main_width / 2.0, top))
        .show(&ctx, |ui| {
            // a scroll bar, when the content is taller than the room, takes
            // a strip right of the main width: the cards stay centred on
            // the screen and whole
            let strip = if content_height > available_height {
                widgets::scroll_strip(ui)
            } else {
                0.0
            };
            ui.set_width(layout.main_width + strip);
            ui.set_max_height(available_height);
            ui.spacing_mut().item_spacing.y = 4.0;
            let scroll = egui::ScrollArea::vertical()
                .id_salt("accounts_content_scroll")
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: egui::scroll_area::DragScroll::Never,
                    ..Default::default()
                })
                .max_height(available_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        if let Some(logo) = logo {
                            ui.add(
                                egui::Image::new(logo)
                                    .fit_to_exact_size(Vec2::splat((if screen.height() < 800.0 { 32.0 } else { 64.0 }) * layout.scale)),
                            );
                        }
                        ui.label(RichText::new("Aurora Viewer").size(28.0 * layout.text_scale).strong().color(p.ink));
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-login-second-life-client-rust-vulkan"))
                                .size(13.0 * layout.text_scale)
                                .color(p.violet_pale),
                        );
                        ui.add_space(8.0);
                        if first_account {
                            ui.label(
                                RichText::new(aurora_i18n::tr!("login-accounts-first-title"))
                                    .size(20.0 * layout.text_scale)
                                    .strong()
                                    .color(p.ink),
                            );
                        }
                        ui.add_space(10.0);
                    });
                    if first_account {
                        ui.horizontal(|ui| {
                            ui.add_space(((ui.available_width() - 372.0) / 2.0).max(0.0));
                            ui.vertical(|ui| {
                                ui.set_width(372.0);
                                let response = super::card(ui, p, 16, true, |ui| {
                                    ui.set_width(340.0);
                                    super::credentials_form::content(ui, p, icons, settings, form, true, progress)
                                });
                                card_rect = response.response.rect;
                                if response.inner {
                                    action = LoginAction::Login;
                                }
                            });
                        });
                    } else {
                        let height = if form.motion.height > 0.0 {
                            form.motion.height
                        } else {
                            portrait_height + place_height + 220.0 * layout.text_scale
                        };
                        let viewport = Rect::from_min_size(
                            egui::pos2(screen.center().x - width / 2.0, ui.next_widget_position().y),
                            Vec2::new(width, height),
                        );
                        let browse_rect = viewport.expand2(Vec2::new(layout.peek, 0.0));
                        form.motion.viewport = viewport;
                        let can_drag = paginated
                            && !grids_open
                            && !form.editing
                            && !form.managing
                            && !form.show_info
                            && !form.busy
                            && form.pending_connect.is_none()
                            && !egui::Popup::is_any_open(&ctx);
                        // Register browsing before children: card clicks wait for
                        // release, and text fields keep their own drag selection.
                        let grips = [
                            Rect::from_min_max(egui::pos2(ui.max_rect().left(), browse_rect.top()), browse_rect.left_bottom()),
                            Rect::from_min_max(browse_rect.right_top(), egui::pos2(ui.max_rect().right(), browse_rect.bottom())),
                            browse_rect,
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(side, rect)| {
                            ui.interact(
                                rect.intersect(ui.clip_rect()),
                                ui.scope_id().with(("account_grip", side)),
                                if !can_drag {
                                    Sense::hover()
                                } else if side == 2 {
                                    Sense::click_and_drag()
                                } else {
                                    Sense::drag()
                                },
                            )
                        })
                        .collect::<Vec<_>>();
                        form.motion.interact(&ctx, &grips, can_drag, stride, visible.len(), &mut form.page);
                        if can_drag {
                            let index = centre_index(form, per_page, stride);
                            centred = Some(index);
                            if !form.motion.moving() && form.accounts[index].key() != current_key {
                                select(settings, form, index);
                                current_key = form.accounts[index].key();
                                form.selection_shown_for = Some(current_key.clone());
                            }
                        }
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = layout.gap;
                            ui.add_space(((ui.available_width() - row_width) / 2.0).max(0.0));
                            if paginated {
                                ui.vertical(|ui| {
                                    ui.set_width(26.0);
                                    ui.add_space(arrow_top);
                                    if widgets::action_button_if(
                                        ui,
                                        p,
                                        "arrow-left",
                                        "",
                                        ActionKind::Flat,
                                        Vec2::new(26.0, 38.0),
                                        !form.busy,
                                    )
                                    .clicked()
                                    {
                                        arrow_direction = -1;
                                    }
                                });
                            }
                            let (browse_rect, _) = ui.allocate_exact_size(Vec2::new(track_width, height), Sense::hover());
                            form.motion.browse_rect = browse_rect.intersect(ui.clip_rect());
                            let viewport = browse_rect.shrink2(Vec2::new(layout.peek, 0.0));
                            form.motion.viewport = viewport;
                            // A moving card can cross an entire stride while the
                            // outer peeks still need another neighbour beyond it.
                            let neighbours = if form.motion.moving() {
                                2
                            } else if paginated {
                                1
                            } else {
                                0
                            };
                            let start = viewport.min + Vec2::new(form.motion.offset - neighbours as f32 * stride, 0.0);
                            let mut track = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt("account_track")
                                    .max_rect(Rect::from_min_size(
                                        start,
                                        Vec2::new(width + 2.0 * neighbours as f32 * stride, height),
                                    ))
                                    .layout(egui::Layout::left_to_right(egui::Align::Min)),
                            );
                            track.set_clip_rect(browse_rect.intersect(ui.clip_rect()));
                            let mut measured_height: f32 = 0.0;
                            track.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = layout.gap;
                                let slots = -neighbours..count as isize + neighbours;
                                for slot in slots {
                                    let index = (form.page as isize + slot).rem_euclid(visible.len() as isize) as usize;
                                    let mut account = form.accounts[index].clone();
                                    let selected = centred.map_or_else(
                                        || account.key() == current_key,
                                        |_| slot == (per_page / 2) as isize - (form.motion.offset / stride).round() as isize,
                                    );
                                    let previous = account.key();
                                    let mut action_rect = Rect::NOTHING;
                                    let occurrence = if slot < 0 {
                                        -1
                                    } else if slot >= count as isize {
                                        1
                                    } else {
                                        0
                                    };
                                    let predicted = Rect::from_min_size(
                                        viewport.min + Vec2::new(form.motion.offset + slot as f32 * stride, 0.0),
                                        Vec2::new(card_width, height),
                                    );
                                    let faded = layout.peek > 0.0 && !viewport.expand(0.5).contains_rect(predicted);
                                    let hovered = !form.busy
                                        && !form.editing
                                        && !form.managing
                                        && !form.show_info
                                        && form.pending_connect.is_none()
                                        && !form.motion.is_dragging()
                                        && !egui::Popup::is_any_open(&ctx)
                                        && ctx.input(|i| {
                                            i.pointer
                                                .hover_pos()
                                                .is_some_and(|at| predicted.intersect(browse_rect).intersect(ui.clip_rect()).contains(at))
                                        });
                                    let hover =
                                        ctx.animate_bool_with_time(ui.scope_id().with(("account_hover", index, occurrence)), hovered, 0.12);
                                    let marker = ui.painter().add(egui::Shape::Noop);
                                    let result = ui.push_id(("account_card", index, occurrence), |ui| {
                                        if faded {
                                            ui.multiply_opacity(super::fade::opacity(predicted, viewport));
                                        }
                                        // Register the card before its children so selectors
                                        // and editable controls retain pointer priority.
                                        ui.scope_builder(
                                            egui::UiBuilder::new()
                                                .layout(egui::Layout::top_down(egui::Align::Min))
                                                .sense(Sense::click()),
                                            |ui| {
                                                if form.busy {
                                                    ui.disable();
                                                }
                                                ui.set_width(card_width);
                                                let shown_card = super::card_hover(ui, p, layout.margin, selected, hover, |ui| {
                                                    ui.set_width(inner_width);
                                                    for style in [egui::TextStyle::Body, egui::TextStyle::Button] {
                                                        if let Some(font) = ui.style_mut().text_styles.get_mut(&style) {
                                                            font.size *= layout.text_scale;
                                                        }
                                                    }
                                                    ui.spacing_mut().interact_size.y *= layout.text_scale;
                                                    let (portrait_rect, portrait_response) =
                                                        ui.allocate_exact_size(Vec2::new(inner_width, portrait_height), Sense::click());
                                                    let accent = if index.is_multiple_of(2) { p.violet } else { p.teal };
                                                    let mut mesh = egui::Mesh::default();
                                                    for (at, color) in [
                                                        (portrait_rect.left_top(), accent.gamma_multiply(0.40 + hover * 0.14)),
                                                        (portrait_rect.right_top(), p.indigo.gamma_multiply(0.50)),
                                                        (portrait_rect.right_bottom(), p.panel),
                                                        (portrait_rect.left_bottom(), p.panel),
                                                    ] {
                                                        mesh.colored_vertex(at, color);
                                                    }
                                                    mesh.add_triangle(0, 1, 2);
                                                    mesh.add_triangle(0, 2, 3);
                                                    ui.painter().add(egui::Shape::mesh(mesh));
                                                    if let Some(texture) = account.agent.and_then(|agent| form.portraits.get(&agent)) {
                                                        let size = texture.size_vec2();
                                                        let fitted =
                                                            size * (portrait_rect.width() / size.x).min(portrait_rect.height() / size.y);
                                                        ui.painter().image(
                                                            texture.id(),
                                                            Rect::from_center_size(portrait_rect.center(), fitted),
                                                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                                            widgets::PICTURE_TINT,
                                                        );
                                                    } else {
                                                        img(
                                                            ui,
                                                            icons,
                                                            "user-filled",
                                                            Rect::from_center_size(
                                                                portrait_rect.center(),
                                                                Vec2::splat(portrait_height * 0.8),
                                                            ),
                                                            p.violet_pale.gamma_multiply(0.75),
                                                        );
                                                    }
                                                    ui.add_space(8.0);
                                                    ui.add(widgets::elided_label(
                                                        ui,
                                                        RichText::new(&account.username)
                                                            .size(16.0 * layout.text_scale)
                                                            .strong()
                                                            .color(p.ink),
                                                        inner_width,
                                                    ));
                                                    ui.add_space(6.0);
                                                    if controls(ui, &mut account, &mut form.start_slurl, &settings.grids) {
                                                        edited = Some((index, previous, account.clone()));
                                                    }
                                                    ui.add_space(8.0);
                                                    ui.label(
                                                        RichText::new(aurora_i18n::tr!("login-accounts-last-place"))
                                                            .size(11.5 * layout.text_scale)
                                                            .color(p.muted),
                                                    );
                                                    let (place_rect, place_response) =
                                                        ui.allocate_exact_size(Vec2::new(inner_width, place_height), Sense::click());
                                                    if let Some(texture) =
                                                        account.last_place.as_ref().and_then(|place| form.places.get(place))
                                                    {
                                                        let size = texture.size_vec2();
                                                        let ratio = place_rect.width() / place_rect.height();
                                                        let source = size.x / size.y;
                                                        let uv_size = if source > ratio {
                                                            Vec2::new(ratio / source, 1.0)
                                                        } else {
                                                            Vec2::new(1.0, source / ratio)
                                                        };
                                                        ui.painter().image(
                                                            texture.id(),
                                                            place_rect,
                                                            Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size),
                                                            widgets::PICTURE_TINT,
                                                        );
                                                    } else {
                                                        ui.painter().rect_filled(place_rect, ui.visuals().window_corner_radius, p.field);
                                                        img(
                                                            ui,
                                                            icons,
                                                            "map-pin-filled",
                                                            Rect::from_center_size(place_rect.center(), Vec2::splat(24.0)),
                                                            p.muted,
                                                        );
                                                    }
                                                    ui.add(widgets::elided_label(
                                                        ui,
                                                        RichText::new(account.last_place.as_ref().map_or_else(
                                                            || aurora_i18n::tr!("login-accounts-place-after-visit"),
                                                            |place| place.region.as_str(),
                                                        ))
                                                        .size(11.0 * layout.text_scale)
                                                        .color(p.muted),
                                                        inner_width,
                                                    ));
                                                    ui.add_space(14.0);
                                                    let response = widgets::action_button_if(
                                                        ui,
                                                        p,
                                                        "login-2",
                                                        aurora_i18n::tr!("ui-login-log-in"),
                                                        if selected { ActionKind::Primary } else { ActionKind::Flat },
                                                        Vec2::new(inner_width, 36.0 * layout.text_scale),
                                                        form.block_reason.is_none(),
                                                    );
                                                    action_rect = response.rect;
                                                    if (portrait_response.clicked() || place_response.clicked() || response.clicked())
                                                        && form.block_reason.is_none()
                                                        && !form.busy
                                                        && !form.motion.suppress_click
                                                    {
                                                        clicked = Some(index);
                                                    }
                                                });
                                                let rect = shown_card.response.rect;
                                                if ui.response().clicked()
                                                    && !egui::Popup::is_any_open(ui.ctx())
                                                    && !ui.ctx().egui_wants_keyboard_input()
                                                    && form.block_reason.is_none()
                                                    && !form.busy
                                                    && !form.motion.suppress_click
                                                {
                                                    clicked = Some(index);
                                                }
                                                rect
                                            },
                                        )
                                        .inner
                                    });
                                    if faded {
                                        super::fade::apply(ui, marker, browse_rect, viewport);
                                    }
                                    let captured = if form.pending_connect.is_some() {
                                        snapshot(&ctx, ui.layer_id(), result.inner, action_rect)
                                    } else {
                                        derez::Snapshot {
                                            rect: result.inner,
                                            action: action_rect,
                                            shapes: Vec::new(),
                                        }
                                    };
                                    measured_height = measured_height.max(result.inner.height());
                                    if result.inner.intersects(browse_rect) {
                                        cards.push((index, captured));
                                    }
                                }
                            });
                            if (form.motion.height - measured_height).abs() > 0.5 {
                                form.motion.height = measured_height;
                                ctx.request_discard("account card height changed");
                            }
                            card_rect = browse_rect;
                            if paginated {
                                ui.vertical(|ui| {
                                    ui.set_width(26.0);
                                    ui.add_space(arrow_top);
                                    if widgets::action_button_if(
                                        ui,
                                        p,
                                        "arrow-right",
                                        "",
                                        ActionKind::Flat,
                                        Vec2::new(26.0, 38.0),
                                        !form.busy,
                                    )
                                    .clicked()
                                    {
                                        arrow_direction = 1;
                                    }
                                });
                            }
                        });
                    }
                });
            form.content_height = scroll.content_size.y;
        });
    if !first_account {
        egui::Area::new(Id::unique("accounts_footer"))
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(
                screen.center().x,
                (top + form.content_height.min(available_height) + 18.0).min(screen.bottom() - 124.0),
            ))
            .show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "settings-filled",
                        aurora_i18n::tr!("login-accounts-manage"),
                        ActionKind::Flat,
                        Vec2::new(160.0, 36.0),
                    )
                    .clicked()
                    {
                        form.managing = true;
                    }
                    if widgets::action_button(
                        ui,
                        p,
                        "plus",
                        aurora_i18n::tr!("login-accounts-add"),
                        ActionKind::Primary,
                        Vec2::new(152.0, 36.0),
                    )
                    .clicked()
                    {
                        form.add_return = Some((settings.login_uri(), settings.username.trim().to_lowercase()));
                        form.adding = true;
                        settings.username.clear();
                        settings.remember_username = true;
                        settings.remember_password = false;
                        settings.mfa_hash.clear();
                        form.password.clear();
                        form.stored = None;
                        form.stored_for = None;
                        form.mfa_authorization = None;
                        form.mfa_for = None;
                        form.remember_mfa = false;
                        form.editing = true;
                        form.focused = false;
                    }
                });
            });
    }
    if let Some((index, previous, account)) = edited {
        let grid_changed = previous.0 != account.uri;
        form.accounts[index] = account.clone();
        if grid_changed || previous != current_key {
            select(settings, form, index);
        } else {
            account.apply(settings);
        }
        if std::env::var_os("AURORA_DEMO").is_none() {
            form.accounts = crate::login_accounts::replace(&previous, account);
        }
    }
    if !grids_open
        && !form.busy
        && !form.editing
        && !form.managing
        && !form.show_info
        && form.pending_connect.is_none()
        && !egui::Popup::is_any_open(&ctx)
    {
        if arrow_direction != 0 {
            navigate(settings, form, arrow_direction, per_page);
            ctx.request_repaint();
        }
        if !ctx.egui_wants_keyboard_input() {
            let direction = ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
                    1
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                    -1
                } else {
                    0
                }
            });
            if direction != 0 {
                navigate(settings, form, direction, per_page);
                ctx.request_repaint();
            }
        }
        if enter_shortcut && form.block_reason.is_none() {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            clicked = if paginated {
                Some(centre_index(form, per_page, stride))
            } else {
                index_of(settings, form)
            };
        }
    }
    if let Some(width) = layout.sidebar
        && card_rect.is_positive()
        && super::chrome::sidebar(&ctx, p, icons, info, card_rect, width, screen.bottom() - 92.0)
    {
        action = LoginAction::CheckUpdates;
    }
    let chrome_action = super::chrome::footer(&ctx, p, icons, info, layout.sidebar.is_some(), form);
    if !matches!(chrome_action, LoginAction::None) {
        action = chrome_action;
    }
    if let Some(index) = form.pending_connect.take() {
        let now = ctx.input(|i| i.time);
        let requested = *form.pending_since.get_or_insert(now);
        // Newly visible egui areas have an invisible sizing pass followed by
        // a short fade. Snapshot only after their real contents are opaque.
        if ctx.will_discard() || now - requested < 0.25 {
            form.pending_connect = Some(index);
        } else if form.accounts.get(index).is_some_and(|a| a.uri != settings.login_uri()) {
            form.pending_connect = None;
        } else if form.block_reason.is_none() && (!form.password.is_empty() || form.stored.is_some()) {
            if let Some((_, chosen)) = cards.iter().find(|(i, _)| *i == index) {
                form.transition = Some(derez::Transition::new(
                    &ctx,
                    chosen.clone(),
                    cards.iter().filter(|(i, _)| *i != index).map(|(_, s)| s.clone()).collect(),
                ));
                form.pending_since = None;
            } else {
                form.page = visible.iter().position(|i| *i == index).unwrap_or(0);
                form.pending_connect = Some(index);
                form.pending_since = Some(now);
            }
        } else {
            form.editing = true;
        }
    }
    if let Some(index) = clicked {
        form.motion.stop();
        reveal(form, index, per_page);
        if form.accounts[index].key() != (settings.login_uri(), settings.username.trim().to_lowercase()) {
            select(settings, form, index);
        }
        form.selection_shown_for = Some(form.accounts[index].key());
        form.pending_connect = Some(index);
        form.show_password = false;
        form.pending_since = Some(ctx.input(|i| i.time));
    }
    if (form.editing || (form.busy && form.adding))
        && !first_account
        && super::credentials_form::dialog(&ctx, p, icons, settings, form, progress)
    {
        action = LoginAction::Login;
    }
    if form.managing {
        super::manager::show(&ctx, p, icons, settings, form);
    }
    action
}

fn reveal(form: &mut LoginForm, index: usize, per_page: usize) {
    let count = form.accounts.len();
    if count > 0 && (index + count - form.page) % count >= per_page {
        form.page = if index < form.page { index } else { index + 1 - per_page };
    }
}

fn index_of(settings: &Settings, form: &LoginForm) -> Option<usize> {
    let key = (settings.login_uri(), settings.username.trim().to_lowercase());
    form.accounts.iter().position(|account| account.key() == key)
}

fn centre(form: &mut LoginForm, index: usize, per_page: usize) {
    form.motion.stop();
    form.page = (index + form.accounts.len() - per_page / 2) % form.accounts.len();
}

fn centre_index(form: &LoginForm, per_page: usize, stride: f32) -> usize {
    (form.page as isize + (per_page / 2) as isize - (form.motion.offset / stride).round() as isize).rem_euclid(form.accounts.len() as isize)
        as usize
}

/// Read discrete wheel events once, before the vertical scroll area can
/// consume them. The full browse rectangle includes gaps and faded previews.
fn wheel(ctx: &Context, settings: &mut Settings, form: &mut LoginForm, layout: &super::layout::Layout) {
    let active = layout.paginated
        && !crate::ui::grids::is_open(ctx)
        && !form.editing
        && !form.managing
        && !form.show_info
        && !form.busy
        && form.pending_connect.is_none()
        && !form.motion.is_dragging()
        && !ctx.egui_wants_keyboard_input()
        && !egui::Popup::is_any_open(ctx)
        && ctx.input(|i| {
            i.modifiers == egui::Modifiers::NONE && i.pointer.hover_pos().is_some_and(|at| form.motion.browse_rect.contains(at))
        });
    if !active {
        form.wheel_scroll = 0.0;
        return;
    }
    let now = ctx.input(|i| i.time);
    let delta = ctx.input_mut(|i| {
        i.smooth_scroll_delta = Vec2::ZERO;
        if form.wheel_time == Some(now) {
            return 0.0;
        }
        i.events
            .iter()
            .filter_map(|event| {
                let egui::Event::MouseWheel {
                    unit, delta, modifiers, ..
                } = event
                else {
                    return None;
                };
                if *modifiers != egui::Modifiers::NONE {
                    return None;
                }
                Some(
                    (delta.x + delta.y)
                        / match unit {
                            egui::MouseWheelUnit::Point => 55.0,
                            _ => 1.0,
                        },
                )
            })
            .sum::<f32>()
    });
    form.wheel_time = Some(now);
    form.wheel_scroll += delta;
    let steps = form.wheel_scroll.abs().floor() as usize;
    let direction = if form.wheel_scroll < 0.0 { 1 } else { -1 };
    for _ in 0..steps % form.accounts.len() {
        navigate(settings, form, direction, layout.per_page);
    }
    if steps > 0 {
        form.wheel_scroll %= 1.0;
        ctx.request_repaint();
    }
}

fn navigate(settings: &mut Settings, form: &mut LoginForm, direction: i32, per_page: usize) {
    if form.accounts.is_empty() {
        return;
    }
    let paginated = form.accounts.len() > 3;
    let current = if paginated {
        Some(centre_index(form, per_page, form.motion.stride()))
    } else {
        index_of(settings, form)
    };
    form.motion.stop();
    let index = match current {
        Some(current) if paginated => (current as i32 + direction).rem_euclid(form.accounts.len() as i32) as usize,
        Some(current) if direction > 0 => (current + 1).min(form.accounts.len() - 1),
        Some(current) => current.saturating_sub(1),
        None => form.page.min(form.accounts.len() - 1),
    };
    if index_of(settings, form) != Some(index) {
        select(settings, form, index);
    }
    if paginated {
        centre(form, index, per_page);
    } else {
        form.page = 0;
    }
    form.selection_shown_for = Some(form.accounts[index].key());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw_fixture(
        ctx: &Context,
        settings: &mut Settings,
        form: &mut LoginForm,
        frame: usize,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        draw_fixture_with_manager(ctx, settings, form, frame, events, None)
    }

    fn draw_fixture_with_manager(
        ctx: &Context,
        settings: &mut Settings,
        form: &mut LoginForm,
        frame: usize,
        events: Vec<egui::Event>,
        mut manager: Option<(&mut crate::ui::grids::Manager, &tokio::runtime::Handle)>,
    ) -> egui::FullOutput {
        let theme = crate::theme::Theme::default();
        theme.apply(ctx, 1.0);
        super::super::test_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 720.0))),
                time: Some(frame as f64 / 30.0),
                events,
                ..Default::default()
            },
            |ui| {
                show(
                    ui,
                    &theme.palette(),
                    &Icons::default(),
                    None,
                    settings,
                    form,
                    &LoginInfo::preview_release("0.3.0"),
                    None,
                );
                if let Some((manager, runtime)) = manager.as_mut() {
                    manager.show(ctx, &theme.palette(), settings, true, runtime);
                }
            },
        ))
    }

    fn text_rect(output: &egui::FullOutput, label: &str) -> Option<Rect> {
        output.shapes.iter().find_map(|source| {
            let egui::Shape::Text(text) = &source.shape else {
                return None;
            };
            (text.galley.text() == label).then(|| Rect::from_min_size(text.pos, text.galley.size()))
        })
    }

    fn fixture(count: usize) -> (Settings, LoginForm) {
        let settings = Settings {
            username: "synthetic-0".into(),
            ..Default::default()
        };
        let form = LoginForm {
            accounts: (0..count)
                .map(|index| {
                    let mut account = Account::from_settings(&settings, true, None);
                    account.username = format!("synthetic-{index}");
                    account
                })
                .collect(),
            accounts_loaded: true,
            ..Default::default()
        };
        (settings, form)
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn long_selector_labels_keep_the_card_width() {
        let ctx = Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut account = Account::from_settings(&Settings::default(), false, None);
        for frame in 0..4 {
            let mut width = 0.0;
            let _ = super::super::test_output(ctx.run_ui(
                egui::RawInput {
                    time: Some(frame as f64 / 30.0),
                    ..Default::default()
                },
                |ui| {
                    ui.scope(|ui| {
                        ui.set_width(185.0);
                        // Larger text exercises the same overflow as a longer
                        // translation, without changing the process-wide locale.
                        ui.style_mut().text_styles.get_mut(&egui::TextStyle::Button).unwrap().size = 24.0;
                        controls(ui, &mut account, &mut None, &[]);
                        width = ui.min_rect().width();
                    });
                },
            ));
            assert!(width <= 185.1, "selector widened the card to {width}");
        }
    }

    fn wheel_event(unit: egui::MouseWheelUnit, delta: Vec2) -> egui::Event {
        egui::Event::MouseWheel {
            unit,
            delta,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn assert_centred(output: &egui::FullOutput, form: &LoginForm, index: usize) {
        let name = text_rect(output, &format!("synthetic-{index}")).expect("central account name");
        let layout =
            super::super::layout::Layout::new(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 720.0)), form.accounts.len());
        let centre = name.left() - f32::from(layout.margin) - 1.0 + layout.card_width / 2.0;
        assert!((centre - 640.0).abs() < 1.0, "selected card must be centred: {centre}");
    }

    #[test]
    fn three_or_fewer_cards_stay_fixed_for_keys_wheel_and_drag() {
        for count in 1..=3 {
            let ctx = Context::default();
            let (mut settings, mut form) = fixture(count);
            let mut output = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
            for frame in 1..8 {
                output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
            }
            let positions = (0..count)
                .map(|index| text_rect(&output, &format!("synthetic-{index}")).unwrap())
                .collect::<Vec<_>>();
            draw_fixture(&ctx, &mut settings, &mut form, 8, vec![key(egui::Key::ArrowRight)]);
            assert_eq!(settings.username, format!("synthetic-{}", (count - 1).min(count / 2 + 1)));
            let at = form.motion.viewport.left_top() + Vec2::new(120.0, 45.0);
            draw_fixture(
                &ctx,
                &mut settings,
                &mut form,
                9,
                vec![
                    egui::Event::PointerMoved(at),
                    wheel_event(egui::MouseWheelUnit::Line, Vec2::new(0.0, -1.0)),
                ],
            );
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            draw_fixture(&ctx, &mut settings, &mut form, 10, vec![button(at, true)]);
            let end = at + Vec2::new(150.0, 0.0);
            draw_fixture(&ctx, &mut settings, &mut form, 11, vec![egui::Event::PointerMoved(end)]);
            draw_fixture(&ctx, &mut settings, &mut form, 12, vec![button(end, false)]);
            output = draw_fixture(&ctx, &mut settings, &mut form, 13, vec![]);
            for (index, before) in positions.iter().enumerate() {
                assert_eq!(*before, text_rect(&output, &format!("synthetic-{index}")).unwrap());
            }
            assert_eq!(form.page, 0);
            assert!(!form.motion.moving() && form.pending_connect.is_none() && !form.editing);
            assert_ne!(output.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
        }
    }

    #[test]
    fn wheel_over_gaps_selects_the_centre_once_and_respects_dialogs() {
        let ctx = Context::default();
        let (mut settings, mut form) = fixture(8);
        let mut output = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
        for frame in 1..8 {
            output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        assert_centred(&output, &form, 0);
        let at = form.motion.viewport.left_top() + Vec2::new(form.motion.stride() - 8.0, 45.0);
        draw_fixture(
            &ctx,
            &mut settings,
            &mut form,
            8,
            vec![
                egui::Event::PointerMoved(at),
                wheel_event(egui::MouseWheelUnit::Line, Vec2::new(0.0, -1.0)),
            ],
        );
        output = draw_fixture(&ctx, &mut settings, &mut form, 9, vec![]);
        assert_eq!(settings.username, "synthetic-1");
        assert_centred(&output, &form, 1);
        // Small trackpad increments accumulate, without smoothed input causing
        // a second selection on the following frame or a sizing pass.
        for frame in 10..12 {
            draw_fixture(
                &ctx,
                &mut settings,
                &mut form,
                frame,
                vec![wheel_event(egui::MouseWheelUnit::Point, Vec2::new(-28.0, 0.0))],
            );
        }
        output = draw_fixture(&ctx, &mut settings, &mut form, 12, vec![]);
        assert_eq!(settings.username, "synthetic-2");
        assert_centred(&output, &form, 2);
        assert!(form.pending_connect.is_none() && form.transition.is_none());
        form.managing = true;
        draw_fixture(
            &ctx,
            &mut settings,
            &mut form,
            13,
            vec![wheel_event(egui::MouseWheelUnit::Line, Vec2::new(0.0, -1.0))],
        );
        assert_eq!(settings.username, "synthetic-2");
    }

    #[test]
    fn grid_manager_owns_wheel_keys_and_pointer_until_closed() {
        let ctx = Context::default();
        let (mut settings, mut form) = fixture(8);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let mut manager = crate::ui::grids::Manager::default();
        for frame in 0..8 {
            draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        let at = form.motion.viewport.left_top() + Vec2::new(form.motion.stride() - 8.0, 45.0);
        let page = form.page;
        manager.demo(&mut settings);
        crate::ui::grids::request_open(&ctx);
        // Cover both the pending opening request and an already displayed modal.
        for frame in 8..16 {
            let events = vec![
                egui::Event::PointerMoved(at),
                wheel_event(egui::MouseWheelUnit::Line, Vec2::new(0.0, -1.0)),
                key(egui::Key::ArrowRight),
                key(egui::Key::Enter),
            ];
            draw_fixture_with_manager(
                &ctx,
                &mut settings,
                &mut form,
                frame,
                events,
                Some((&mut manager, runtime.handle())),
            );
            assert_eq!(settings.username, "synthetic-0");
            assert_eq!(form.page, page);
            assert!(!form.motion.moving() && form.pending_connect.is_none() && !form.editing);
            assert!(manager.open);
        }
        let output = draw_fixture_with_manager(&ctx, &mut settings, &mut form, 16, vec![], Some((&mut manager, runtime.handle())));
        let close = text_rect(&output, aurora_i18n::tr!("common-close")).expect("close button").center();
        for (frame, pressed) in [(17, true), (18, false)] {
            draw_fixture_with_manager(
                &ctx,
                &mut settings,
                &mut form,
                frame,
                vec![
                    egui::Event::PointerMoved(close),
                    egui::Event::PointerButton {
                        pos: close,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                Some((&mut manager, runtime.handle())),
            );
        }
        assert!(!manager.open && !crate::ui::grids::is_open(&ctx));
        draw_fixture_with_manager(
            &ctx,
            &mut settings,
            &mut form,
            19,
            vec![
                egui::Event::PointerMoved(at),
                wheel_event(egui::MouseWheelUnit::Line, Vec2::new(0.0, -1.0)),
            ],
            Some((&mut manager, runtime.handle())),
        );
        assert_eq!(
            settings.username, "synthetic-1",
            "normal browsing resumes after closing the manager"
        );
    }

    #[test]
    fn keyboard_and_button_arrows_centre_and_wrap_without_connecting() {
        let ctx = Context::default();
        let (mut settings, mut form) = fixture(8);
        for frame in 0..8 {
            draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        draw_fixture(&ctx, &mut settings, &mut form, 8, vec![key(egui::Key::ArrowLeft)]);
        let output = draw_fixture(&ctx, &mut settings, &mut form, 9, vec![]);
        assert_eq!(settings.username, "synthetic-7");
        assert_centred(&output, &form, 7);
        let layout = super::super::layout::Layout::new(ctx.content_rect(), 8);
        let arrow = egui::pos2(
            form.motion.viewport.right() + layout.peek + layout.gap + 13.0,
            form.motion.viewport.center().y,
        );
        let click = |pressed| {
            vec![
                egui::Event::PointerMoved(arrow),
                egui::Event::PointerButton {
                    pos: arrow,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        draw_fixture(&ctx, &mut settings, &mut form, 10, click(true));
        draw_fixture(&ctx, &mut settings, &mut form, 11, click(false));
        let output = draw_fixture(&ctx, &mut settings, &mut form, 12, vec![]);
        assert_eq!(settings.username, "synthetic-0");
        assert_centred(&output, &form, 0);
        assert!(form.pending_connect.is_none() && form.transition.is_none());
    }

    #[test]
    fn hover_lights_the_surface_without_moving_or_selecting_it() {
        let ctx = Context::default();
        let (mut settings, mut form) = fixture(3);
        let mut before = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
        for frame in 1..8 {
            before = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        let name = text_rect(&before, "synthetic-0").unwrap();
        let at = egui::pos2(name.left() + 100.0, form.motion.viewport.top() + 45.0);
        fn surface_border(shape: &egui::Shape, at: egui::Pos2) -> Option<egui::Color32> {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| surface_border(shape, at)),
                egui::Shape::Rect(rect) if rect.rect.contains(at) && rect.rect.height() > 300.0 && rect.stroke.width == 1.0 => {
                    Some(rect.stroke.color)
                }
                _ => None,
            }
        }
        let border = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|source| surface_border(&source.shape, at))
                .expect("account surface border")
        };
        let original = border(&before);
        let mut after = draw_fixture(&ctx, &mut settings, &mut form, 8, vec![egui::Event::PointerMoved(at)]);
        for frame in 9..16 {
            after = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        assert_ne!(original, border(&after));
        assert_eq!(name, text_rect(&after, "synthetic-0").unwrap());
        assert_eq!(settings.username, "synthetic-1");
        assert_eq!(after.platform_output.cursor_icon, egui::CursorIcon::Default);
        assert!(form.pending_connect.is_none());
    }

    #[test]
    fn side_drag_selects_the_settled_centre_without_connecting_or_changing_preferences() {
        for count in [4, 8] {
            let ctx = Context::default();
            let mut settings = Settings {
                username: "synthetic-0".into(),
                ..Default::default()
            };
            let accounts = (0..count)
                .map(|index| {
                    let mut account = Account::from_settings(&settings, index % 2 == 0, None);
                    account.username = format!("synthetic-{index}");
                    account.start_location = if index % 2 == 0 { "last" } else { "home" }.into();
                    account
                })
                .collect::<Vec<_>>();
            let original = accounts.clone();
            let mut form = LoginForm {
                accounts,
                accounts_loaded: true,
                password: "synthetic-marker".into(),
                ..Default::default()
            };
            for frame in 0..9 {
                draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
            }
            let start = form.motion.viewport.left_center() - Vec2::new(24.0, 0.0);
            draw_fixture(
                &ctx,
                &mut settings,
                &mut form,
                9,
                vec![egui::Event::PointerMoved(start + Vec2::new(80.0, 0.0))],
            );
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            draw_fixture(
                &ctx,
                &mut settings,
                &mut form,
                10,
                vec![egui::Event::PointerMoved(start), button(start, true)],
            );
            assert_eq!(form.motion.offset, 0.0, "approaching the grip must not move the cards on press");
            let moved = start - Vec2::new(200.0, 0.0);
            draw_fixture(&ctx, &mut settings, &mut form, 11, vec![egui::Event::PointerMoved(moved)]);
            draw_fixture(&ctx, &mut settings, &mut form, 12, vec![]);
            assert!(form.motion.offset < -100.0, "cards must follow the held pointer");
            // Release over a card after dragging in the other direction: this
            // must remain browsing, even though that card normally connects.
            let end = start + Vec2::new(200.0, 0.0);
            draw_fixture(&ctx, &mut settings, &mut form, 13, vec![egui::Event::PointerMoved(end)]);
            draw_fixture(&ctx, &mut settings, &mut form, 14, vec![button(end, false)]);
            for frame in 15..50 {
                draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
            }
            assert_eq!(form.page, count - 3);
            assert!(!form.motion.moving());
            assert_eq!(settings.username, format!("synthetic-{}", count - 2));
            assert!(form.password.is_empty());
            assert!(form.pending_connect.is_none() && form.transition.is_none() && !form.editing);
            for (account, before) in form.accounts.iter().zip(&original) {
                assert_eq!(account.key(), before.key());
                assert_eq!(account.remember_password, before.remember_password);
                assert_eq!(account.start_location, before.start_location);
            }
        }
    }

    #[test]
    fn dragging_card_content_browses_but_a_simple_click_still_connects() {
        for count in [4, 8] {
            for area in ["portrait", "name", "place", "button", "surface"] {
                let ctx = Context::default();
                let mut settings = Settings {
                    username: "synthetic-0".into(),
                    ..Default::default()
                };
                let mut form = LoginForm {
                    accounts: (0..count)
                        .map(|index| {
                            let mut account = Account::from_settings(&settings, true, None);
                            account.username = format!("synthetic-{index}");
                            account
                        })
                        .collect(),
                    accounts_loaded: true,
                    password: "synthetic-marker".into(),
                    ..Default::default()
                };
                let mut output = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
                for frame in 1..8 {
                    output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
                }
                let name = text_rect(&output, "synthetic-0").expect("account name");
                let start = match area {
                    "portrait" => form.motion.viewport.left_top() + Vec2::new(120.0, 45.0),
                    "name" => name.center(),
                    "place" => {
                        let label = text_rect(&output, aurora_i18n::tr!("login-accounts-last-place")).expect("place caption");
                        egui::pos2(name.left() + 100.0, label.bottom() + 25.0)
                    }
                    "button" => text_rect(&output, aurora_i18n::tr!("ui-login-log-in"))
                        .expect("connect button")
                        .center(),
                    _ => egui::pos2(form.motion.viewport.left() + 6.0, name.center().y),
                };
                let slot = ((start.x - form.motion.viewport.left()) / form.motion.stride()).floor() as usize;
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                let hover = draw_fixture(&ctx, &mut settings, &mut form, 8, vec![egui::Event::PointerMoved(start)]);
                assert!(
                    !matches!(
                        hover.platform_output.cursor_icon,
                        egui::CursorIcon::Grab | egui::CursorIcon::Grabbing | egui::CursorIcon::Move | egui::CursorIcon::AllScroll
                    ),
                    "hovering {area} must not suggest moving the cards"
                );
                if area != "button" {
                    assert_eq!(hover.platform_output.cursor_icon, egui::CursorIcon::Default);
                }
                draw_fixture(
                    &ctx,
                    &mut settings,
                    &mut form,
                    9,
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                );
                assert_eq!(form.motion.offset, 0.0, "pressing {area} must not move the carousel");
                assert!(form.pending_connect.is_none());
                let left = start - Vec2::new(200.0, 0.0);
                let dragging = draw_fixture(&ctx, &mut settings, &mut form, 10, vec![egui::Event::PointerMoved(left)]);
                assert_eq!(dragging.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
                assert!(form.motion.is_dragging());
                assert!(form.motion.offset < -100.0, "dragging {area} must move the cards");
                // Reverse and release over a card to verify click suppression.
                let right = start + Vec2::new(200.0, 0.0);
                draw_fixture(&ctx, &mut settings, &mut form, 11, vec![egui::Event::PointerMoved(right)]);
                let released = draw_fixture(&ctx, &mut settings, &mut form, 12, vec![button(right, false)]);
                assert!(!form.motion.is_dragging() && form.motion.moving());
                assert_ne!(released.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
                for frame in 13..50 {
                    draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
                }
                assert_eq!(form.page, count - 3);
                assert!(!form.motion.moving());
                assert_eq!(settings.username, format!("synthetic-{}", count - 2));
                assert!(form.password.is_empty());
                assert!(
                    form.pending_connect.is_none() && form.transition.is_none() && !form.editing,
                    "releasing a drag from {area} must not connect"
                );
                draw_fixture(
                    &ctx,
                    &mut settings,
                    &mut form,
                    50,
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                );
                draw_fixture(&ctx, &mut settings, &mut form, 51, vec![button(start, false)]);
                assert_eq!(
                    form.pending_connect,
                    Some((count - 3 + slot) % count),
                    "a simple click on {area} must still connect"
                );
            }
        }
    }

    #[test]
    fn dragging_region_text_selects_text_without_browsing_or_connecting() {
        let ctx = Context::default();
        let mut settings = Settings {
            username: "synthetic-alpha".into(),
            ..Default::default()
        };
        let mut alpha = Account::from_settings(&settings, true, None);
        alpha.start_location = "region".into();
        alpha.start_region = "Synthetic region".into();
        let mut beta = Account::from_settings(&settings, false, None);
        beta.username = "synthetic-beta".into();
        let mut form = LoginForm {
            accounts: vec![alpha, beta],
            accounts_loaded: true,
            ..Default::default()
        };
        form.accounts.extend(fixture(6).1.accounts);
        let mut output = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
        for frame in 1..8 {
            output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        let text = text_rect(&output, "Synthetic region").expect("region text input");
        let start = egui::pos2(text.left() + 4.0, text.center().y);
        let end = start + Vec2::new(60.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        draw_fixture(
            &ctx,
            &mut settings,
            &mut form,
            8,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        draw_fixture(&ctx, &mut settings, &mut form, 9, vec![egui::Event::PointerMoved(end)]);
        draw_fixture(&ctx, &mut settings, &mut form, 10, vec![button(end, false)]);
        assert_eq!(form.motion.offset, 0.0);
        assert_eq!(form.page, 7);
        assert!(ctx.egui_wants_keyboard_input());
        assert!(form.pending_connect.is_none() && form.transition.is_none());
        assert_eq!(form.accounts[0].start_region, "Synthetic region");
        let gap = form.motion.viewport.left_top() + Vec2::new(form.motion.stride() - 8.0, 45.0);
        draw_fixture(
            &ctx,
            &mut settings,
            &mut form,
            11,
            vec![
                egui::Event::PointerMoved(gap),
                wheel_event(egui::MouseWheelUnit::Line, Vec2::new(0.0, -1.0)),
                key(egui::Key::ArrowLeft),
            ],
        );
        assert_eq!(form.page, 7);
        assert_eq!(settings.username, "synthetic-alpha");
    }

    #[test]
    fn wrapped_cards_keep_the_selected_surface_and_fit_the_viewport() {
        let ctx = Context::default();
        let mut settings = Settings {
            username: "synthetic-alpha".into(),
            ..Default::default()
        };
        let mut beta = Account::from_settings(&settings, false, None);
        beta.username = "synthetic-beta".into();
        let mut form = LoginForm {
            accounts: vec![Account::from_settings(&settings, false, None), beta],
            accounts_loaded: true,
            page: 1,
            selection_shown_for: Some((settings.login_uri(), settings.username.to_lowercase())),
            ..Default::default()
        };
        let mut output = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
        for frame in 1..8 {
            output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        let alpha = text_rect(&output, "synthetic-alpha").expect("selected card");
        fn strips(shape: &egui::Shape, result: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| strips(shape, result)),
                egui::Shape::Mesh(mesh) if mesh.vertices.len() == 4 && (mesh.calc_bounds().height() - 2.0).abs() < 0.1 => {
                    result.push(mesh.calc_bounds());
                }
                _ => {}
            }
        }
        let mut accent = Vec::new();
        for source in &output.shapes {
            strips(&source.shape, &mut accent);
        }
        assert_eq!(accent.len(), 1, "one selected-card accent");
        assert!(
            (accent[0].left() - (alpha.left() - 12.0)).abs() < 1.0,
            "accent must follow the selected card: {accent:?}, {alpha:?}"
        );
        assert!(
            form.motion.viewport.contains_rect(accent[0]),
            "last card must not be clipped at rest"
        );
    }

    #[test]
    fn three_full_cards_keep_faded_neighbours_and_preview_click_reveals_before_connecting() {
        let ctx = Context::default();
        let mut settings = Settings {
            username: "synthetic-0".into(),
            ..Default::default()
        };
        let mut form = LoginForm {
            accounts: (0..8)
                .map(|index| {
                    let mut account = Account::from_settings(&settings, true, None);
                    account.username = format!("synthetic-{index}");
                    account
                })
                .collect(),
            accounts_loaded: true,
            password: "synthetic-marker".into(),
            ..Default::default()
        };
        let mut output = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
        for frame in 1..8 {
            output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        for index in [7, 0, 1] {
            assert!(text_rect(&output, &format!("synthetic-{index}")).is_some());
        }
        for index in 2..7 {
            assert!(
                text_rect(&output, &format!("synthetic-{index}")).is_none(),
                "neighbours are faded meshes"
            );
        }
        let layout = super::super::layout::Layout::new(ctx.content_rect(), 8);
        assert_eq!(layout.per_page, 3);
        let at = form.motion.viewport.left_top() + Vec2::new(-layout.peek * 0.6, 45.0);
        let button = |pressed| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        draw_fixture(&ctx, &mut settings, &mut form, 8, button(true));
        draw_fixture(&ctx, &mut settings, &mut form, 9, button(false));
        assert_eq!(settings.username, "synthetic-6");
        assert_eq!(form.page, 6);
        assert_eq!(form.pending_connect, Some(6));
        // Supply only a synthetic in-memory marker, never an OS credential.
        form.password = "synthetic-marker".into();
        for frame in 10..21 {
            output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        assert!(form.transition.is_some());
        assert!(
            output.shapes.iter().any(|source| {
                let egui::Shape::Text(text) = &source.shape else {
                    return false;
                };
                text.galley.text() == "synthetic-6" && source.clip_rect.contains_rect(Rect::from_min_size(text.pos, text.galley.size()))
            }),
            "the chosen name must be fully visible during validation"
        );
    }

    #[test]
    fn saved_cards_omit_password_and_grid_menu_does_not_connect() {
        let ctx = Context::default();
        let mut settings = Settings {
            username: "synthetic-alpha".into(),
            ..Default::default()
        };
        let mut beta = Account::from_settings(&settings, false, None);
        beta.username = "synthetic-beta".into();
        let mut form = LoginForm {
            accounts: vec![Account::from_settings(&settings, false, None), beta],
            accounts_loaded: true,
            ..Default::default()
        };
        let mut output = draw_fixture(&ctx, &mut settings, &mut form, 0, vec![]);
        for frame in 1..8 {
            output = draw_fixture(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        assert!(text_rect(&output, aurora_i18n::tr!("ui-login-password")).is_none());
        assert!(text_rect(&output, aurora_i18n::tr!("login-accounts-title")).is_none());
        assert!(text_rect(&output, &aurora_i18n::tr!("login-accounts-count", count = 1)).is_none());
        assert!(text_rect(&output, aurora_i18n::tr!("ui-login-remember-password")).is_none());
        assert!(text_rect(&output, aurora_i18n::tr!("login-accounts-last-place")).is_some());
        let grid = text_rect(&output, "Second Life").expect("grid selector").center();
        let click = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        draw_fixture(&ctx, &mut settings, &mut form, 8, click(grid, true));
        draw_fixture(&ctx, &mut settings, &mut form, 9, click(grid, false));
        assert!(egui::Popup::is_any_open(&ctx));
        assert!(form.pending_connect.is_none());
        draw_fixture(
            &ctx,
            &mut settings,
            &mut form,
            10,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        output = draw_fixture(&ctx, &mut settings, &mut form, 11, vec![]);
        let name = text_rect(&output, "synthetic-alpha").expect("account name").center();
        draw_fixture(&ctx, &mut settings, &mut form, 12, click(name, true));
        draw_fixture(&ctx, &mut settings, &mut form, 13, click(name, false));
        assert_eq!(form.pending_connect, Some(0));
    }

    #[test]
    fn keyboard_changes_account_reveals_page_and_enter_requests_login() {
        let ctx = Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::default();
        let info = LoginInfo::preview_release("0.3.0");
        let mut settings = Settings {
            username: "synthetic-3".into(),
            ..Default::default()
        };
        let mut form = LoginForm {
            accounts: (0..8)
                .map(|index| {
                    let mut account = Account::from_settings(&settings, true, None);
                    account.username = format!("synthetic-{index}");
                    account.remember_password = true;
                    account
                })
                .collect(),
            accounts_loaded: true,
            ..Default::default()
        };
        let draw = |key: Option<egui::Key>, form: &mut LoginForm, settings: &mut Settings| {
            let _ = super::super::test_output(
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 720.0))),
                        time: Some(0.1),
                        events: key
                            .map(|key| egui::Event::Key {
                                key,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: egui::Modifiers::NONE,
                            })
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui, &p, &icons, None, settings, form, &info, None);
                    },
                ),
            );
        };
        draw(None, &mut form, &mut settings);
        draw(Some(egui::Key::ArrowRight), &mut form, &mut settings);
        assert_eq!(settings.username, "synthetic-4");
        assert_eq!(form.page, 3);
        assert!(form.pending_connect.is_none());
        assert!(form.accounts[3].remember_password && form.accounts[3].remember_mfa);
        draw(Some(egui::Key::Enter), &mut form, &mut settings);
        assert_eq!(form.pending_connect, Some(4));
        form.pending_connect = None;
        form.block_reason = Some(aurora_i18n::tr!("update-login-required"));
        draw(Some(egui::Key::Enter), &mut form, &mut settings);
        assert!(form.pending_connect.is_none());
    }

    #[test]
    fn wide_login_restores_information_and_keeps_actions_near_growing_cards() {
        let mut spacing = Vec::new();
        for size in [Vec2::new(1600.0, 900.0), Vec2::new(2560.0, 1440.0)] {
            let ctx = Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let mut settings = Settings {
                username: "synthetic-alpha".into(),
                ..Default::default()
            };
            let mut beta = Account::from_settings(&settings, false, None);
            beta.username = "synthetic-beta".into();
            let mut form = LoginForm {
                accounts_loaded: true,
                accounts: vec![Account::from_settings(&settings, false, None), beta],
                ..Default::default()
            };
            let mut output = None;
            for frame in 0..8 {
                output = Some(super::super::test_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(frame as f64 / 10.0),
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui,
                            &theme.palette(),
                            &Icons::default(),
                            None,
                            &mut settings,
                            &mut form,
                            &LoginInfo::preview_release("0.3.0"),
                            None,
                        );
                    },
                )));
            }
            let output = output.expect("synthetic frame");
            for label in [
                "Discord",
                "GitHub",
                aurora_i18n::tr!("ui-login-website"),
                aurora_i18n::tr!("ui-login-join-the-community"),
                aurora_i18n::tr!("ui-login-version"),
                aurora_i18n::tr!("ui-login-news"),
                aurora_i18n::tr!("ui-login-second-life-client-rust-vulkan"),
            ] {
                assert!(text_rect(&output, label).is_some(), "missing {label}");
            }
            let alpha = text_rect(&output, "synthetic-alpha").expect("first card");
            let beta = text_rect(&output, "synthetic-beta").expect("second card");
            spacing.push(beta.left() - alpha.left());
            let layout = super::super::layout::Layout::new(Rect::from_min_size(egui::Pos2::ZERO, size), 2);
            assert!(
                text_rect(&output, aurora_i18n::tr!("ui-login-news")).expect("news column").left()
                    > beta.left() + layout.card_width - f32::from(layout.margin)
            );
            let connect = text_rect(&output, aurora_i18n::tr!("ui-login-log-in")).expect("connect button");
            let add = text_rect(&output, aurora_i18n::tr!("login-accounts-add")).expect("add button");
            assert!(add.top() > connect.bottom() && add.top() - connect.bottom() < 100.0);
        }
        assert!(spacing[1] > spacing[0] * 1.15 && spacing[1] <= spacing[0] * 1.25);
    }

    #[test]
    fn short_window_keeps_card_controls_and_add_button_visible() {
        let ctx = Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::default();
        let info = LoginInfo::preview_release("0.3.0");
        let logo = ctx.load_texture(
            "synthetic-logo",
            egui::ColorImage::filled([1, 1], p.violet),
            egui::TextureOptions::LINEAR,
        );
        let mut settings = Settings {
            username: "synthetic-alpha".into(),
            ..Default::default()
        };
        let mut second = Account::from_settings(&settings, false, None);
        second.username = "synthetic-beta".into();
        let mut form = LoginForm {
            accounts: vec![Account::from_settings(&settings, false, None), second],
            accounts_loaded: true,
            ..Default::default()
        };
        let mut last = None;
        for frame in 0..8 {
            last = Some(super::super::test_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 733.0))),
                    time: Some(frame as f64 / 10.0),
                    ..Default::default()
                },
                |ui| {
                    show(ui, &p, &icons, Some(&logo), &mut settings, &mut form, &info, None);
                },
            )));
        }
        let output = last.expect("synthetic frame");
        for label in [
            aurora_i18n::tr!("ui-login-grid"),
            aurora_i18n::tr!("ui-login-start"),
            aurora_i18n::tr!("ui-login-log-in"),
            aurora_i18n::tr!("login-accounts-add"),
        ] {
            let positions: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|source| {
                    let egui::Shape::Text(text) = &source.shape else {
                        return None;
                    };
                    (text.galley.text() == label).then_some((source.clip_rect, Rect::from_min_size(text.pos, text.galley.size())))
                })
                .collect();
            assert!(!positions.is_empty(), "missing {label}");
            assert!(
                positions
                    .iter()
                    .all(|(clip, text)| clip.contains_rect(*text) && text.bottom() < 663.0),
                "clipped {label}: {positions:?}"
            );
        }
    }

    #[test]
    fn first_account_mfa_form_keeps_submit_visible() {
        let ctx = Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::default();
        let info = LoginInfo::preview_release("0.3.0");
        let logo = ctx.load_texture(
            "synthetic-logo",
            egui::ColorImage::filled([1, 1], p.violet),
            egui::TextureOptions::LINEAR,
        );
        let mut settings = Settings {
            username: "synthetic-alpha".into(),
            ..Default::default()
        };
        let mut form = LoginForm {
            accounts_loaded: true,
            show_mfa: true,
            ..Default::default()
        };
        let mut output = None;
        for frame in 0..8 {
            output = Some(super::super::test_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 733.0))),
                    time: Some(frame as f64 / 10.0),
                    ..Default::default()
                },
                |ui| {
                    show(ui, &p, &icons, Some(&logo), &mut settings, &mut form, &info, None);
                },
            )));
        }
        let output = output.expect("synthetic frame");
        let submit = text_rect(&output, aurora_i18n::tr!("ui-login-log-in")).expect("submit button");
        assert!(submit.bottom() < 653.0);
        assert!(output.shapes.iter().any(|source| matches!(&source.shape, egui::Shape::Text(text)
            if text.galley.text() == aurora_i18n::tr!("ui-login-log-in") && source.clip_rect.contains_rect(submit))));
    }

    /// Adding a second account: from the submit of the Add form to its MFA
    /// challenge, the login form keeps the new account's identity. The
    /// carousel used to re-select a saved card on the first frame without
    /// the dialog (editing off while the request ran), so the challenge
    /// showed, forgot and resubmitted the first account.
    #[test]
    fn adding_an_account_keeps_its_identity_through_the_mfa_challenge() {
        let ctx = Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::default();
        let info = LoginInfo::preview_release("0.3.0");
        let (mut settings, mut form) = fixture(1);
        let mut frame = 0;
        let mut run = |settings: &mut Settings, form: &mut LoginForm| {
            for _ in 0..3 {
                frame += 1;
                let _ = super::super::test_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 720.0))),
                        time: Some(frame as f64 / 30.0),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui, &p, &icons, None, settings, form, &info, None);
                    },
                ));
            }
        };
        run(&mut settings, &mut form);
        assert_eq!(form.selection_shown_for.as_ref().map(|key| key.1.as_str()), Some("synthetic-0"));
        // what the Add button prepares, then the typed identity
        form.add_return = Some((settings.login_uri(), settings.username.clone()));
        form.adding = true;
        form.editing = true;
        settings.username = "synthetic-new".into();
        form.password = "synthetic-input".into();
        run(&mut settings, &mut form);
        assert_eq!(settings.username, "synthetic-new");
        // the dialog's submit closes the editor; the request is in flight
        form.editing = false;
        form.busy = true;
        run(&mut settings, &mut form);
        assert_eq!(settings.username, "synthetic-new", "a saved card replaced the account being added");
        assert_eq!(form.password, "synthetic-input");
        // the server asks for the second factor (NetEvent::LoginFailed)
        form.busy = false;
        form.show_mfa = true;
        run(&mut settings, &mut form);
        assert_eq!(settings.username, "synthetic-new");
        assert!(form.adding && form.editing, "the Add form asks for the code");
        assert_eq!(form.password, "synthetic-input");
        assert_eq!(form.accounts.len(), 1, "the new account is saved only after its login");
    }

    #[test]
    fn switching_accounts_preserves_other_password_preferences() {
        let mut settings = Settings::default();
        let mut form = LoginForm {
            accounts: vec![
                Account {
                    username: "synthetic-alpha".into(),
                    uri: "test-grid".into(),
                    remember_password: true,
                    remember_mfa: true,
                    start_location: "region".into(),
                    start_region: "Synthetic region".into(),
                    agent: None,
                    last_place: None,
                },
                Account {
                    username: "synthetic-beta".into(),
                    uri: "test-grid".into(),
                    remember_password: false,
                    remember_mfa: false,
                    start_location: "home".into(),
                    start_region: String::new(),
                    agent: None,
                    last_place: None,
                },
            ],
            stored: Some("synthetic-marker".into()),
            password: "synthetic-input".into(),
            ..Default::default()
        };
        select(&mut settings, &mut form, 1);
        assert!(form.stored.is_none());
        assert!(form.password.is_empty());
        assert!(form.accounts[0].remember_password);
        assert!(!form.accounts[1].remember_password);
        assert!(!settings.remember_password);
        assert_eq!(settings.start_location, "home");
        assert!(!form.remember_mfa);
        select(&mut settings, &mut form, 0);
        assert!(settings.remember_password);
        assert!(form.remember_mfa);
        assert_eq!(settings.start_location, "region");
        assert_eq!(settings.start_region, "Synthetic region");
        assert_eq!(settings.login_uri(), "test-grid");
        assert_eq!(form.accounts.len(), 2);
    }
}
