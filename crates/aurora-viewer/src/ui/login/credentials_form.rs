//! Full first-account/Add form, and a compact prompt for an existing account.
use super::{LoginForm, REMEMBERED_PASSWORD_MASK, field};
use crate::login_accounts::Account;
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::{
    icons::Icons,
    widgets::{self, ActionKind},
};
use egui::{Id, RichText, Vec2};

pub(super) fn connection_progress(ui: &mut egui::Ui, p: &Palette, progress: Option<(&str, f32)>) {
    let (message, fraction) = progress.unwrap_or((aurora_i18n::tr!("app-connecting"), 0.05));
    let width = ui.available_width();
    ui.add(widgets::elided_label(
        ui,
        RichText::new(message).size(11.5).color(p.violet_pale),
        width,
    ));
    widgets::progress(ui, p, "account_connection_progress", fraction, width, "", None);
}

pub fn content(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    settings: &mut Settings,
    form: &mut LoginForm,
    new_account: bool,
    progress: Option<(&str, f32)>,
) -> bool {
    let grids_open = crate::ui::grids::is_open(ui.ctx());
    let enter = !grids_open && !egui::Popup::is_any_open(ui.ctx()) && widgets::enter_pressed(ui);
    let can = ui
        .add_enabled_ui(!form.busy && !grids_open, |ui| {
            ui.label(
                RichText::new(if new_account {
                    aurora_i18n::tr!("login-accounts-add")
                } else {
                    &settings.username
                })
                .size(20.0)
                .strong()
                .color(p.ink),
            );
            ui.add_space(10.0);
            let user = if new_account {
                let mut options = Account::from_settings(settings, form.remember_mfa, None);
                // the name typed so far: an account stored here offers its
                // favourites (FSPanelLogin::onUsernameTextChanged)
                let key = options.key();
                let mut favorites = super::StartFavorites::of(&form.favorites, &form.start_favorite, &key);
                let edited = super::carousel::controls(ui, &mut options, &mut form.start_slurl, &mut favorites, &settings.grids);
                favorites.keep(&mut form.start_favorite, key);
                if edited {
                    let changed_grid = options.uri != settings.login_uri();
                    options.apply(settings);
                    if changed_grid {
                        form.stored = None;
                        form.stored_for = None;
                        form.password.clear();
                        form.mfa_authorization = None;
                        form.mfa_for = None;
                        form.mfa_token.clear();
                        form.show_mfa = false;
                        form.error = None;
                        form.tos_message = None;
                        form.agree_tos = false;
                        form.critical_message = None;
                        form.read_critical = false;
                        form.forget_next_server();
                    }
                }
                ui.add_space(8.0);
                super::caption(ui, p, aurora_i18n::tr!("common-username"));
                let user = field(
                    ui,
                    p,
                    icons,
                    "new_account_user",
                    "user-filled",
                    aurora_i18n::tr!("extra-first-last-or-username"),
                    &mut settings.username,
                    None,
                );
                if user.changed() {
                    form.show_mfa = false;
                    form.mfa_token.clear();
                    form.error = None;
                    form.tos_message = None;
                    form.agree_tos = false;
                    form.critical_message = None;
                    form.read_critical = false;
                    form.forget_next_server();
                }
                Some(user)
            } else {
                None
            };
            let password = if new_account || !form.show_mfa || (form.stored.is_none() && form.password.is_empty()) {
                super::caption(ui, p, aurora_i18n::tr!("ui-login-password"));
                let response = field(
                    ui,
                    p,
                    icons,
                    "account_login_password",
                    "lock-filled",
                    if form.stored.is_some() { REMEMBERED_PASSWORD_MASK } else { "" },
                    &mut form.password,
                    Some(&mut form.show_password),
                );
                widgets::switch_labeled(
                    ui,
                    p,
                    &mut settings.remember_password,
                    aurora_i18n::tr!("ui-login-remember-password"),
                )
                .on_hover_text(super::remember_password_hint());
                Some(response)
            } else {
                None
            };
            let mfa = if form.show_mfa {
                ui.add_space(8.0);
                super::caption(ui, p, aurora_i18n::tr!("ui-login-authentication-code-2fa"));
                let response = field(
                    ui,
                    p,
                    icons,
                    "account_mfa",
                    "shield-lock-filled",
                    "000000",
                    &mut form.mfa_token,
                    None,
                );
                widgets::checkbox(ui, p, &mut form.remember_mfa, aurora_i18n::tr!("login-accounts-remember-mfa"));
                ui.label(RichText::new(aurora_i18n::tr!("login-accounts-mfa-once")).size(11.0).color(p.muted));
                Some(response)
            } else {
                None
            };
            if !form.focused && !form.busy {
                if let Some(mfa) = mfa {
                    mfa.request_focus();
                } else if settings.username.is_empty()
                    && let Some(user) = user
                {
                    user.request_focus();
                } else if let Some(password) = password {
                    password.request_focus();
                }
                form.focused = true;
            }
            if let Some(terms) = &form.tos_message {
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    ui.label(terms);
                });
                if let Some(url) = super::terms_url(terms, &settings.login_uri()) {
                    super::terms_link(ui, p, &url);
                }
                widgets::checkbox(
                    ui,
                    p,
                    &mut form.agree_tos,
                    aurora_i18n::tr!("ui-login-i-accept-the-terms-and-want-to-log-in"),
                );
            }
            // its own step: reading it acknowledges this message only
            if let Some(message) = &form.critical_message {
                ui.label(RichText::new(aurora_i18n::tr!("ui-login-critical-message")).strong().color(p.amber));
                egui::ScrollArea::vertical()
                    .id_salt("critical_message")
                    .max_height(120.0)
                    .show(ui, |ui| {
                        ui.label(message);
                    });
                widgets::checkbox(ui, p, &mut form.read_critical, aurora_i18n::tr!("ui-login-critical-message-read"));
            }
            // the login would go on to another server: the user decides
            if let Some(server) = &form.next_server {
                ui.add_space(8.0);
                ui.label(RichText::new(aurora_i18n::tr!("ui-login-next-server")).strong());
                ui.label(aurora_i18n::tr!("ui-login-next-server-body", server = server.as_str()));
                widgets::checkbox(ui, p, &mut form.accept_next_server, aurora_i18n::tr!("ui-login-next-server-accept"));
            }
            if let Some(error) = &form.error {
                ui.label(RichText::new(error).color(p.danger));
            }
            if let Some(reason) = form.block_reason {
                ui.label(RichText::new(reason).color(p.warn));
            }
            form.block_reason.is_none()
                && !settings.username.trim().is_empty()
                && !settings.login_uri().is_empty()
                && (!form.password.is_empty() || form.stored.is_some())
                && (!form.show_mfa || !form.mfa_token.trim().is_empty())
                && (form.tos_message.is_none() || form.agree_tos)
                && (form.critical_message.is_none() || form.read_critical)
                && (form.next_server.is_none() || form.accept_next_server)
        })
        .inner
        && !grids_open;
    ui.add_space(10.0);
    if form.busy {
        connection_progress(ui, p, progress);
        return false;
    }
    // after a region restart cut the session: "Reconnect"
    let label = if form.reconnect && !new_account {
        aurora_i18n::tr!("ui-login-reconnect")
    } else {
        aurora_i18n::tr!("ui-login-log-in")
    };
    let pressed = widgets::action_button_if(
        ui,
        p,
        "login-2",
        label,
        ActionKind::Primary,
        Vec2::new(ui.available_width(), 38.0),
        can,
    )
    .clicked()
        || (can && enter && !egui::Popup::is_any_open(ui.ctx()));
    if pressed {
        settings.remember_username = true;
        form.show_password = false;
        form.reconnect = false;
        form.error = None;
        form.tos_message = None;
        form.critical_message = None;
        form.show_mfa = false;
        let next_server = form.next_server.take();
        if std::mem::take(&mut form.accept_next_server)
            && let Some(server) = next_server
        {
            form.accepted_next_servers.push(server);
        }
    }
    pressed
}

