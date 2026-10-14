//! Photos to the profile feed (LLWebProfile::uploadImageCoro / buildPostData
//! and LLViewerMedia::getOpenIDCookieCoro, LLPanelSnapshotProfile,
//! indra/newview, originally LGPL 2.1). The web profile is reached with the
//! OpenID cookie of the login (`media::openid`): a first GET of the profile
//! gives its own session cookie (the "auth cookie"), then
//! `snapshots/s3_upload_config` gives the form of the storage the PNG is
//! posted to, whose 303 is followed to finish the post.
//!
//! The cookies are session secrets: never logged.

use super::Aspect;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// LLWebProfile::MAX_WEB_DATASIZE: a larger PNG is likely refused.
pub const MAX_WEB_DATASIZE: usize = 8 * 1024 * 1024;
/// The caption's text editor (panel_snapshot_profile.xml, max_length).
pub const MAX_CAPTION: usize = 700;
/// buildPostData's boundary.
const BOUNDARY: &str = "----------------------------0123abcdefab";
const TIMEOUT: Duration = Duration::from_secs(60);

/// The sizes of panel_snapshot_profile.xml's combo (Current Window,
/// 640x480, 800x600, 1024x768), as long sides with the chosen proportions
/// (LastSnapshotToProfileResolution: 640 by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileSize {
    Window,
    S640,
    S800,
    S1024,
}

impl ProfileSize {
    pub const ALL: [ProfileSize; 4] = [ProfileSize::S640, ProfileSize::S800, ProfileSize::S1024, ProfileSize::Window];

    pub fn side(self) -> Option<u32> {
        match self {
            ProfileSize::S640 => Some(640),
            ProfileSize::S800 => Some(800),
            ProfileSize::S1024 => Some(1024),
            ProfileSize::Window => None,
        }
    }
}

/// The size of a photo for the profile: the preset as its long side with
/// the proportions, or the window's size.
pub fn capture_size(aspect: Aspect, size: ProfileSize, window: (u32, u32)) -> (u32, u32) {
    match size.side() {
        Some(long) => super::photo_size(aspect, super::Resolution::Custom, sides(aspect, long, window), window),
        None => super::photo_size(aspect, super::Resolution::Window, [0, 0], window),
    }
}

fn sides(aspect: Aspect, long: u32, window: (u32, u32)) -> [u32; 2] {
    let ratio = aspect.ratio((window.0.max(1), window.1.max(1)));
    if ratio >= 1.0 {
        [long, (long as f32 / ratio).round() as u32]
    } else {
        [(long as f32 * ratio).round() as u32, long]
    }
}

/// The grid's web profile address (LLGridManager::getWebProfileURL):
/// Second Life's, the beta grid's, or the `web_profile_url` of the login.
pub fn grid_web_profile(grid: &crate::settings::GridChoice, login_url: &str) -> Option<String> {
    match grid {
        crate::settings::GridChoice::SecondLife => Some("https://my.secondlife.com/".to_owned()),
        crate::settings::GridChoice::SecondLifeBeta => Some("https://my.secondlife-beta.com/".to_owned()),
        crate::settings::GridChoice::Custom => Some(login_url.to_owned()).filter(|u| !u.trim().is_empty()),
    }
}

/// The grid's web profile address, lowercase like getProfileURL("").
pub fn profile_url(base: &str) -> Option<String> {
    let base = base.trim();
    if base.is_empty() {
        return None;
    }
    let mut url = base.to_lowercase();
    if !url.ends_with('/') {
        url.push('/');
    }
    Some(url)
}

/// What a post needs: the profile's address, the OpenID cookie of the
/// login (raw, as Firestorm sends it) and the user agent.
#[derive(Clone)]
pub struct Web {
    pub profile: String,
    pub openid: String,
    pub user_agent: String,
}

