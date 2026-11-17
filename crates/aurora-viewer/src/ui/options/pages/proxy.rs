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
//! - Stricter than the Legacy Viewer: voice (WebRTC) and the media plugins
//!   (the web browser without the HTTP proxy, the video plugin always) cannot
//!   go through these proxies and show the real address, so they are off while
//!   a proxy is on, unless the user turns them back on after a warning and a
//!   confirmation (`Direct`, `direct_modal`).

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
    /// The direct connection waiting for its confirmation.
    confirm: Option<Direct>,
    /// The authentication went back to None: the saved credentials are
    /// removed when the window closes (the Legacy Viewer removes them on OK).
    forget_on_close: bool,
    /// New credentials were saved: they apply at the next start.
    credentials_saved: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Status {
    #[default]
    Unknown,
    Saved,
    Missing,
    Failed,
}

/// What may connect around the proxy once the user confirms it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direct {
    Voice,
    Media,
}

impl Direct {
    fn get(self, s: &mut Settings) -> &mut bool {
        match self {
            Direct::Voice => &mut s.proxy.direct_voice,
            Direct::Media => &mut s.proxy.direct_media,
        }
    }

    /// The confirmation's title, text, button and icon.
    fn confirm(self) -> [&'static str; 4] {
        use aurora_i18n::tr;
        match self {
            Direct::Voice => [
                tr!("ui-options-proxy-direct-voice-confirm-title"),
                tr!("ui-options-proxy-direct-voice-confirm-text"),
                tr!("ui-options-proxy-direct-voice-confirm"),
                "microphone-filled",
            ],
            Direct::Media => [
                tr!("ui-options-proxy-direct-media-confirm-title"),
                tr!("ui-options-proxy-direct-media-confirm-text"),
                tr!("ui-options-proxy-direct-media-confirm"),
                "player-play-filled",
            ],
        }
    }
}

/// The longest host the rule takes (`Endpoint::new`).
const HOST_CHARS: usize = 253;

pub fn group(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-proxy"), |cx| {
        cx.note(tr!("ui-options-proxy-restart-note"));
        if crate::proxy::restart_needed(&cx.s.proxy) || cx.st.proxy.credentials_saved {
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
            cx.row_with(
                tr!("ui-options-proxy-auth"),
                tr!("ui-options-proxy-auth-help"),
                &field!(proxy.socks_auth),
                |ui, s, st, _| {
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
                    if changed {
                        // LLFloaterPreferenceProxy::saveSettings: credentials
                        // no longer used are removed from the store, once
                        // the window closes (a misclick loses nothing)
                        st.proxy.forget_on_close = s.proxy.socks_auth == SocksAuth::None;
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
        // Aurora's own: voice and media do not go through the proxy
        cx.greyed(!cx.s.proxy.any_proxy());
        cx.keywords("proxy voice webrtc");
        direct_row(
            cx,
            Direct::Voice,
            [
                tr!("ui-options-proxy-direct-voice"),
                tr!("ui-options-proxy-direct-voice-help"),
                tr!("ui-options-proxy-direct-voice-warning"),
            ],
            &field!(proxy.direct_voice),
        );
        cx.keywords("proxy media video web browser");
        direct_row(
            cx,
            Direct::Media,
            [
                tr!("ui-options-proxy-direct-media"),
                tr!("ui-options-proxy-direct-media-help"),
                tr!("ui-options-proxy-direct-media-warning"),
            ],
            &field!(proxy.direct_media),
        );
    });
}

/// A switch that lets `direct` connect around the proxy: turned off at once,
/// turned on only once its warning is confirmed (`direct_modal`); the warning
/// stays written beside it.
fn direct_row(cx: &mut Cx, direct: Direct, [label, hint, warning]: [&str; 3], f: &super::super::cx::F<bool>) {
    let p = cx.p;
    cx.wide().row_with(label, hint, f, |ui, s, st, _| {
        let mut on = *direct.get(s);
        let mut changed = false;
        ui.vertical(|ui| {
            if widgets::switch(ui, &p, &mut on).changed() {
                if on {
                    st.proxy.confirm = Some(direct);
                } else {
                    *direct.get(s) = false;
                    changed = true;
                }
            }
            ui.add(egui::Label::new(RichText::new(warning).size(11.5).color(p.warn)).wrap());
        });
        changed
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
                    page.credentials_saved = true;
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

/// Each frame of the window: the direct connection's confirmation, and the
/// credentials removed once the window closes with no authentication.
pub fn frame(ctx: &egui::Context, p: &Palette, s: &mut Settings, st: &mut OptionsUi, open: bool) {
    if !open && std::mem::take(&mut st.proxy.forget_on_close) && s.proxy.socks_auth == SocksAuth::None {
        crate::credentials::forget_proxy();
        st.proxy.loaded = false;
    }
    direct_modal(ctx, p, s, st, open);
}

/// Asked before voice or media may connect directly while a proxy is on.
fn direct_modal(ctx: &egui::Context, p: &Palette, s: &mut Settings, st: &mut OptionsUi, open: bool) {
    let Some(direct) = st.proxy.confirm.filter(|_| open) else {
        st.proxy.confirm = None;
        return;
    };
    let [title, text, button, icon] = direct.confirm();
    let modal = widgets::Modal::new(egui::Id::unique("options_proxy_direct")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(12.5).color(p.muted));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if action_button(ui, p, icon, button, ActionKind::Danger, size).clicked() {
                *direct.get(s) = true;
                st.proxy.confirm = None;
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                st.proxy.confirm = None;
            }
        });
    });
    if modal.should_close() {
        st.proxy.confirm = None;
    }
}
