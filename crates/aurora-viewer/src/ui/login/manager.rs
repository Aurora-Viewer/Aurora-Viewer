//! Account manager: portrait rows, credential summaries and scoped confirmation.
//! Account identities, rather than row indices, survive catalogue refreshes.
use super::{LoginForm, field};
use crate::login_accounts::Account;
use crate::settings::{GridChoice, Settings};
use crate::theme::Palette;
use crate::ui::{
    icons::Icons,
    widgets::{self, ActionKind},
};
use egui::{Context, Id, RichText, Sense, Vec2};

#[derive(Default)]
pub struct State {
    pub search: String,
    pending: Option<Confirmation>,
    drag: super::manager_drag::State,
    order_error: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Password,
    Mfa,
    Remove,
}

struct Confirmation {
    account: Account,
    action: Action,
}

impl State {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_dragging()
    }

    pub fn demo_events(&mut self, frame: u64) -> Vec<egui::Event> {
        self.drag.demo_events(frame)
    }

    pub fn preview_remove(&mut self, account: &Account) {
        self.pending = Some(Confirmation {
            account: account.clone(),
            action: Action::Remove,
        });
    }
}

fn grid_name(account: &Account) -> &'static str {
    match crate::login_accounts::grid(&account.uri) {
        GridChoice::SecondLife => "Second Life",
        GridChoice::SecondLifeBeta => "Aditi",
        GridChoice::Custom => aurora_i18n::tr!("ui-login-other-grid"),
    }
}

fn start_name(account: &Account) -> &str {
    match account.start_location.as_str() {
        "home" => aurora_i18n::tr!("common-home"),
        "region" if !account.start_region.is_empty() => &account.start_region,
        "region" => aurora_i18n::tr!("ui-login-region"),
        _ => aurora_i18n::tr!("ui-login-last-location"),
    }
}

fn matches(account: &Account, search: &str) -> bool {
    search.is_empty()
        || format!("{} {} {}", account.username, grid_name(account), start_name(account))
            .to_lowercase()
            .contains(search)
}

fn summary(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, remembered: bool) {
    ui.horizontal(|ui| {
        widgets::glyph(ui, icon, if remembered { p.teal } else { p.muted }, 15.0);
        ui.label(RichText::new(label).size(11.5).color(if remembered { p.teal } else { p.muted }));
    });
}

pub fn show(ctx: &Context, p: &Palette, icons: &Icons, settings: &mut Settings, form: &mut LoginForm) {
    show_impl(ctx, p, icons, settings, form, std::env::var_os("AURORA_DEMO").is_some());
}

