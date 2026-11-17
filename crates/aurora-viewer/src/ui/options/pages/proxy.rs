//! Preferences › Network and cache › Proxy: port of LLFloaterPreferenceProxy
//! (indra/newview/llfloaterpreference.cpp, floater_preferences_proxy.xml,
//! originally LGPL 2.1).
//!
//! - The settings take effect at the next start, as in the Legacy Viewer
//!   ("Proxy settings take effect after you restart"): the rule of every
//!   connection is installed once, before the first one (`crate::proxy`).
//! - The SOCKS 5 user name and password go to the OS credential store
//!   (`crate::credentials`), never to the settings. The password is never
//!   read back into the window; saving it is an explicit button.
//! - Stricter than the Legacy Viewer: voice (WebRTC) cannot go through these
//!   proxies and shows the real address to the voice servers, so it is off
//!   while a proxy is on, unless the user turns direct voice back on after a
//!   warning and a confirmation (`direct_voice_modal`).

use super::super::cx::{Cx, Nothing, field};
use super::super::{OptionsUi, SLIDER_W};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind, action_button};
use aurora_proxy::{Endpoint, HttpProxyType, SocksAuth};
use egui::{RichText, Vec2};

/// The window's state for the credentials (the store is read once, for the
/// user name alone).
#[derive(Default)]
pub struct ProxyPage {
    loaded: bool,
    username: String,
    password: String,
    status: Status,
    confirm_direct_voice: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Status {
    #[default]
    Unknown,
    Saved,
    Missing,
    Failed,
}

/// The longest host the rule takes (`Endpoint::new`).
const HOST_CHARS: usize = 253;

pub fn group(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-proxy"), |cx| {
        cx.note(tr!("ui-options-proxy-restart-note"));
        if crate::proxy::restart_needed(&cx.s.proxy) {
            cx.note(tr!("ui-options-proxy-restart-needed"));
        }
        // the HTTP proxy of the web pages (BrowserProxyEnabled)
        let web_on = cx.s.proxy.web_enabled;
        cx.keywords("proxy http");
        cx.toggle(
            tr!("ui-options-proxy-web"),
            tr!("ui-options-proxy-web-help"),
            &field!(proxy.web_enabled),
        );
        cx.subs(!web_on, |cx| {
            cx.row(
                tr!("ui-options-proxy-address"),
                tr!("ui-options-proxy-web-address-help"),
                &field!(proxy.web_host),
                |ui, s| host_field(ui, &p, &mut s.proxy.web_host, s.proxy.web_port, s.proxy.web_enabled),
            );
            cx.row(
                tr!("ui-options-proxy-port"),
                tr!("ui-options-proxy-port-help"),
                &field!(proxy.web_port),
                |ui, s| port_field(ui, &mut s.proxy.web_port),
            );
        });
        // the SOCKS 5 proxy of the circuits (Socks5ProxyEnabled)
        let socks_on = cx.s.proxy.socks_enabled;
        cx.keywords("proxy socks socks5");
        cx.toggle(
            tr!("ui-options-proxy-socks"),
            tr!("ui-options-proxy-socks-help"),
            &field!(proxy.socks_enabled),
        );
        cx.subs(!socks_on, |cx| {
            cx.row(
                tr!("ui-options-proxy-address"),
                tr!("ui-options-proxy-socks-address-help"),
                &field!(proxy.socks_host),
                |ui, s| host_field(ui, &p, &mut s.proxy.socks_host, s.proxy.socks_port, s.proxy.socks_enabled),
            );
            cx.row(
                tr!("ui-options-proxy-port"),
                tr!("ui-options-proxy-port-help"),
                &field!(proxy.socks_port),
                |ui, s| port_field(ui, &mut s.proxy.socks_port),
            );
            cx.row(
                tr!("ui-options-proxy-auth"),
                tr!("ui-options-proxy-auth-help"),
                &field!(proxy.socks_auth),
                |ui, s| {
                    let mut changed = false;
                    ui.vertical(|ui| {
                        for (auth, label) in [
                            (SocksAuth::None, tr!("ui-options-proxy-auth-none")),
                            (SocksAuth::UserPass, tr!("ui-options-proxy-auth-userpass")),
                        ] {
                            if widgets::radio(ui, &p, s.proxy.socks_auth == auth, label).clicked() && s.proxy.socks_auth != auth {
                                s.proxy.socks_auth = auth;
                                changed = true;
                            }
                        }
                    });
                    if changed && s.proxy.socks_auth == SocksAuth::None {
                        // LLFloaterPreferenceProxy::saveSettings: credentials
                        // no longer used are removed from the store
                        crate::credentials::forget_proxy();
                    }
                    changed
                },
            );
            if cx.s.proxy.socks_auth == SocksAuth::UserPass {
                cx.below().row_with(
                    tr!("ui-options-proxy-credentials"),
                    tr!("ui-options-proxy-credentials-help"),
                    &Nothing,
                    |ui, _, st, _| {
                        credentials(ui, &p, st);
                        false
                    },
                );
            }
        });
        // the viewer's other HTTP traffic (HttpProxyType); a choice whose
        // proxy is off is greyed, as the Legacy Viewer's radio buttons
        cx.keywords("proxy http socks");
        cx.row(
            tr!("ui-options-proxy-http-type"),
            tr!("ui-options-proxy-http-type-help"),
            &field!(proxy.http_type),
            |ui, s| {
                let mut changed = false;
                let current = s.proxy.effective_http_type();
                ui.vertical(|ui| {
                    for (kind, label, allowed) in [
                        (HttpProxyType::None, tr!("ui-options-proxy-http-none"), true),
                        (HttpProxyType::Web, tr!("ui-options-proxy-http-web"), s.proxy.web_enabled),
                        (HttpProxyType::Socks, tr!("ui-options-proxy-http-socks"), s.proxy.socks_enabled),
                    ] {
                        let clicked = ui
                            .add_enabled_ui(allowed, |ui| widgets::radio(ui, &p, current == kind, label).clicked())
                            .inner;
                        if clicked && s.proxy.http_type != kind {
                            s.proxy.http_type = kind;
                            changed = true;
                        }
                    }
                });
                changed
            },
        );
        // Aurora's own: voice does not go through the proxy
        cx.greyed(!cx.s.proxy.any_proxy());
        cx.keywords("proxy voice webrtc");
        cx.row_with(
            tr!("ui-options-proxy-direct-voice"),
            tr!("ui-options-proxy-direct-voice-help"),
            &field!(proxy.direct_voice),
            |ui, s, st, _| {
                let mut on = s.proxy.direct_voice;
                let mut changed = false;
                ui.vertical(|ui| {
                    if widgets::switch(ui, &p, &mut on).changed() {
                        if on {
                            // turned on only once the warning is confirmed
                            st.proxy.confirm_direct_voice = true;
                        } else {
                            s.proxy.direct_voice = false;
                            changed = true;
                        }
                    }
                    ui.add(egui::Label::new(RichText::new(tr!("ui-options-proxy-direct-voice-warning")).size(11.5).color(p.warn)).wrap());
                });
                changed
            },
        );
    });
}