/// Where a post stopped (logged; the window says it failed, like
/// reportImageUploadStatus).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The web profile gave no session cookie.
    NoCookie,
    /// The upload configuration was refused (HTTP status).
    Config(u16),
    /// The upload configuration could not be read.
    ConfigData,
    /// The image was refused (HTTP status).
    Upload(u16),
    /// No address to finish the post.
    NoRedirect,
    /// The post's last step failed (HTTP status).
    Finish(u16),
    Network,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoCookie => write!(f, "the web profile gave no cookie"),
            Error::Config(s) => write!(f, "upload configuration refused (HTTP {s})"),
            Error::ConfigData => write!(f, "upload configuration unreadable"),
            Error::Upload(s) => write!(f, "image refused (HTTP {s})"),
            Error::NoRedirect => write!(f, "no address after the upload"),
            Error::Finish(s) => write!(f, "post not finished (HTTP {s})"),
            Error::Network => write!(f, "network error"),
        }
    }
}

/// LLURI::escape: everything but the unreserved characters percent-encoded.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// LLWebProfile::buildDefaultHeaders (LLViewerMedia::getHeaders) and the
/// auth cookie appended to them.
fn request(http: &reqwest::Client, method: reqwest::Method, url: &str, web: &Web, auth: Option<&str>) -> reqwest::RequestBuilder {
    let mut r = http
        .request(method, url)
        .header("Accept", "*/*")
        .header("Content-Type", "application/xml")
        .header("Cookie", web.openid.as_str())
        .header("User-Agent", web.user_agent.as_str())
        .timeout(TIMEOUT);
    if let Some(auth) = auth {
        r = r.header("Cookie", auth);
    }
    r
}

/// getOpenIDCookieCoro: the web profile with the OpenID cookie, redirects
/// followed; its Set-Cookie, path stripped, is the auth cookie of the posts.
pub async fn auth_cookie(web: &Web) -> Result<String, Error> {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|_| Error::Network)?;
    let response = http
        .get(&web.profile)
        .header("Accept", "*/*")
        .header("Cookie", web.openid.as_str())
        .header("User-Agent", web.user_agent.as_str())
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() {
        log::warn!("photo: web profile: HTTP {}", response.status().as_u16());
        return Err(Error::NoCookie);
    }
    let cookie = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|c| c.split(';').next().unwrap_or("").trim().to_owned())
        .filter(|c| !c.is_empty());
    cookie.ok_or(Error::NoCookie)
}

/// uploadImageCoro: the upload configuration, the form posted to its URL,
/// then the 303's address.
pub async fn post(web: &Web, auth: &str, png: Vec<u8>, caption: &str, add_location: bool) -> Result<(), Error> {
    // setFollowRedirects(false): the 303 is read, then followed by hand
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Error::Network)?;
    let config_url = format!(
        "{}snapshots/s3_upload_config?caption={}&add_loc={}",
        web.profile,
        escape(caption),
        if add_location { "1" } else { "0" }
    );
    let response = request(&http, reqwest::Method::GET, &config_url, web, Some(auth))
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() {
        return Err(Error::Config(response.status().as_u16()));
    }
    let bytes = response.bytes().await.map_err(|_| Error::Network)?;
    let config: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| Error::ConfigData)?;
    let upload_url = config["url"].as_str().filter(|u| !u.is_empty()).ok_or(Error::ConfigData)?;
    let body = form(&config["data"], &png);
    let response = request(&http, reqwest::Method::POST, upload_url, web, Some(auth))
        .header("Content-Type", format!("multipart/form-data; boundary={BOUNDARY}"))
        .body(body)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    let status = response.status();
    if !status.is_success() && status != reqwest::StatusCode::SEE_OTHER {
        return Err(Error::Upload(status.as_u16()));
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .ok_or(Error::NoRedirect)?;
    let response = request(&http, reqwest::Method::GET, &location, web, Some(auth))
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(Error::Finish(response.status().as_u16()));
    }
    Ok(())
}

/// buildPostData: the storage form's fields, in the order the server needs,
/// then the PNG.
pub fn form(data: &serde_json::Value, png: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(png.len() + 2048);
    for name in [
        "key",
        "AWSAccessKeyId",
        "acl",
        "Content-Type",
        "policy",
        "signature",
        "success_action_redirect",
    ] {
        let value = match &data[name] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"snapshot.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// The PNG of a photo (RGBA rows top to bottom), like LLImagePNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err("photo pixels".into());
    }
    let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut out = Vec::new();
    let progress = std::sync::atomic::AtomicU32::new(0);
    super::save::encode(&mut out, width, height, &rgb, super::FileFormat::Png, 0, &progress)?;
    Ok(out)
}

pub mod demo;