fn show_impl(ctx: &Context, p: &Palette, icons: &Icons, settings: &mut Settings, form: &mut LoginForm, offline: bool) {
    let width = (ctx.content_rect().width() - 112.0).clamp(280.0, 676.0);
    let mut close = false;
    let mut cancel = false;
    let mut confirmed = false;
    let mut moved = None;
    form.manager
        .drag
        .begin(ctx, form.manager.pending.is_none() && !form.busy && form.pending_connect.is_none());
    let response = widgets::Modal::new(Id::unique("manage_login_accounts"))
        .login_surface(p)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                widgets::glyph(ui, "users-group", p.violet_light, 34.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("login-accounts-manage"))
                            .size(21.0)
                            .strong()
                            .color(p.ink),
                    );
                    ui.label(
                        RichText::new(aurora_i18n::tr!("login-accounts-count", count = form.accounts.len()))
                            .size(11.5)
                            .color(p.muted),
                    );
                });
            });
            ui.label(
                RichText::new(aurora_i18n::tr!("login-accounts-manager-description"))
                    .size(12.0)
                    .color(p.muted),
            );
            ui.horizontal(|ui| {
                widgets::glyph(ui, "grip-vertical", p.violet_light, 17.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("login-accounts-reorder"))
                        .size(11.5)
                        .color(p.violet_pale),
                );
            });
            if form.manager.order_error {
                ui.label(RichText::new(aurora_i18n::tr!("login-accounts-reorder-failed")).color(p.danger));
            }
            ui.add_space(4.0);
            let pending = form.manager.pending.is_some();
            ui.add_enabled_ui(
                !pending && !(form.manager.drag.is_dragging() && ctx.input(|i| i.pointer.primary_down())),
                |ui| {
                    field(
                        ui,
                        p,
                        icons,
                        "account_manager_search",
                        "search-filled",
                        aurora_i18n::tr!("login-accounts-search"),
                        &mut form.manager.search,
                        None,
                    );
                },
            );
            let search = form.manager.search.trim().to_lowercase();
            let accounts: Vec<_> = form.accounts.iter().filter(|account| matches(account, &search)).cloned().collect();
            let reserved = if pending { 450.0 } else { 303.0 };
            let list_height = (ctx.content_rect().height() - reserved).clamp(100.0, 440.0);
            let mut list_painter = None;
            let scroll = egui::ScrollArea::vertical()
                .id_salt("managed_accounts_list")
                .vertical_scroll_offset(form.manager.drag.scroll)
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: egui::scroll_area::DragScroll::Never,
                    ..Default::default()
                })
                .max_height(list_height)
                .show(ui, |ui| {
                    // beside the scroll bar's strip, when it shows
                    let width = ui.available_width().min(width);
                    ui.set_width(width);
                    list_painter = Some(ui.painter().clone());
                    if accounts.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            widgets::glyph(ui, "search-filled", p.muted, 28.0);
                            ui.label(RichText::new(aurora_i18n::tr!("login-accounts-no-results")).color(p.muted));
                        });
                        ui.add_space(20.0);
                    }
                    ui.add_enabled_ui(!pending, |ui| {
                        for account in accounts {
                            let key = account.key();
                            let dragging = form.manager.drag.is_dragging() && ctx.input(|i| i.pointer.primary_down());
                            let row = ui.scope_builder(egui::UiBuilder::new().id_salt(key.clone()).sense(Sense::click_and_drag()), |ui| {
                                if form.manager.drag.is_source(&key) {
                                    ui.multiply_opacity(0.6);
                                }
                                widgets::surface(ui, p, |ui| {
                                    ui.set_width(width - 20.0);
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    ui.horizontal_top(|ui| {
                                        widgets::glyph(ui, "grip-vertical", p.muted, 20.0);
                                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                                        widgets::avatar_picture(
                                            ui,
                                            p,
                                            rect,
                                            account.agent.and_then(|agent| form.portraits.get(&agent)),
                                            &account.username,
                                            false,
                                        );
                                        ui.vertical(|ui| {
                                            let remaining = ui.available_width();
                                            ui.horizontal_top(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.set_width((remaining - 214.0).max(160.0));
                                                    ui.add(widgets::elided_label(
                                                        ui,
                                                        RichText::new(&account.username).size(17.0).strong().color(p.ink),
                                                        ui.available_width(),
                                                    ));
                                                    ui.label(RichText::new(grid_name(&account)).size(11.5).color(p.violet_pale));
                                                    ui.add(widgets::elided_label(
                                                        ui,
                                                        RichText::new(aurora_i18n::tr!(
                                                            "login-accounts-start-summary",
                                                            location = start_name(&account)
                                                        ))
                                                        .size(11.0)
                                                        .color(p.muted),
                                                        ui.available_width(),
                                                    ));
                                                });
                                                ui.vertical(|ui| {
                                                    ui.set_width(206.0);
                                                    summary(
                                                        ui,
                                                        p,
                                                        "lock-filled",
                                                        if account.remember_password {
                                                            aurora_i18n::tr!("login-accounts-remembered")
                                                        } else {
                                                            aurora_i18n::tr!("login-accounts-password-not-saved")
                                                        },
                                                        account.remember_password,
                                                    );
                                                    summary(
                                                        ui,
                                                        p,
                                                        "shield-lock-filled",
                                                        if account.remember_mfa {
                                                            aurora_i18n::tr!("login-accounts-mfa-saved")
                                                        } else {
                                                            aurora_i18n::tr!("login-accounts-mfa-not-saved")
                                                        },
                                                        account.remember_mfa,
                                                    );
                                                });
                                            });
                                        });
                                    });
                                    ui.add_space(3.0);
                                    ui.add_enabled_ui(!dragging, |ui| {
                                        ui.horizontal(|ui| {
                                            if widgets::action_button_if(
                                                ui,
                                                p,
                                                "lock-filled",
                                                aurora_i18n::tr!("login-accounts-forget-password"),
                                                ActionKind::Flat,
                                                Vec2::new(0.0, 30.0),
                                                account.remember_password,
                                            )
                                            .clicked()
                                            {
                                                form.manager.pending = Some(Confirmation {
                                                    account: account.clone(),
                                                    action: Action::Password,
                                                });
                                            }
                                            if widgets::action_button_if(
                                                ui,
                                                p,
                                                "shield-lock-filled",
                                                aurora_i18n::tr!("login-accounts-forget-mfa"),
                                                ActionKind::Flat,
                                                Vec2::new(0.0, 30.0),
                                                account.remember_mfa,
                                            )
                                            .clicked()
                                            {
                                                form.manager.pending = Some(Confirmation {
                                                    account: account.clone(),
                                                    action: Action::Mfa,
                                                });
                                            }
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if widgets::action_button(
                                                    ui,
                                                    p,
                                                    "trash-filled",
                                                    aurora_i18n::tr!("login-accounts-remove"),
                                                    ActionKind::FlatDanger,
                                                    Vec2::new(0.0, 30.0),
                                                )
                                                .clicked()
                                                {
                                                    form.manager.pending = Some(Confirmation {
                                                        account: account.clone(),
                                                        action: Action::Remove,
                                                    });
                                                }
                                            });
                                        });
                                    });
                                });
                            });
                            form.manager.drag.row(key, &row.response);
                            ui.add_space(4.0);
                        }
                    });
                });
            form.manager.drag.scroll = scroll.state.unclamped_offset().y;
            if let Some(painter) = list_painter {
                moved = form.manager.drag.finish(ctx, p, &painter, scroll.inner_rect);
            }
            if let Some(pending) = &form.manager.pending {
                ui.add_space(2.0);
                widgets::surface(ui, p, |ui| {
                    ui.set_width(width - 20.0);
                    ui.horizontal(|ui| {
                        widgets::glyph(ui, "alert-triangle-filled", p.warn, 22.0);
                        ui.label(
                            RichText::new(aurora_i18n::tr!("login-accounts-confirm-title"))
                                .strong()
                                .color(p.ink),
                        );
                    });
                    ui.label(match pending.action {
                        Action::Password => aurora_i18n::tr!("login-accounts-confirm-password", name = pending.account.username.as_str()),
                        Action::Mfa => aurora_i18n::tr!("login-accounts-confirm-mfa", name = pending.account.username.as_str()),
                        Action::Remove => aurora_i18n::tr!("login-accounts-confirm-remove", name = pending.account.username.as_str()),
                    });
                    ui.horizontal(|ui| {
                        cancel = widgets::action_button(
                            ui,
                            p,
                            "x",
                            aurora_i18n::tr!("common-cancel"),
                            ActionKind::Flat,
                            Vec2::new(0.0, 32.0),
                        )
                        .clicked();
                        confirmed = widgets::action_button(
                            ui,
                            p,
                            if pending.action == Action::Remove {
                                "trash-filled"
                            } else {
                                "check"
                            },
                            match pending.action {
                                Action::Password => aurora_i18n::tr!("login-accounts-forget-password"),
                                Action::Mfa => aurora_i18n::tr!("login-accounts-forget-mfa"),
                                Action::Remove => aurora_i18n::tr!("login-accounts-remove"),
                            },
                            ActionKind::Danger,
                            Vec2::new(0.0, 32.0),
                        )
                        .clicked();
                    });
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("login-accounts-local-only"))
                        .size(11.0)
                        .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = widgets::action_button(
                        ui,
                        p,
                        "x",
                        aurora_i18n::tr!("common-close"),
                        ActionKind::Flat,
                        Vec2::new(90.0, 32.0),
                    )
                    .clicked();
                });
            });
        });
    if let Some(key) = form.manager.drag.source()
        && let Some(account) = form.accounts.iter().find(|account| account.key() == *key)
        && let Some(at) = ctx.input(|i| i.pointer.interact_pos())
    {
        let screen = ctx.content_rect();
        let pos = egui::pos2(
            (at.x + 16.0).min(screen.right() - 282.0).max(screen.left()),
            (at.y + 20.0).min(screen.bottom() - 90.0).max(screen.top()),
        );
        egui::Area::new(Id::unique("moving_login_account"))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                super::card(ui, p, 10, true, |ui| {
                    ui.set_width(250.0);
                    ui.horizontal(|ui| {
                        widgets::glyph(ui, "grip-vertical", p.violet_pale, 22.0);
                        ui.vertical(|ui| {
                            ui.add(widgets::elided_label(
                                ui,
                                RichText::new(&account.username).size(16.0).strong().color(p.ink),
                                218.0,
                            ));
                            ui.label(RichText::new(grid_name(account)).size(11.5).color(p.muted));
                        });
                    });
                });
            });
    }
    if let Some(moved) = moved {
        reorder(ctx, settings, form, moved, offline);
    }
    if cancel {
        form.manager.pending = None;
    }
    if confirmed && let Some(pending) = form.manager.pending.take() {
        apply(settings, form, pending, offline);
    }
    if close || response.should_close() {
        if response.should_close() && form.manager.pending.is_some() {
            form.manager.pending = None;
        } else {
            form.managing = false;
            form.manager.pending = None;
            form.manager.drag.begin(ctx, false);
        }
    }
}

