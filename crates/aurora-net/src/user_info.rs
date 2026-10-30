//! The account's directory settings of Preferences › Privacy: who sees that
//! we are online, and offline IMs sent by e-mail. The `UserInfo` capability
//! and its UDP fallback (UserInfoRequest / UserInfoReply / UpdateUserInfo),
//! as LLAgent::sendAgentUserInfoRequest / requestAgentUserInfoCoro /
//! sendAgentUpdateUserInfo / updateAgentUserInfoCoro send and read them
//! (indra/newview/llagent.cpp, originally LGPL 2.1).
//!
//! Second Life keeps the e-mail setting on its account web page: the Legacy
//! Viewer neither reads `im_via_email` there nor sends it, and only on other
//! grids (OpenSim) shows the checkbox. The viewer applies that rule
//! (`world/user_info.rs`); this module reads and sends what it is given.

use crate::caps::{self, CapsError};
use aurora_llsd::{Llsd, llsd_map};

/// VISIBILITY_DEFAULT (llfloaterpreference.cpp): everyone sees us online.
pub const VISIBILITY_DEFAULT: &str = "default";
/// VISIBILITY_HIDDEN: only friends and groups see us online.
pub const VISIBILITY_HIDDEN: &str = "hidden";

/// What the server knows of the account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserInfo {
    /// "default", "hidden", or another value (a liaison's) that the
    /// checkbox shows ticked and cannot change (setPersonalInfo).
    pub directory_visibility: String,
    pub im_via_email: bool,
    pub email: String,
}

impl UserInfo {
    /// "Only friends and groups know I'm online" ticked.
    pub fn hide_online(&self) -> bool {
        self.directory_visibility != VISIBILITY_DEFAULT
    }

    /// The checkbox can change it: "default" or "hidden" only.
    pub fn visibility_editable(&self) -> bool {
        self.directory_visibility == VISIBILITY_DEFAULT || self.directory_visibility == VISIBILITY_HIDDEN
    }
}

/// The checkbox's value as the server spells it (onBtnOK).
pub fn visibility(hide_online: bool) -> &'static str {
    if hide_online { VISIBILITY_HIDDEN } else { VISIBILITY_DEFAULT }
}

/// requestAgentUserInfoCoro: `success` false is an error with the server's
/// `message`.
pub fn parse_reply(reply: &Llsd) -> Result<UserInfo, String> {
    if !reply["success"].as_bool() {
        return Err(reply["message"].to_string_value());
    }
    Ok(UserInfo {
        directory_visibility: reply["directory_visibility"].to_string_value(),
        im_via_email: reply["im_via_email"].as_bool(),
        email: reply["email"].to_string_value(),
    })
}

/// updateAgentUserInfoCoro's body: `dir_visibility`, and `im_via_email` off
/// Second Life only (None there).
pub fn update_body(directory_visibility: &str, im_via_email: Option<bool>) -> Llsd {
    let mut body = llsd_map! { "dir_visibility" => directory_visibility };
    if let Some(email) = im_via_email {
        body.insert("im_via_email", email);
    }
    body
}

/// Why a request did not go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No answer, or an HTTP error.
    Http(String),
    /// `success` false, with the server's message.
    Refused(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(e) => write!(f, "{e}"),
            Error::Refused(m) => write!(f, "refused: {m}"),
        }
    }
}

fn http_error(e: CapsError) -> Error {
    Error::Http(e.to_string())
}

/// GET of the capability (requestAgentUserInfoCoro).
pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<UserInfo, Error> {
    let reply = caps::get_llsd(http, url).await.map_err(http_error)?;
    parse_reply(&reply).map_err(Error::Refused)
}

/// POST of the new settings (updateAgentUserInfoCoro).
pub async fn update(http: &reqwest::Client, url: &str, directory_visibility: &str, im_via_email: Option<bool>) -> Result<(), Error> {
    let reply = caps::post_llsd(http, url, &update_body(directory_visibility, im_via_email))
        .await
        .map_err(http_error)?;
    if reply["success"].as_bool() {
        Ok(())
    } else {
        Err(Error::Refused(reply["message"].to_string_value()))
    }
}

