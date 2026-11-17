//! Preferences › Network › Proxy at startup: the settings and the credential
//! store give the rule every connection of this process follows
//! (`aurora_proxy`), installed before the first one. Changes take effect at
//! the next start, as in the Legacy Viewer ("Proxy settings take effect after
//! you restart", LLFloaterPreferenceProxy::onClose); the window says so.

use std::sync::OnceLock;

use aurora_proxy::{ProxySettings, Rule, SocksAuth};

static INSTALLED: OnceLock<ProxySettings> = OnceLock::new();

/// Installs the rule of `settings`, once per process (the viewer, and the
/// crash helper before it sends a report).
pub fn install(settings: &ProxySettings) {
    // the credential store is read only when the settings ask for it
    let credentials = (settings.socks_enabled && settings.socks_auth == SocksAuth::UserPass)
        .then(crate::credentials::load_proxy)
        .flatten();
    if aurora_proxy::install(Rule::from_settings(settings, credentials)) {
        let _ = INSTALLED.set(settings.clone());
    }
}

/// Whether the settings differ from the ones this process runs with.
pub fn restart_needed(current: &ProxySettings) -> bool {
    INSTALLED.get().is_some_and(|installed| installed != current)
}