pub fn dialog(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    settings: &mut Settings,
    form: &mut LoginForm,
    progress: Option<(&str, f32)>,
) -> bool {
    let adding = form.adding;
    let mut submitted = false;
    let mut cancelled = false;
    let response = widgets::Modal::new(Id::unique("account_login_dialog"))
        .login_surface(p)
        .show(ctx, |ui| {
            ui.set_width(if adding { 340.0 } else { 320.0 });
            ui.set_max_height((ctx.content_rect().height() - 100.0).max(200.0));
            submitted = content(ui, p, icons, settings, form, adding, progress);
            cancelled = ui
                .add_enabled_ui(!form.busy, |ui| {
                    widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(ui.available_width(), 30.0))
                })
                .inner
                .clicked();
        });
    if !form.busy && (cancelled || response.should_close()) {
        form.editing = false;
        form.adding = false;
        form.error = None;
        form.reconnect = false;
        form.show_mfa = false;
        form.tos_message = None;
        form.critical_message = None;
        form.forget_next_server();
        form.password.clear();
        form.mfa_token.clear();
        if adding
            && let Some(key) = form.add_return.take()
            && let Some(index) = form.accounts.iter().position(|account| account.key() == key)
        {
            super::carousel::select(settings, form, index);
        }
    }
    if submitted {
        form.editing = false;
        if !adding {
            form.pending_connect = form
                .accounts
                .iter()
                .position(|account| account.key() == (settings.login_uri(), settings.username.trim().to_lowercase()));
            form.pending_since = Some(ctx.input(|i| i.time));
        }
    }
    submitted && adding
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_connection_keeps_fields_read_only_and_enter_does_not_resubmit() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut settings = Settings::default();
        let mut form = LoginForm {
            password: "synthetic-input".into(),
            ..Default::default()
        };
        let _ = super::super::test_output(ctx.run_ui(egui::RawInput::default(), |ui| {
            content(ui, &theme.palette(), &Icons::default(), &mut settings, &mut form, true, None);
        }));
        settings.username = "synthetic-alpha".into();
        form.busy = true;
        let mut submitted = false;
        let output = super::super::test_output(ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::Text("ignored-input".into()),
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                submitted = content(
                    ui,
                    &theme.palette(),
                    &Icons::default(),
                    &mut settings,
                    &mut form,
                    true,
                    Some((aurora_i18n::tr!("session-connecting-to-login-server"), 0.35)),
                );
            },
        ));
        assert!(!submitted);
        assert_eq!(settings.username, "synthetic-alpha");
        assert_eq!(form.password, "synthetic-input");
        assert!(output.shapes.iter().any(|source| matches!(&source.shape, egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("session-connecting-to-login-server"))));
    }

    #[test]
    fn first_account_and_existing_password_mfa_use_the_required_fields() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut settings = Settings {
            username: "synthetic-alpha".into(),
            ..Default::default()
        };
        let mut form = LoginForm {
            password: "synthetic-input".into(),
            ..Default::default()
        };
        let mut submitted = false;
        let mut draw = |new_account: bool, mfa: bool, enter: bool| {
            form.show_mfa = mfa;
            let output = super::super::test_output(
                ctx.run_ui(
                    egui::RawInput {
                        events: enter
                            .then_some(egui::Event::Key {
                                key: egui::Key::Enter,
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
                        submitted = content(ui, &theme.palette(), &Icons::default(), &mut settings, &mut form, new_account, None);
                    },
                ),
            );
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.text().to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let first = draw(true, false, false);
        assert!(first.iter().any(|text| text == aurora_i18n::tr!("common-username")));
        assert!(first.iter().any(|text| text == aurora_i18n::tr!("ui-login-password")));
        assert!(first.iter().any(|text| text == aurora_i18n::tr!("ui-login-grid")));
        let compact = draw(false, false, false);
        assert!(!compact.iter().any(|text| text == aurora_i18n::tr!("common-username")));
        assert!(!compact.iter().any(|text| text == aurora_i18n::tr!("ui-login-grid")));
        assert!(compact.iter().any(|text| text == aurora_i18n::tr!("ui-login-remember-password")));
        let mfa = draw(false, true, false);
        assert!(!mfa.iter().any(|text| text == aurora_i18n::tr!("ui-login-password")));
        assert!(mfa.iter().any(|text| text == aurora_i18n::tr!("login-accounts-remember-mfa")));
        draw(true, false, true);
        assert!(submitted);
        assert!(form.accounts.is_empty(), "profiles are registered only after a successful login");
    }
}