/// sendAgentUserInfoRequestMessage, without the capability.
pub fn udp_request(agent_id: uuid::Uuid, session_id: uuid::Uuid) -> aurora_msg::msgs::UserInfoRequest {
    let mut m = aurora_msg::msgs::UserInfoRequest::default();
    m.agent_data.agent_id = agent_id;
    m.agent_data.session_id = session_id;
    m
}

/// sendAgentUpdateUserInfoMessage: IMViaEMail is left false on Second Life.
pub fn udp_update(
    agent_id: uuid::Uuid,
    session_id: uuid::Uuid,
    directory_visibility: &str,
    im_via_email: Option<bool>,
) -> aurora_msg::msgs::UpdateUserInfo {
    let mut m = aurora_msg::msgs::UpdateUserInfo::default();
    m.agent_data.agent_id = agent_id;
    m.agent_data.session_id = session_id;
    m.user_data.im_via_e_mail = im_via_email.unwrap_or(false);
    m.user_data.directory_visibility = aurora_msg::str_field(directory_visibility);
    m
}

/// process_user_info_reply (llviewermessage.cpp): the UDP answer.
pub fn from_udp(m: &aurora_msg::msgs::UserInfoReply) -> UserInfo {
    UserInfo {
        directory_visibility: aurora_msg::field_str(&m.user_data.directory_visibility),
        im_via_email: m.user_data.im_via_e_mail,
        email: aurora_msg::field_str(&m.user_data.e_mail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_reply_reads_visibility_and_mail_or_the_refusal() {
        let reply = llsd_map! {
            "success" => true,
            "directory_visibility" => "hidden",
            "im_via_email" => true,
            "email" => "resident@example.com",
        };
        let info = parse_reply(&reply).expect("info");
        assert_eq!(info.directory_visibility, "hidden");
        assert!(info.hide_online() && info.visibility_editable());
        assert!(info.im_via_email);
        assert_eq!(info.email, "resident@example.com");
        let refused = llsd_map! { "success" => false, "message" => "no such agent" };
        assert_eq!(parse_reply(&refused), Err("no such agent".to_owned()));
        assert!(parse_reply(&Llsd::Undef).is_err());
    }

    #[test]
    fn a_liaison_visibility_shows_ticked_and_locked() {
        let info = UserInfo {
            directory_visibility: "liaison".into(),
            ..Default::default()
        };
        assert!(info.hide_online());
        assert!(!info.visibility_editable());
        let open = UserInfo {
            directory_visibility: VISIBILITY_DEFAULT.into(),
            ..Default::default()
        };
        assert!(!open.hide_online() && open.visibility_editable());
        assert_eq!(visibility(true), "hidden");
        assert_eq!(visibility(false), "default");
    }

    #[test]
    fn the_update_sends_the_mail_flag_only_off_second_life() {
        let sl = aurora_llsd::from_xml(&aurora_llsd::to_xml(&update_body("hidden", None))).expect("body");
        assert_eq!(sl["dir_visibility"].as_str(), "hidden");
        assert_eq!(sl.as_map().expect("map").len(), 1);
        let os = aurora_llsd::from_xml(&aurora_llsd::to_xml(&update_body("default", Some(true)))).expect("body");
        assert_eq!(os["dir_visibility"].as_str(), "default");
        assert!(os["im_via_email"].as_bool());
    }

    #[test]
    fn the_udp_messages_round_trip() {
        let (agent, session) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
        let mut wire = Vec::new();
        aurora_msg::build_packet(&udp_update(agent, session, "hidden", Some(true)), 1, true, false, &[], &mut wire);
        let m: aurora_msg::msgs::UpdateUserInfo = aurora_msg::parse_packet(&wire).expect("packet").decode().expect("decode");
        assert_eq!(m.agent_data.agent_id, agent);
        assert_eq!(m.agent_data.session_id, session);
        assert!(m.user_data.im_via_e_mail);
        assert_eq!(aurora_msg::field_str(&m.user_data.directory_visibility), "hidden");
        assert!(!udp_update(agent, session, "default", None).user_data.im_via_e_mail);

        let mut wire = Vec::new();
        aurora_msg::build_packet(&udp_request(agent, session), 2, true, false, &[], &mut wire);
        let r: aurora_msg::msgs::UserInfoRequest = aurora_msg::parse_packet(&wire).expect("packet").decode().expect("decode");
        assert_eq!((r.agent_data.agent_id, r.agent_data.session_id), (agent, session));

        let mut reply = aurora_msg::msgs::UserInfoReply::default();
        reply.user_data.im_via_e_mail = true;
        reply.user_data.directory_visibility = aurora_msg::str_field("default");
        reply.user_data.e_mail = aurora_msg::str_field("resident@example.com");
        let info = from_udp(&reply);
        assert_eq!(
            (info.directory_visibility.as_str(), info.im_via_email, info.email.as_str()),
            ("default", true, "resident@example.com")
        );
    }

    /// One request to a local capability: its first line and body, and the
    /// reply sent back.
    async fn serve_once(status: &'static str, reply: Llsd) -> (String, tokio::task::JoinHandle<(String, Vec<u8>)>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}/userinfo", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("request");
            let mut bytes = Vec::new();
            let (line, body) = loop {
                let mut buffer = [0; 4096];
                let len = socket.read(&mut buffer).await.expect("read");
                assert!(len > 0);
                bytes.extend_from_slice(&buffer[..len]);
                if let Some(start) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..start]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                        .unwrap_or(0);
                    if bytes.len() >= start + 4 + length {
                        let line = headers.lines().next().unwrap_or_default().to_owned();
                        break (line, bytes[start + 4..start + 4 + length].to_vec());
                    }
                }
            };
            let out = aurora_llsd::to_xml(&reply);
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                out.len()
            );
            socket.write_all(head.as_bytes()).await.expect("write");
            socket.write_all(&out).await.expect("write");
            crate::test_http::finish(socket);
            (line, body)
        });
        (url, server)
    }

    #[tokio::test]
    async fn the_capability_is_read_and_written_and_its_errors_kept_apart() {
        let http = reqwest::Client::new();
        let info = llsd_map! { "success" => true, "directory_visibility" => "default", "email" => "a@example.com" };
        let (url, server) = serve_once("200 OK", info).await;
        let got = fetch(&http, &url).await.expect("info");
        assert_eq!(got.directory_visibility, "default");
        assert!(server.await.expect("server").0.starts_with("get /userinfo "));

        let (url, server) = serve_once("200 OK", llsd_map! { "success" => false, "message" => "busy" }).await;
        assert_eq!(fetch(&http, &url).await, Err(Error::Refused("busy".into())));
        server.await.expect("server");

        let (url, server) = serve_once("200 OK", llsd_map! { "success" => true }).await;
        assert_eq!(update(&http, &url, "hidden", None).await, Ok(()));
        let (line, body) = server.await.expect("server");
        assert!(line.starts_with("post /userinfo "));
        let sent = aurora_llsd::from_xml(&body).expect("body");
        assert_eq!(sent["dir_visibility"].as_str(), "hidden");
        assert!(!sent.has("im_via_email"));

        let (url, server) = serve_once("200 OK", llsd_map! { "success" => false, "message" => "nope" }).await;
        assert_eq!(
            update(&http, &url, "default", Some(false)).await,
            Err(Error::Refused("nope".into()))
        );
        server.await.expect("server");

        let (url, server) = serve_once("503 Service Unavailable", Llsd::Undef).await;
        assert!(matches!(update(&http, &url, "default", None).await, Err(Error::Http(_))));
        server.await.expect("server");
    }
}