/// A proxy's host, and why the rule would refuse it while its proxy is on.
fn host_field(ui: &mut egui::Ui, p: &Palette, host: &mut String, port: u16, enabled: bool) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        changed = ui
            .add(
                egui::TextEdit::singleline(host)
                    .desired_width(SLIDER_W + 60.0)
                    .char_limit(HOST_CHARS),
            )
            .changed();
        if enabled && Endpoint::new(host, port).is_none() {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-options-proxy-invalid-host"))
                    .size(11.5)
                    .color(p.danger),
            );
        }
    });
    changed
}

fn port_field(ui: &mut egui::Ui, port: &mut u16) -> bool {
    ui.add(
        egui::DragValue::new(port)
            .custom_parser(aurora_i18n::parse_number)
            .range(1..=u16::MAX)
            .speed(1.0),
    )
    .changed()
}

/// The user name and password of the SOCKS 5 proxy, kept in the credential
/// store by the Save button.
fn credentials(ui: &mut egui::Ui, p: &Palette, st: &mut OptionsUi) {
    use aurora_i18n::tr;
    let page = &mut st.proxy;
    if !page.loaded {
        page.loaded = true;
        match crate::credentials::proxy_username() {
            Some(username) => {
                page.username = username;
                page.status = Status::Saved;
            }
            None => page.status = Status::Missing,
        }
    }
    ui.vertical(|ui| {
        egui::Grid::new("proxy_credentials")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(tr!("ui-options-proxy-username"));
                ui.add(
                    egui::TextEdit::singleline(&mut page.username)
                        .desired_width(SLIDER_W)
                        .char_limit(255),
                );
                ui.end_row();
                ui.label(tr!("ui-options-proxy-password"));
                ui.add(
                    egui::TextEdit::singleline(&mut page.password)
                        .password(true)
                        .desired_width(SLIDER_W)
                        .char_limit(255)
                        .hint_text(if page.status == Status::Saved {
                            tr!("ui-options-proxy-password-kept")
                        } else {
                            ""
                        }),
                );
                ui.end_row();
            });
        ui.horizontal(|ui| {
            let ready = !page.username.is_empty() && !page.password.is_empty();
            let save = ui
                .add_enabled_ui(ready, |ui| {
                    action_button(
                        ui,
                        p,
                        "device-floppy-filled",
                        tr!("ui-options-proxy-save"),
                        ActionKind::Flat,
                        Vec2::new(0.0, 26.0),
                    )
                    .clicked()
                })
                .inner;
            if save {
                page.status = if crate::credentials::store_proxy(&page.username, &page.password) {
                    Status::Saved
                } else {
                    Status::Failed
                };
                // not kept in the window once saved
                page.password.clear();
            }
            let (text, color) = match page.status {
                Status::Saved => (tr!("ui-options-proxy-credentials-saved"), p.muted),
                Status::Missing => (tr!("ui-options-proxy-credentials-missing"), p.warn),
                Status::Failed => (tr!("ui-options-proxy-credentials-failed"), p.danger),
                Status::Unknown => ("", p.muted),
            };
            ui.label(RichText::new(text).size(11.5).color(color));
        });
    });
}

/// Asked before voice may connect directly while a proxy is on.
pub fn direct_voice_modal(ctx: &egui::Context, p: &Palette, s: &mut Settings, st: &mut OptionsUi, open: bool) {
    use aurora_i18n::tr;
    if !(st.proxy.confirm_direct_voice && open) {
        st.proxy.confirm_direct_voice = false;
        return;
    }
    let modal = widgets::Modal::new(egui::Id::unique("options_proxy_direct_voice")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(
            RichText::new(tr!("ui-options-proxy-direct-voice-confirm-title"))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr!("ui-options-proxy-direct-voice-confirm-text"))
                .size(12.5)
                .color(p.muted),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if action_button(
                ui,
                p,
                "microphone-filled",
                tr!("ui-options-proxy-direct-voice-confirm"),
                ActionKind::Danger,
                size,
            )
            .clicked()
            {
                s.proxy.direct_voice = true;
                st.proxy.confirm_direct_voice = false;
            }
            if action_button(ui, p, "x", tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                st.proxy.confirm_direct_voice = false;
            }
        });
    });
    if modal.should_close() {
        st.proxy.confirm_direct_voice = false;
    }
}
