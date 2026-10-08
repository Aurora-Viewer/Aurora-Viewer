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
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), ".0");

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
    /// Stable hashed machine identifiers.
    pub mac: String,
    pub id0: String,
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
    Refused { reason: String, message: String },
    #[error("two-factor authentication required: {0}")]
    MfaRequired(String),
    #[error("terms of service must be accepted: {0}")]
    TosRequired(String),
    #[error("critical message must be read: {0}")]
    CriticalMessage(String),
    #[error("malformed login response: {0}")]
    Malformed(&'static str),
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

fn platform_version() -> String {
    // Kept generic: the exact build number is not needed by the grid.
    if cfg!(windows) {
        "10.0".into()
    } else {
        std::env::consts::OS.into()
    }
}

fn build_params(req: &LoginRequest) -> Llsd {
    let (first, last) = split_username(&req.username);
    let mut p = Map::new();
    p.insert("first".into(), first.into());
    p.insert("last".into(), last.into());
    p.insert("passwd".into(), hash_password(&req.password, is_linden_grid(&req.login_uri)).into());
    p.insert("start".into(), req.start.to_login_string().into());
    p.insert("channel".into(), CHANNEL.into());
    p.insert("version".into(), VERSION.into());
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
    p.insert("platform_version".into(), platform_version().into());
    p.insert("platform_string".into(), std::env::consts::OS.into());
    p.insert("address_size".into(), 64.into());
    p.insert("mac".into(), req.mac.clone().into());
    p.insert("id0".into(), req.id0.clone().into());
    p.insert("agree_to_tos".into(), req.agree_to_tos.into());
    p.insert("read_critical".into(), req.read_critical.into());
    p.insert("last_exec_event".into(), 0.into());
    p.insert("last_exec_duration".into(), 0.into());
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
    let size = |k: &str| match v[k].as_i32() {
        n if n >= 256 => n as u32,
        _ => 256,
    };
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
        seed_capability: v["seed_capability"].as_str().to_owned(),
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

/// Perform the login exchange, following "indeterminate" redirects.
pub async fn login(http: &reqwest::Client, req: &LoginRequest) -> Result<LoginResponse, LoginError> {
    let mut uri = req.login_uri.clone();
    let mut method = "login_to_simulator".to_owned();
    for _attempt in 0..5 {
        let body = xmlrpc::build_call(&method, &build_params(req));
        let resp = http
            .post(&uri)
            .header("Content-Type", "text/xml")
            .body(body)
            .send()
            .await
            .map_err(|e| LoginError::Http(e.to_string()))?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| LoginError::Http(e.to_string()))?;
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
                uri = next.to_owned();
                let m = v["next_method"].as_str();
                if !m.is_empty() {
                    method = m.to_owned();
                }
                continue;
            }
            _ => {
                let reason = v["reason"].as_str().to_owned();
                let message = v["message"].as_str().to_owned();
                return Err(match reason.as_str() {
                    "mfa_challenge" => LoginError::MfaRequired(message),
                    "tos" => LoginError::TosRequired(message),
                    "critical" => LoginError::CriticalMessage(message),
                    _ => LoginError::Refused { reason, message },
                });
            }
        }
    }
    Err(LoginError::Malformed("too many redirects"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn ll_vector() {
        let v = parse_ll_vector("[r0.98, r0.17, r0]").unwrap();
        assert!((v.x - 0.98).abs() < 1e-6 && (v.y - 0.17).abs() < 1e-6 && v.z == 0.0);
    }
}