fn reorder(ctx: &Context, settings: &mut Settings, form: &mut LoginForm, moved: super::manager_drag::Move, offline: bool) {
    let accounts = if offline {
        let mut accounts = form.accounts.clone();
        crate::login_accounts::move_in(&mut accounts, &moved.from, &moved.target, moved.after);
        Ok(accounts)
    } else {
        crate::login_accounts::move_relative(&moved.from, &moved.target, moved.after)
    };
    match accounts {
        Ok(accounts) => {
            form.accounts = accounts;
            let key = (settings.login_uri(), settings.username.trim().to_lowercase());
            let selected = form.accounts.iter().position(|account| account.key() == key);
            if let Some(index) = selected {
                form.accounts[index].apply(settings);
            }
            let layout = super::layout::Layout::new(ctx.content_rect(), form.accounts.len());
            form.motion.stop();
            form.page = if layout.paginated {
                form.accounts
                    .iter()
                    .position(|account| account.key() == key)
                    .map_or(0, |index| (index + form.accounts.len() - layout.per_page / 2) % form.accounts.len())
            } else {
                0
            };
            form.selection_shown_for = selected.map(|_| key);
            form.manager.order_error = false;
            ctx.request_repaint();
        }
        Err(_) => form.manager.order_error = true,
    }
}

