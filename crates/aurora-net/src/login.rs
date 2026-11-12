//! XML-RPC login (`login_to_simulator`), modelled on
//! `newview/lllogininstance.cpp`.

use crate::xmlrpc;
use aurora_llsd::{Llsd, Map};
use glam::Vec3;
use std::net::Ipv4Addr;
use uuid::Uuid;

pub const SL_MAIN_GRID: &str = "https://login.agni.lindenlab.com/cgi-bin/login.cgi";
pub const SL_BETA_GRID: &str = "https://login.aditi.lindenlab.com/cgi-bin/login.cgi";

pub const CHANNEL: &str = "Aurora Viewer";

#[derive(Debug, Clone)]
pub enum StartLocation {
    Last,
    Home,
    Region { name: String, x: u32, y: u32, z: u32 },
}

impl StartLocation {
    pub fn to_login_string(&self) -> String {
        match self {
            StartLocation::Last => "last".into(),
            StartLocation::Home => "home".into(),
            StartLocation::Region { name, x, y, z } => format!("uri:{name}&{x}&{y}&{z}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoginRequest {
    pub login_uri: String,
    /// "First Last" or a single username (last name "Resident").
    pub username: String,
    pub password: String,
    pub start: StartLocation,
    pub agree_to_tos: bool,
    pub read_critical: bool,
    pub mfa_token: String,
    pub mfa_hash: String,
    /// The device ids of LLLoginInstance::constructAuthParams
    /// (lllogininstance.cpp:232, 239), hashed by the viewer.
    pub mac: String,
    pub id0: String,
    /// How the previous session of this account ended.
    pub last_exec: LastExec,
    /// The system the viewer runs on, read by the viewer.
    pub platform: Platform,
    /// The other login servers (`login_authority`) the user agreed to
    /// continue this login on (`LoginError::NextServer`).
    pub accepted_next_servers: Vec<String>,
}

/// The system a login names (LLAppViewer::init's setPlatformInfo with
/// LLOSInfo, indra/llcommon/llsys.cpp): `platform_version`, the dotted
/// "major.minor.build" of getOSVersionString, and `platform_string`, the
/// short name of getOSStringSimple. Read by the viewer, which has the
/// system probe; an unknown version is left out of the request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Platform {
    pub version: String,
    pub name: String,
}

impl Platform {
    /// Windows from its build number (the registry's CurrentBuildNumber):
    /// every build from 10240 (Windows 10) on is NT 10.0, and LLOSInfo
    /// names builds from 22000 on Windows 11; "64-bit" on x86-64, as it
    /// adds for PROCESSOR_ARCHITECTURE_AMD64. Rust programs need Windows 10.
    pub fn windows(build: &str, x86_64: bool) -> Platform {
        let bits = if x86_64 { " 64-bit" } else { "" };
        match build.trim().parse::<u32>() {
            Ok(build) if build >= 10240 => Platform {
                version: format!("10.0.{build}"),
                name: format!("Microsoft Windows {}{bits}", if build >= 22000 { 11 } else { 10 }),
            },
            _ => Platform {
                version: String::new(),
                name: format!("Microsoft Windows{bits}"),
            },
        }
    }

    /// macOS from its product version ("14.5"): "14.5.0" and "macOS
    /// 14.5.0", as LLGetDarwinOSInfo's major, minor and bugfix numbers.
    pub fn macos(product: &str) -> Platform {
        match dotted(product) {
            Some(version) => Platform {
                name: format!("macOS {version}"),
                version,
            },
            None => Platform {
                version: String::new(),
                name: "macOS".into(),
            },
        }
    }

    /// Linux from its kernel release (uname's, "6.8.0-45-generic"): the
    /// version "6.8.0", and "Linux 6.8", LLOSInfo's name cut at the second
    /// dot. Deliberate difference: the Legacy Viewer's version on Linux is
    /// glibc's; the kernel's is the system's own.
    pub fn linux(release: &str) -> Platform {
        match dotted(release) {
            Some(version) => Platform {
                name: format!("Linux {}", version.rsplit_once('.').map_or(version.as_str(), |(head, _)| head)),
                version,
            },
            None => Platform {
                version: String::new(),
                name: "Linux".into(),
            },
        }
    }
}

/// The first one to three numbers of `s` ("14.5", "6.8.0-45-generic"),
/// as "major.minor.build" with zeros for the missing ones.
fn dotted(s: &str) -> Option<String> {
    let numbers: Vec<u32> = s
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .take_while(|part| !part.is_empty())
        .take(3)
        .map_while(|part| part.parse().ok())
        .collect();
    let major = *numbers.first()?;
    Some(format!(
        "{major}.{}.{}",
        numbers.get(1).copied().unwrap_or(0),
        numbers.get(2).copied().unwrap_or(0)
    ))
}

/// What the login server is told of the previous run (lllogininstance.cpp
/// `last_exec_event` / `last_exec_duration` / `last_exec_session_id`,
/// eLastExecEvent in llappviewer.h): the session id only when that run did
/// not end cleanly, as the Legacy Viewer reads it from its exec marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastExec {
    pub event: i32,
    /// Seconds, or -1 when unknown.
    pub duration: i32,
    pub session_id: Uuid,
}

impl LastExec {
    /// LAST_EXEC_NORMAL
    pub const NORMAL: i32 = 0;
    /// LAST_EXEC_UNKNOWN: the run ended without a clean exit, cause unknown.
    pub const UNKNOWN: i32 = 9;
}

impl Default for LastExec {
    fn default() -> Self {
        Self {
            event: Self::NORMAL,
            duration: -1,
            session_id: Uuid::nil(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoginResponse {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub secure_session_id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub sim_ip: Ipv4Addr,
    pub sim_port: u16,
    pub circuit_code: u32,
    pub region_x: u32,
    pub region_y: u32,
    pub region_size_x: u32,
    pub region_size_y: u32,
    pub seed_capability: String,
    pub look_at: Vec3,
    pub message: String,
    pub agent_appearance_service: String,
    pub inventory_root: Uuid,
    pub mfa_hash: Option<String>,
    pub raw: Llsd,
}

impl LoginResponse {
    pub fn region_handle(&self) -> u64 {
        ((self.region_x as u64) << 32) | self.region_y as u64
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("network error: {0}")]
    Http(String),
    #[error("{0}")]
    XmlRpc(#[from] xmlrpc::XmlRpcError),
    #[error("login refused ({reason}): {message}")]
    Refused {
        reason: String,
        message: String,
        /// `message_id` of the extended errors (`LoginFailedPendingLogout`…).
        message_id: String,
    },
    #[error("two-factor authentication required: {0}")]
    MfaRequired(String),
    #[error("terms of service must be accepted: {0}")]
    TosRequired(String),
    #[error("critical message must be read: {0}")]
    CriticalMessage(String),
    #[error("malformed login response: {0}")]
    Malformed(&'static str),
    /// The login server sends the login on to another server (its
    /// `login_authority`): the user decides, and the next attempt names it
    /// in `LoginRequest::accepted_next_servers`.
    #[error("the login continues on {0}")]
    NextServer(String),
    /// The login was not sent on (`NextServer::Refuse`).
    #[error("{0}")]
    Stopped(&'static str),
}

impl LoginError {
    /// The grid still holds a session of this account: the login server
    /// asks to wait while it logs it out (reason "presence", and the
    /// LoginFailedPendingLogout / LoginFailedLoggingOutSession /
    /// LogoutFailed* message ids of the Legacy Viewer's strings.xml, which it
    /// shows as they come, LLLoginInstance::handleLoginFailure).
    pub fn previous_session(&self) -> bool {
        match self {
            LoginError::Refused { reason, message_id, .. } => {
                reason == "presence"
                    || matches!(
                        message_id.as_str(),
                        "LoginFailedPendingLogout"
                            | "LoginFailedLoggingOutSession"
                            | "LoginFailedAlreadyLoggedIn"
                            | "LogoutFailedLoggingOut"
                            | "LogoutFailedStillLoggingOut"
                            | "LogoutFailedLogoutBegun"
                            | "LogoutFailedPendingLogin"
                            | "LogoutSucceeded"
                    )
            }
            _ => false,
        }
    }
}

/// The error of a `login` answer that is neither true nor indeterminate.
fn refusal(v: &Llsd) -> LoginError {
    let reason = v["reason"].as_str().to_owned();
    let message = v["message"].as_str().to_owned();
    let message_id = v["message_id"].as_str().to_owned();
    match reason.as_str() {
        "mfa_challenge" => LoginError::MfaRequired(message),
        "tos" => LoginError::TosRequired(message),
        "critical" => LoginError::CriticalMessage(message),
        _ => LoginError::Refused {
            reason,
            message,
            message_id,
        },
    }
}

fn split_username(username: &str) -> (String, String) {
    let t = username.trim();
    let mut parts = t.split([' ', '.']).filter(|s| !s.is_empty());
    let first = parts.next().unwrap_or("").to_owned();
    let last = parts.next().unwrap_or("Resident").to_owned();
    (first, last)
}

/// `$1$` + md5 hex of the password (SL truncates passwords to 16 chars).
pub fn hash_password(password: &str, truncate16: bool) -> String {
    if password.len() == 35 && password.starts_with("$1$") {
        return password.to_owned();
    }
    let p: String = if truncate16 {
        password.chars().take(16).collect()
    } else {
        password.to_owned()
    };
    format!("$1${:x}", md5::compute(p.as_bytes()))
}

/// The password form the login sends to this grid (what a remembered
/// password keeps).
pub fn login_hash(login_uri: &str, password: &str) -> String {
    hash_password(password, is_linden_grid(login_uri))
}

fn is_linden_grid(uri: &str) -> bool {
    uri.contains("lindenlab.com")
}

/// The login server's host, lower case, with its port when one is given.
fn login_authority(uri: &str) -> Option<String> {
    let url = reqwest::Url::parse(uri.trim()).ok()?;
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// A Linden Lab login server: its host is lindenlab.com or one of its
/// subdomains (the host itself, not a mere mention of the name anywhere in
/// the address).
pub fn is_linden_login(uri: &str) -> bool {
    login_authority(uri).is_some_and(|authority| {
        let host = authority.split(':').next().unwrap_or("");
        host == "lindenlab.com" || host.ends_with(".lindenlab.com")
    })
}

/// A device identifier (`mac`, `id0`, the viewer statistics' serial) as the
/// grid of `login_uri` gets it. Linden Lab's grids get the real hash, as
/// with their own viewer (the Third Party Viewer Policy: machine bans must
/// hold). Any other grid gets the md5 hex of that hash and of the grid's
/// login host: the same form and length, steady for this machine on this
/// grid, but unlike the real one and unlike any other grid's, so that grids
/// cannot match a machine between them nor learn the Second Life one. An
/// empty identifier (none could be read) stays empty. Deliberate difference:
/// the Legacy Viewer sends the same LLMachineID / llHashedUniqueID values to
/// every grid (LLLoginInstance::constructAuthParams).
pub fn device_id_for(login_uri: &str, id: &str) -> String {
    if id.is_empty() || is_linden_login(login_uri) {
        return id.to_owned();
    }
    let authority = login_authority(login_uri).unwrap_or_default();
    format!("{:x}", md5::compute(format!("{id}\n{authority}")))
}

/// The network adapter address of the viewer statistics as the grid of
/// `login_uri` gets it: the real one on Linden Lab's grids, else six bytes
/// derived the same way as `device_id_for` (locally administered and
/// unicast, so that it reads as an address and never as a vendor's).
pub fn adapter_for(login_uri: &str, node: Option<[u8; 6]>) -> Option<[u8; 6]> {
    let node = node?;
    if is_linden_login(login_uri) {
        return Some(node);
    }
    let authority = login_authority(login_uri).unwrap_or_default();
    let mut text = node.iter().map(|b| format!("{b:02x}")).collect::<String>();
    text.push('\n');
    text.push_str(&authority);
    let digest = md5::compute(text);
    let mut out = [0u8; 6];
    out.copy_from_slice(&digest[..6]);
    out[0] = (out[0] | 0x02) & !0x01;
    Some(out)
}

/// Whether a login may go on from the server at `from` to `to` (an
/// "indeterminate" answer's `next_url`, or the `Location` of an HTTP
/// redirect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextServer {
    /// The same server (host and port; `http` to `https` included), or one
    /// Linden Lab login server to another.
    Follow,
    /// Another server, by its `login_authority`: the user is asked first.
    Ask(String),
    /// Not sent: an encrypted login would go on unencrypted, or the
    /// address is not an `http(s)` one.
    Refuse,
}

/// Where a login may be sent on to. `accepted` are the servers the user
/// already agreed to (`LoginRequest::accepted_next_servers`). Deliberate
/// difference: the Legacy Viewer sends the login on to any `next_url`
/// (LLLogin::Impl::loginCoro, viewer_components/login/lllogin.cpp) and
/// through any HTTP redirect (llcorehttp's CURLOPT_FOLLOWLOCATION), with
/// the password, the second factor and the device ids of the first grid.
pub fn next_server(from: &str, to: &str, accepted: &[String]) -> NextServer {
    let (Ok(from_url), Ok(to_url)) = (reqwest::Url::parse(from.trim()), reqwest::Url::parse(to.trim())) else {
        return NextServer::Refuse;
    };
    let encrypted = to_url.scheme() == "https";
    if !matches!(to_url.scheme(), "https" | "http") || (from_url.scheme() == "https" && !encrypted) {
        return NextServer::Refuse;
    }
    let (Some(here), Some(there)) = (login_authority(from), login_authority(to)) else {
        return NextServer::Refuse;
    };
    if here == there || (is_linden_login(from) && is_linden_login(to)) || accepted.contains(&there) {
        NextServer::Follow
    } else {
        NextServer::Ask(there)
    }
}

/// The HTTP redirects of the login request: followed by the client only
/// when `next_server` follows them on its own; the others come back to
/// `login`, which asks or refuses (a 307 / 308 resends the request).
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error(aurora_i18n::tr!("login-too-many-redirects"));
        }
        let follow = attempt
            .previous()
            .last()
            .is_some_and(|from| next_server(from.as_str(), attempt.url().as_str(), &[]) == NextServer::Follow);
        if follow { attempt.follow() } else { attempt.stop() }
    })
}

/// `Ok` when the login may go on from `from` to `to`.
fn go_on(from: &str, to: &str, accepted: &[String]) -> Result<(), LoginError> {
    match next_server(from, to, accepted) {
        NextServer::Follow => Ok(()),
        NextServer::Ask(server) => Err(LoginError::NextServer(server)),
        NextServer::Refuse => Err(LoginError::Stopped(aurora_i18n::tr!("login-next-server-refused"))),
    }
}

/// The login request's parameters for the server at `uri`: the device ids
/// are those this server gets (`device_id_for`), the password's form that
/// of the account's grid.
fn build_params(req: &LoginRequest, uri: &str) -> Llsd {
    let (first, last) = split_username(&req.username);
    let mut p = Map::new();
    p.insert("first".into(), first.into());
    p.insert("last".into(), last.into());
    p.insert("passwd".into(), hash_password(&req.password, is_linden_grid(&req.login_uri)).into());
    p.insert("start".into(), req.start.to_login_string().into());
    p.insert("channel".into(), CHANNEL.into());
    p.insert("version".into(), aurora_version::login_version().into());
    p.insert(
        "platform".into(),
        if cfg!(windows) {
            "win"
        } else if cfg!(target_os = "macos") {
            "mac"
        } else {
            "lnx"
        }
        .into(),
    );
    // the real system (LLOSInfo), or no version at all when it is unknown
    if !req.platform.version.is_empty() {
        p.insert("platform_version".into(), req.platform.version.clone().into());
    }
    let name = if req.platform.name.is_empty() {
        std::env::consts::OS
    } else {
        req.platform.name.as_str()
    };
    p.insert("platform_string".into(), name.into());
    p.insert("address_size".into(), 64.into());
    // the real hashes on Linden Lab's grids only (device_id_for)
    p.insert("mac".into(), device_id_for(uri, &req.mac).into());
    p.insert("id0".into(), device_id_for(uri, &req.id0).into());
    // the HostID setting, empty outside hosted instances (lllogininstance.cpp:240)
    p.insert("host_id".into(), "".into());
    p.insert("agree_to_tos".into(), req.agree_to_tos.into());
    p.insert("read_critical".into(), req.read_critical.into());
    p.insert("last_exec_event".into(), req.last_exec.event.into());
    p.insert("last_exec_duration".into(), req.last_exec.duration.into());
    p.insert("last_exec_session_id".into(), req.last_exec.session_id.to_string().into());
    p.insert("extended_errors".into(), true.into());
    p.insert("host_id".into(), "".into());
    p.insert("token".into(), req.mfa_token.clone().into());
    p.insert("mfa_hash".into(), req.mfa_hash.clone().into());
    let options: Vec<Llsd> = [
        "inventory-root",
        "inventory-skeleton",
        "inventory-lib-root",
        "inventory-lib-owner",
        "inventory-skel-lib",
        "initial-outfit",
        "gestures",
        "display_names",
        "event_categories",
        "event_notifications",
        "classified_categories",
        "adult_compliant",
        "buddy-list",
        "newuser-config",
        "ui-config",
        "advanced-mode",
        "max-agent-groups",
        "map-server-url",
        "voice-config",
        "tutorial_setting",
        "login-flags",
        "global-textures",
    ]
    .iter()
    .map(|s| Llsd::from(*s))
    .collect();
    p.insert("options".into(), Llsd::Array(options));
    Llsd::Map(p)
}

/// Parse "[r0.1, r0.9, r0]" style vectors used by the login response.
pub fn parse_ll_vector(s: &str) -> Option<Vec3> {
    let cleaned: String = s.chars().filter(|c| !matches!(c, '[' | ']' | 'r' | ' ')).collect();
    let mut it = cleaned.split(',').map(|p| p.parse::<f32>().ok());
    Some(Vec3::new(it.next()??, it.next()??, it.next()??))
}

fn parse_success(v: Llsd) -> Result<LoginResponse, LoginError> {
    let uuid = |k: &str| v[k].as_uuid();
    let agent_id = uuid("agent_id");
    if agent_id.is_nil() {
        return Err(LoginError::Malformed("agent_id"));
    }
    let sim_ip: Ipv4Addr = v["sim_ip"].as_str().parse().map_err(|_| LoginError::Malformed("sim_ip"))?;
    let sim_port = u16::try_from(v["sim_port"].as_i32()).map_err(|_| LoginError::Malformed("sim_port"))?;
    let inventory_root = v["inventory-root"][0]["folder_id"].as_uuid();
    let size = |k: &str| crate::session::region_side(u32::try_from(v[k].as_i32()).unwrap_or(256));
    Ok(LoginResponse {
        agent_id,
        session_id: uuid("session_id"),
        secure_session_id: uuid("secure_session_id"),
        first_name: v["first_name"].as_str().trim_matches('"').to_owned(),
        last_name: v["last_name"].as_str().trim_matches('"').to_owned(),
        sim_ip,
        sim_port,
        circuit_code: v["circuit_code"].as_u32(),
        region_x: v["region_x"].as_u32(),
        region_y: v["region_y"].as_u32(),
        region_size_x: size("region_size_x"),
        region_size_y: size("region_size_y"),
        // The login seed is handed to `CURLOPT_URL` as received
        // (`LLViewerRegion::setSeedCapability`). A missing scheme is guessed.
        seed_capability: crate::curl_url::ensure_scheme(v["seed_capability"].as_str()),
        look_at: parse_ll_vector(v["look_at"].as_str()).unwrap_or(Vec3::X),
        message: v["message"].as_str().to_owned(),
        agent_appearance_service: v["agent_appearance_service"].as_str().to_owned(),
        inventory_root,
        mfa_hash: match v["mfa_hash"].as_str() {
            "" => None,
            s => Some(s.to_owned()),
        },
        raw: v,
    })
}

/// Perform the login exchange, following "indeterminate" redirects where
/// `next_server` allows. `http` follows its HTTP redirects with
/// `redirect_policy`.
pub async fn login(http: &reqwest::Client, req: &LoginRequest) -> Result<LoginResponse, LoginError> {
    let mut uri = req.login_uri.clone();
    let mut method = "login_to_simulator".to_owned();
    let accepted = &req.accepted_next_servers;
    for _attempt in 0..5 {
        let body = xmlrpc::build_call(&method, &build_params(req, &uri));
        let resp = http
            .post(&uri)
            .header("Content-Type", "text/xml")
            .body(body)
            .send()
            .await
            .map_err(|e| LoginError::Http(e.without_url().to_string()))?;
        let status = resp.status();
        // where the client's own redirects (same server) ended
        let at = resp.url().clone();
        // a redirect the client stopped at: another server, asked or refused
        if matches!(status.as_u16(), 307 | 308)
            && let Some(next) = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|location| location.to_str().ok())
                .and_then(|location| at.join(location).ok())
        {
            go_on(at.as_str(), next.as_str(), accepted)?;
            uri = next.into();
            continue;
        }
        let bytes = resp.bytes().await.map_err(|e| LoginError::Http(e.without_url().to_string()))?;
        if !status.is_success() {
            return Err(LoginError::Http(format!("HTTP {status}")));
        }
        let v = xmlrpc::parse_response(&bytes)?;
        match v["login"].to_string_value().as_str() {
            "true" => return parse_success(v),
            "indeterminate" => {
                let next = v["next_url"].as_str();
                if next.is_empty() {
                    return Err(LoginError::Malformed("indeterminate without next_url"));
                }
                // a relative address stays on this server
                let next = at
                    .join(next)
                    .map_err(|_| LoginError::Stopped(aurora_i18n::tr!("login-next-server-refused")))?;
                go_on(at.as_str(), next.as_str(), accepted)?;
                uri = next.into();
                let m = v["next_method"].as_str();
                if !m.is_empty() {
                    method = m.to_owned();
                }
                continue;
            }
            _ => return Err(refusal(&v)),
        }
    }
    Err(LoginError::Malformed(aurora_i18n::tr!("login-too-many-redirects")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_seed_without_a_scheme_is_fetched_as_http() {
        let mut m = Map::new();
        m.insert("agent_id".into(), Llsd::Uuid(Uuid::from_u128(1)));
        m.insert("sim_ip".into(), "127.0.0.1".into());
        m.insert("sim_port".into(), 9000.into());
        m.insert("seed_capability".into(), "grid.example:8002/cap/seed".into());
        let login = parse_success(Llsd::Map(m)).expect("login");
        assert_eq!(login.seed_capability, "http://grid.example:8002/cap/seed");
    }

    #[test]
    fn username_split() {
        assert_eq!(split_username("jane"), ("jane".into(), "Resident".into()));
        assert_eq!(split_username("Jane Doe"), ("Jane".into(), "Doe".into()));
        assert_eq!(split_username("jane.doe"), ("jane".into(), "doe".into()));
    }

    #[test]
    fn password_hash() {
        // md5("test") = 098f6bcd4621d373cade4e832627b4f6
        assert_eq!(hash_password("test", true), "$1$098f6bcd4621d373cade4e832627b4f6");
        // truncated to 16 chars on SL
        assert_eq!(hash_password("0123456789abcdefXYZ", true), hash_password("0123456789abcdef", true));
    }

    #[test]
    fn a_session_still_on_the_grid_is_told_apart_from_other_refusals() {
        let answer = |reason: &str, id: &str| {
            let mut m = Map::new();
            m.insert("login".into(), "false".into());
            m.insert("reason".into(), reason.into());
            m.insert("message".into(), "The system is logging you out right now.".into());
            m.insert("message_id".into(), id.into());
            refusal(&Llsd::Map(m))
        };
        assert!(answer("presence", "").previous_session());
        assert!(answer("", "LoginFailedLoggingOutSession").previous_session());
        assert!(answer("other", "LoginFailedPendingLogout").previous_session());
        assert!(!answer("key", "LoginFailedAuthenticationFailed").previous_session());
        assert!(!answer("tos", "").previous_session());
        let LoginError::Refused { message, .. } = answer("presence", "LoginFailedPendingLogout") else {
            panic!("a presence answer is a refusal");
        };
        assert_eq!(message, "The system is logging you out right now.");
    }

    #[test]
    fn the_previous_run_goes_to_the_login_server() {
        let session = Uuid::from_u128(7);
        let req = LoginRequest {
            login_uri: SL_MAIN_GRID.into(),
            username: "synthetic".into(),
            password: "x".into(),
            start: StartLocation::Last,
            agree_to_tos: false,
            read_critical: false,
            mfa_token: String::new(),
            mfa_hash: String::new(),
            mac: String::new(),
            id0: String::new(),
            last_exec: LastExec {
                event: LastExec::UNKNOWN,
                duration: 42,
                session_id: session,
            },
            platform: Platform::default(),
            accepted_next_servers: Vec::new(),
        };
        let p = build_params(&req, SL_MAIN_GRID);
        assert_eq!(p["last_exec_event"].as_i32(), LastExec::UNKNOWN);
        assert_eq!(p["last_exec_duration"].as_i32(), 42);
        assert_eq!(p["last_exec_session_id"].as_str(), session.to_string());
        let clean = build_params(
            &LoginRequest {
                last_exec: LastExec::default(),
                ..req
            },
            SL_MAIN_GRID,
        );
        assert_eq!(clean["last_exec_event"].as_i32(), 0);
        assert_eq!(clean["last_exec_duration"].as_i32(), -1);
        assert_eq!(clean["last_exec_session_id"].as_str(), Uuid::nil().to_string());
    }

    #[test]
    fn only_linden_lab_hosts_are_linden_grids() {
        for uri in [
            SL_MAIN_GRID,
            SL_BETA_GRID,
            "https://LOGIN.agni.lindenlab.com./cgi-bin/login.cgi",
            "https://lindenlab.com/",
        ] {
            assert!(is_linden_login(uri), "{uri}");
        }
        for uri in [
            "https://grid.example:8002/",
            "https://lindenlab.com.grid.example/",
            "https://grid.example/lindenlab.com",
            "https://login.agni.lindenlab.com@grid.example/",
            "https://notlindenlab.com/",
            "not an address",
            "",
        ] {
            assert!(!is_linden_login(uri), "{uri}");
        }
    }

    #[test]
    fn other_grids_get_their_own_device_ids() {
        let real = format!("{:x}", md5::compute("synthetic machine"));
        // Linden Lab: the real hash, as with their viewer
        assert_eq!(device_id_for(SL_MAIN_GRID, &real), real);
        assert_eq!(device_id_for(SL_BETA_GRID, &real), real);
        let a = device_id_for("https://grid.example:8002/", &real);
        let b = device_id_for("https://other.example/", &real);
        for derived in [&a, &b] {
            // same form: 32 lower case hex digits
            assert_eq!(derived.len(), 32);
            assert!(derived.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert_ne!(*derived, real);
        }
        assert_ne!(a, b, "two grids cannot match the machine");
        // steady for one grid, whatever the path or the case of its host
        assert_eq!(device_id_for("https://GRID.example:8002/other/path", &real), a);
        // a host merely naming Linden Lab is another grid
        assert_ne!(device_id_for("https://lindenlab.com.grid.example/", &real), real);
        // nothing read stays nothing
        assert_eq!(device_id_for("https://grid.example/", ""), "");
    }

    #[test]
    fn the_login_sends_the_device_ids_of_its_grid() {
        let real_mac = format!("{:x}", md5::compute("synthetic adapter"));
        let real_id0 = format!("{:x}", md5::compute("synthetic disk"));
        let req = |login_uri: &str| LoginRequest {
            login_uri: login_uri.into(),
            username: "synthetic".into(),
            password: "x".into(),
            start: StartLocation::Last,
            agree_to_tos: false,
            read_critical: false,
            mfa_token: String::new(),
            mfa_hash: String::new(),
            mac: real_mac.clone(),
            id0: real_id0.clone(),
            last_exec: LastExec::default(),
            platform: Platform::windows("26200", true),
            accepted_next_servers: Vec::new(),
        };
        let linden = build_params(&req(SL_MAIN_GRID), SL_MAIN_GRID);
        assert_eq!(linden["mac"].as_str(), real_mac);
        assert_eq!(linden["id0"].as_str(), real_id0);
        let other = build_params(&req("https://grid.example:8002/"), "https://grid.example:8002/");
        assert_eq!(other["mac"].as_str(), device_id_for("https://grid.example:8002/", &real_mac));
        assert_eq!(other["id0"].as_str(), device_id_for("https://grid.example:8002/", &real_id0));
        assert_ne!(other["mac"].as_str(), real_mac);
        assert_ne!(other["id0"].as_str(), real_id0);
        // sent on to another grid: that grid's ids, never the first one's
        let moved = build_params(&req(SL_MAIN_GRID), "https://grid.example:8002/");
        assert_eq!(moved["mac"].as_str(), other["mac"].as_str());
        assert_eq!(moved["id0"].as_str(), other["id0"].as_str());
        // the password keeps the form of the account's grid
        assert_eq!(moved["passwd"].as_str(), linden["passwd"].as_str());
    }

    #[test]
    fn a_login_goes_on_alone_only_to_the_same_server() {
        let grid = "https://grid.example:8002/login";
        let none: &[String] = &[];
        // the same server: its path, the case of its host, http to https
        assert_eq!(next_server(grid, "https://GRID.example:8002/other", none), NextServer::Follow);
        assert_eq!(
            next_server("http://grid.example/", "https://grid.example/", none),
            NextServer::Follow
        );
        assert_eq!(
            next_server("http://grid.example/", "http://grid.example/next", none),
            NextServer::Follow
        );
        // one Linden Lab login server to another
        assert_eq!(
            next_server(SL_MAIN_GRID, "https://login2.lindenlab.com/cgi-bin/login.cgi", none),
            NextServer::Follow
        );
        // another server, another port, a host merely naming the first: asked
        for to in [
            "https://other.example/",
            "https://grid.example:9000/",
            "https://grid.example.other.example:8002/",
            "https://grid.example:8002@other.example/",
        ] {
            assert!(matches!(next_server(grid, to, none), NextServer::Ask(_)), "{to}");
        }
        assert_eq!(
            next_server(grid, "https://Other.example/", none),
            NextServer::Ask("other.example".into())
        );
        assert!(matches!(
            next_server(SL_MAIN_GRID, "https://lindenlab.com.other.example/", none),
            NextServer::Ask(_)
        ));
        assert!(matches!(
            next_server("http://grid.example/", "http://other.example/", none),
            NextServer::Ask(_)
        ));
        // once agreed, that server only
        let agreed = ["other.example".to_owned()];
        assert_eq!(next_server(grid, "https://other.example/next", &agreed), NextServer::Follow);
        assert!(matches!(next_server(grid, "https://third.example/", &agreed), NextServer::Ask(_)));
        // encrypted to unencrypted, or no web address: never, agreed or not
        for to in [
            "http://grid.example:8002/",
            "http://other.example/",
            "ftp://other.example/",
            "file:///login",
            "",
            "not an address",
        ] {
            assert_eq!(next_server(grid, to, &agreed), NextServer::Refuse, "{to}");
        }
        assert_eq!(
            next_server(SL_MAIN_GRID, "http://login.agni.lindenlab.com/", none),
            NextServer::Refuse
        );
    }

    /// The login names the real system, as LLOSInfo writes it, and leaves
    /// out a version it does not know.
    #[test]
    fn the_login_names_the_real_system() {
        assert_eq!(
            Platform::windows("26200", true),
            Platform {
                version: "10.0.26200".into(),
                name: "Microsoft Windows 11 64-bit".into()
            }
        );
        assert_eq!(Platform::windows("19045", false).name, "Microsoft Windows 10");
        assert_eq!(Platform::windows("", true).version, "");
        assert_eq!(
            Platform::macos("14.5"),
            Platform {
                version: "14.5.0".into(),
                name: "macOS 14.5.0".into()
            }
        );
        assert_eq!(Platform::macos("15.1.1").version, "15.1.1");
        assert_eq!(
            Platform::linux("6.8.0-45-generic"),
            Platform {
                version: "6.8.0".into(),
                name: "Linux 6.8".into()
            }
        );
        assert_eq!(Platform::linux("6.17-rc1").name, "Linux 6.17");
        assert_eq!(Platform::linux("unknown").version, "");
        let req = |platform: Platform| LoginRequest {
            login_uri: SL_MAIN_GRID.into(),
            username: "synthetic".into(),
            password: "x".into(),
            start: StartLocation::Last,
            agree_to_tos: false,
            read_critical: false,
            mfa_token: String::new(),
            mfa_hash: String::new(),
            mac: String::new(),
            id0: String::new(),
            last_exec: LastExec::default(),
            platform,
            accepted_next_servers: Vec::new(),
        };
        let p = build_params(&req(Platform::windows("26200", true)), SL_MAIN_GRID);
        assert_eq!(p["platform_version"].as_str(), "10.0.26200");
        assert_eq!(p["platform_string"].as_str(), "Microsoft Windows 11 64-bit");
        let unknown = build_params(&req(Platform::default()), SL_MAIN_GRID);
        assert!(!unknown.has("platform_version"));
        assert_eq!(unknown["platform_string"].as_str(), std::env::consts::OS);
    }

    /// A loopback login server: each connection gets the next reply
    /// (status line, extra header lines, XML-RPC body); `hits` counts the
    /// requests it read.
    async fn login_server(replies: Vec<(&'static str, String, String)>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}/login", listener.local_addr().expect("address"));
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        tokio::spawn(async move {
            for (status, headers, body) in replies {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 8192];
                    let len = socket.read(&mut buffer).await.expect("read");
                    assert!(len > 0);
                    bytes.extend_from_slice(&buffer[..len]);
                    if let Some(start) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&bytes[..start]).to_ascii_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                            .unwrap_or(0);
                        if bytes.len() >= start + 4 + length {
                            break;
                        }
                    }
                }
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let head = format!(
                    "HTTP/1.1 {status}\r\n{headers}Content-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.expect("write");
                socket.write_all(body.as_bytes()).await.expect("write");
                crate::test_http::finish(socket);
            }
        });
        (url, hits)
    }

    /// An XML-RPC login answer with these string members.
    fn answer(members: &[(&str, &str)]) -> String {
        let members: String = members
            .iter()
            .map(|(name, value)| format!("<member><name>{name}</name><value><string>{value}</string></value></member>"))
            .collect();
        format!(
            "<?xml version='1.0'?><methodResponse><params><param><value><struct>{members}</struct></value></param></params></methodResponse>"
        )
    }

    fn wrong_password() -> String {
        answer(&[("login", "false"), ("reason", "key"), ("message", "synthetic refusal")])
    }

    fn synthetic_request(login_uri: &str, accepted: &[String]) -> LoginRequest {
        LoginRequest {
            login_uri: login_uri.into(),
            username: "synthetic".into(),
            password: "x".into(),
            start: StartLocation::Last,
            agree_to_tos: false,
            read_critical: false,
            mfa_token: String::new(),
            mfa_hash: String::new(),
            mac: String::new(),
            id0: String::new(),
            last_exec: LastExec::default(),
            platform: Platform::default(),
            accepted_next_servers: accepted.to_vec(),
        }
    }

    fn authority_of(url: &str) -> String {
        login_authority(url).expect("an authority")
    }

    #[tokio::test]
    async fn another_server_gets_the_login_only_once_agreed() {
        let http = reqwest::Client::builder().redirect(redirect_policy()).build().expect("client");
        // "indeterminate" to another server: nothing sent there, the user is asked
        let (other, other_hits) = login_server(vec![("200 OK", String::new(), wrong_password())]).await;
        let next = answer(&[
            ("login", "indeterminate"),
            ("next_url", &other),
            ("next_method", "login_to_simulator"),
        ]);
        let (grid, _) = login_server(vec![("200 OK", String::new(), next.clone()), ("200 OK", String::new(), next)]).await;
        let asked = login(&http, &synthetic_request(&grid, &[])).await;
        assert!(
            matches!(&asked, Err(LoginError::NextServer(server)) if *server == authority_of(&other)),
            "{asked:?}"
        );
        assert_eq!(other_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        // agreed: the login goes on there
        let agreed = login(&http, &synthetic_request(&grid, &[authority_of(&other)])).await;
        assert!(
            matches!(agreed, Err(LoginError::Refused { ref reason, .. }) if reason == "key"),
            "{agreed:?}"
        );
        assert_eq!(other_hits.load(std::sync::atomic::Ordering::SeqCst), 1);

        // an HTTP 307 to another server: stopped by the client, asked the same way
        let (other, other_hits) = login_server(vec![("200 OK", String::new(), wrong_password())]).await;
        let (grid, _) = login_server(vec![("307 Temporary Redirect", format!("Location: {other}\r\n"), String::new())]).await;
        let asked = login(&http, &synthetic_request(&grid, &[])).await;
        assert!(
            matches!(&asked, Err(LoginError::NextServer(server)) if *server == authority_of(&other)),
            "{asked:?}"
        );
        assert_eq!(other_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_same_server_is_followed_without_a_question() {
        let http = reqwest::Client::builder().redirect(redirect_policy()).build().expect("client");
        // a 307 to another path, then a relative next_url: the same server
        let (grid, hits) = login_server(vec![
            ("307 Temporary Redirect", "Location: /login2\r\n".into(), String::new()),
            (
                "200 OK",
                String::new(),
                answer(&[("login", "indeterminate"), ("next_url", "/login3")]),
            ),
            ("200 OK", String::new(), wrong_password()),
        ])
        .await;
        let refused = login(&http, &synthetic_request(&grid, &[])).await;
        assert!(
            matches!(refused, Err(LoginError::Refused { ref reason, .. }) if reason == "key"),
            "{refused:?}"
        );
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn other_grids_get_their_own_adapter_address() {
        let node = [0x00, 0x1b, 0x21, 0x3c, 0x4d, 0x5e];
        assert_eq!(adapter_for(SL_MAIN_GRID, Some(node)), Some(node));
        assert_eq!(adapter_for("https://grid.example/", None), None);
        let a = adapter_for("https://grid.example/", Some(node)).expect("an address");
        let b = adapter_for("https://other.example/", Some(node)).expect("an address");
        assert_ne!(a, node);
        assert_ne!(a, b);
        // locally administered, unicast
        assert_eq!(a[0] & 0x03, 0x02);
        assert_eq!(adapter_for("https://grid.example/", Some(node)), Some(a));
    }

    #[test]
    fn ll_vector() {
        let v = parse_ll_vector("[r0.98, r0.17, r0]").unwrap();
        assert!((v.x - 0.98).abs() < 1e-6 && (v.y - 0.17).abs() < 1e-6 && v.z == 0.0);
    }
}
