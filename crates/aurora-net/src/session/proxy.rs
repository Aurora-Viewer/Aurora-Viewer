//! The proxy at the start of a session, before the login request: port of
//! LLStartUp::startLLProxy (indra/newview/llstartup.cpp, originally LGPL
//! 2.1). Settings the rule cannot use, or a SOCKS 5 proxy that refuses the
//! UDP association, stop the login with the Legacy Viewer's message
//! (notifications.xml, `SOCKS_*` and `PROXY_INVALID_*`), and nothing is sent
//! around the proxy. The messages leave the proxy's address out: the login
//! error is also written to the log.

use aurora_proxy::udp::UdpAssociation;
use aurora_proxy::{Problem, SocksError};

use super::Shared;
use crate::NetEvent;

/// The UDP association of the session when the SOCKS 5 proxy is on, or why
/// the login cannot go on.
pub(super) async fn start(_sh: &Shared) -> Result<Option<UdpAssociation>, String> {
    let rule = aurora_proxy::rule();
    if let Some(problem) = rule.problem {
        log::warn!("login refused: {}", rule.summary());
        return Err(problem_message(problem));
    }
    let Some(server) = rule.udp.as_ref() else {
        return Ok(None);
    };
    // the proxy's own name is the one resolved on this computer
    let proxy = tokio::net::lookup_host((server.endpoint.host(), server.endpoint.port()))
        .await
        .ok()
        .and_then(|mut addrs| addrs.next())
        .ok_or_else(|| socks_message(SocksError::InvalidHost))?;
    match UdpAssociation::open(proxy, server.credentials.clone()).await {
        Ok(association) => {
            log::info!("SOCKS 5 UDP relay ready");
            Ok(Some(association))
        }
        Err(error) => {
            log::warn!("SOCKS 5 proxy: {error}");
            Err(socks_message(error))
        }
    }
}

/// Completes when the proxy ends the session's association; never without
/// one.
pub(super) async fn closed(association: &mut Option<UdpAssociation>) {
    match association {
        Some(association) => association.closed().await,
        None => std::future::pending().await,
    }
}

pub(super) fn login_failed(error: String) -> NetEvent {
    NetEvent::LoginFailed {
        error,
        mfa_required: false,
        tos_required: false,
        critical_required: false,
        bad_credentials: false,
        previous_session: false,
        next_server: None,
    }
}

/// The Legacy Viewer's message for a SOCKS 5 failure.
pub fn socks_message(error: SocksError) -> String {
    match error {
        SocksError::InvalidHost => aurora_i18n::tr!("proxy-socks-invalid-host"),
        SocksError::HostConnectFailed => aurora_i18n::tr!("proxy-socks-host-connect-failed"),
        SocksError::ConnectError | SocksError::Protocol => aurora_i18n::tr!("proxy-socks-connect-error"),
        SocksError::Timeout => aurora_i18n::tr!("proxy-socks-timeout"),
        SocksError::NotAcceptable => aurora_i18n::tr!("proxy-socks-not-acceptable"),
        SocksError::AuthFailed => aurora_i18n::tr!("proxy-socks-auth-fail"),
        SocksError::NotPermitted => aurora_i18n::tr!("proxy-socks-not-permitted"),
        SocksError::Refused(_) => aurora_i18n::tr!("proxy-socks-udp-not-granted"),
        SocksError::BadCredentials => aurora_i18n::tr!("proxy-socks-bad-creds"),
    }
    .to_string()
}

/// The Legacy Viewer's message for settings the rule cannot use.
pub fn problem_message(problem: Problem) -> String {
    match problem {
        Problem::InvalidSocksHost => aurora_i18n::tr!("proxy-socks-invalid-host"),
        Problem::InvalidWebHost => aurora_i18n::tr!("proxy-http-invalid-host"),
        Problem::BadCredentials => aurora_i18n::tr!("proxy-socks-bad-creds"),
    }
    .to_string()
}