fn apply(settings: &mut Settings, form: &mut LoginForm, pending: Confirmation, offline: bool) {
    let Some(index) = form.accounts.iter().position(|account| account.key() == pending.account.key()) else {
        return;
    };
    let account = form.accounts[index].clone();
    let current = account.key() == (settings.login_uri(), settings.username.trim().to_lowercase());
    if !offline {
        if pending.action != Action::Mfa {
            crate::credentials::forget(&account.uri, &account.username);
        }
        if pending.action != Action::Password {
            crate::credentials::forget_mfa(&account.uri, &account.username);
        }
    }
    match pending.action {
        Action::Remove => {
            if offline {
                form.accounts.remove(index);
            } else {
                form.accounts = crate::login_accounts::remove(&account);
            }
            form.pending_connect = None;
            if current {
                settings.username.clear();
                settings.remember_password = false;
                form.stored = None;
                form.stored_for = None;
                form.password.clear();
                form.remember_mfa = false;
                form.mfa_authorization = None;
                form.mfa_for = None;
                form.mfa_token.clear();
            }
        }
        Action::Password | Action::Mfa => {
            if offline {
                if pending.action == Action::Mfa {
                    form.accounts[index].remember_mfa = false;
                } else {
                    form.accounts[index].remember_password = false;
                }
            } else {
                form.accounts = crate::login_accounts::forget_preference(&account, pending.action == Action::Mfa);
            }
            if current {
                if pending.action == Action::Mfa {
                    form.remember_mfa = false;
                    form.mfa_authorization = None;
                    form.mfa_token.clear();
                } else {
                    settings.remember_password = false;
                    form.stored = None;
                    form.password.clear();
                }
            }
        }
    }
    if current && !offline {
        settings.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Settings, LoginForm) {
        let settings = Settings {
            username: "synthetic-0".into(),
            remember_password: true,
            ..Default::default()
        };
        let form = LoginForm {
            accounts: (0..8)
                .map(|index| {
                    let mut account = Account::from_settings(&settings, index % 2 == 0, None);
                    account.username = format!("synthetic-{index}");
                    account
                })
                .collect(),
            managing: true,
            stored: Some("synthetic-marker".into()),
            password: "synthetic-input".into(),
            remember_mfa: true,
            ..Default::default()
        };
        (settings, form)
    }

    fn draw(ctx: &Context, settings: &mut Settings, form: &mut LoginForm, frame: usize, events: Vec<egui::Event>) -> egui::FullOutput {
        let theme = crate::theme::Theme::default();
        theme.apply(ctx, 1.0);
        super::super::test_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 720.0))),
                time: Some(frame as f64 / 30.0),
                events,
                ..Default::default()
            },
            |_ui| {
                show_impl(ctx, &theme.palette(), &Icons::default(), settings, form, true);
            },
        ))
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn label(output: &egui::FullOutput, value: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|source| {
                let egui::Shape::Text(text) = &source.shape else {
                    return None;
                };
                (text.galley.text() == value).then(|| egui::Rect::from_min_size(text.pos, text.galley.size()))
            })
            .expect("synthetic label")
    }

    #[test]
    fn dragging_rows_changes_order_without_changing_identity_or_credentials() {
        for area in ["portrait", "name", "surface", "button"] {
            let ctx = Context::default();
            let (mut settings, mut form) = fixture();
            let original = form.accounts.clone();
            let mut output = draw(&ctx, &mut settings, &mut form, 0, vec![]);
            for frame in 1..8 {
                output = draw(&ctx, &mut settings, &mut form, frame, vec![]);
            }
            let source = form.manager.drag.rows[0].1;
            let target = form.manager.drag.rows[1].1;
            let start = match area {
                "name" => label(&output, "synthetic-0").center(),
                "button" => label(&output, aurora_i18n::tr!("login-accounts-forget-password")).center(),
                "surface" => source.left_top() + Vec2::new(5.0, 45.0),
                _ => source.left_top() + Vec2::new(55.0, 45.0),
            };
            let end = target.left_bottom() + Vec2::new(55.0, -20.0);
            draw(
                &ctx,
                &mut settings,
                &mut form,
                8,
                vec![egui::Event::PointerMoved(start), button(start, true)],
            );
            assert!(!form.manager.is_dragging(), "a press is not yet a drag");
            draw(&ctx, &mut settings, &mut form, 9, vec![egui::Event::PointerMoved(end)]);
            let held = draw(&ctx, &mut settings, &mut form, 10, vec![]);
            assert!(form.manager.is_dragging(), "drag from {area}");
            assert_eq!(held.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
            assert_eq!(form.accounts[0].username, "synthetic-0", "save only on release");
            let released = draw(&ctx, &mut settings, &mut form, 11, vec![button(end, false)]);
            assert_eq!(form.accounts[0].username, "synthetic-1", "drop from {area}");
            assert_eq!(form.accounts[1].username, "synthetic-0");
            assert!(!form.manager.is_dragging());
            assert_ne!(released.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
            assert!(form.manager.pending.is_none() && form.pending_connect.is_none());
            assert_eq!(settings.username, "synthetic-0");
            assert_eq!(form.stored.as_deref(), Some("synthetic-marker"));
            assert_eq!(form.password, "synthetic-input");
            assert!(form.remember_mfa && settings.remember_password);
            for account in &form.accounts {
                let old = original.iter().find(|old| old.key() == account.key()).unwrap();
                assert_eq!(serde_json::to_value(account).unwrap(), serde_json::to_value(old).unwrap());
            }
        }
    }

    #[test]
    fn escape_and_outside_release_cancel_ordering_and_simple_buttons_still_work() {
        let ctx = Context::default();
        let (mut settings, mut form) = fixture();
        draw(&ctx, &mut settings, &mut form, 0, vec![]);
        for frame in 1..8 {
            draw(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        let start = form.manager.drag.rows[0].1.left_top() + Vec2::new(55.0, 45.0);
        let end = form.manager.drag.rows[1].1.center();
        draw(
            &ctx,
            &mut settings,
            &mut form,
            8,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        draw(&ctx, &mut settings, &mut form, 9, vec![egui::Event::PointerMoved(end)]);
        draw(
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
        assert!(form.managing && !form.manager.is_dragging());
        draw(&ctx, &mut settings, &mut form, 11, vec![button(end, false)]);
        assert_eq!(form.accounts[0].username, "synthetic-0");
        draw(
            &ctx,
            &mut settings,
            &mut form,
            12,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        draw(&ctx, &mut settings, &mut form, 13, vec![egui::Event::PointerMoved(end)]);
        let outside = egui::pos2(2.0, 2.0);
        let output = draw(
            &ctx,
            &mut settings,
            &mut form,
            14,
            vec![egui::Event::PointerMoved(outside), button(outside, false)],
        );
        assert_eq!(form.accounts[0].username, "synthetic-0");
        assert!(form.managing && !form.manager.is_dragging());
        let forget = label(&output, aurora_i18n::tr!("login-accounts-forget-password")).center();
        draw(
            &ctx,
            &mut settings,
            &mut form,
            15,
            vec![egui::Event::PointerMoved(forget), button(forget, true)],
        );
        draw(&ctx, &mut settings, &mut form, 16, vec![button(forget, false)]);
        assert_eq!(form.manager.pending.as_ref().unwrap().account.username, "synthetic-0");
        assert!(form.stored.is_some() && form.accounts[0].remember_password);
    }

    #[test]
    fn dragging_near_the_list_edge_scrolls_without_saving_until_release() {
        let ctx = Context::default();
        let (mut settings, mut form) = fixture();
        for frame in 0..8 {
            draw(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        let start = form.manager.drag.rows[0].1.left_top() + Vec2::new(55.0, 45.0);
        // Three rows almost fill this viewport; its lower edge is below row 2.
        let end = egui::pos2(start.x, form.manager.drag.rows[0].1.top() + 414.0);
        draw(
            &ctx,
            &mut settings,
            &mut form,
            8,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        draw(&ctx, &mut settings, &mut form, 9, vec![egui::Event::PointerMoved(end)]);
        for frame in 10..20 {
            draw(&ctx, &mut settings, &mut form, frame, vec![]);
        }
        assert!(form.manager.drag.scroll > 20.0);
        assert_eq!(form.accounts[0].username, "synthetic-0");
        draw(&ctx, &mut settings, &mut form, 20, vec![button(end, false)]);
        assert!(!form.manager.is_dragging());
        assert!(form.manager.pending.is_none());
    }

    #[test]
    fn confirmation_targets_identity_after_rows_change_and_scopes_mfa_reset() {
        let mut settings = Settings {
            username: "synthetic-alpha".into(),
            remember_password: true,
            ..Default::default()
        };
        let alpha = Account::from_settings(&settings, true, None);
        let mut beta = alpha.clone();
        beta.username = "synthetic-beta".into();
        let mut form = LoginForm {
            accounts: vec![beta, alpha.clone()],
            stored: Some("synthetic-marker".into()),
            remember_mfa: true,
            ..Default::default()
        };
        apply(
            &mut settings,
            &mut form,
            Confirmation {
                account: alpha.clone(),
                action: Action::Mfa,
            },
            true,
        );
        assert!(form.accounts[0].remember_mfa);
        assert!(!form.accounts[1].remember_mfa);
        assert!(form.accounts[1].remember_password);
        assert!(form.stored.is_some());
        assert!(!form.remember_mfa);
        assert!(matches(&form.accounts[0], "SYNTHETIC-BETA".to_lowercase().as_str()));
        apply(
            &mut settings,
            &mut form,
            Confirmation {
                account: alpha.clone(),
                action: Action::Remove,
            },
            true,
        );
        assert_eq!(form.accounts.len(), 1);
        assert_eq!(form.accounts[0].username, "synthetic-beta");
        apply(
            &mut settings,
            &mut form,
            Confirmation {
                account: alpha,
                action: Action::Remove,
            },
            true,
        );
        assert_eq!(form.accounts.len(), 1);
    }
}
