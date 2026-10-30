//! World › Set Home to Here: the `HomeLocation` capability and its UDP
//! fallback, as LLAgent::setStartPosition / setStartPositionSuccess send and
//! read them (indra/newview/llagent.cpp, originally LGPL 2.1), and the
//! simulator's answers in its alerts (process_special_alert_messages and the
//! HomePositionSet / CantSetHomeAtRegion / ListValidHomeLocations /
//! ResetHomePositionNotLegal notifications, llviewermessage.cpp).
//!
//! The simulator decides: it checks the land (our own, a mainland infohub, a
//! group parcel whose role has "Allow Set Home"…) and says so in an alert,
//! "Home position set." when it agrees.

use crate::caps::{self, CapsError};
use aurora_llsd::{Llsd, llsd_map};
use glam::Vec3;

/// START_LOCATION_ID_HOME (EStartLocation, llstartup.h).
pub const START_LOCATION_ID_HOME: u32 = 1;

/// REGION_FLAGS_ALLOW_SET_HOME (llregionflags.h): World › Set Home to Here
/// is offered only there (LLWorldEnableSetHomeLocation).
pub const REGION_ALLOW_SET_HOME: u32 = 1 << 2;

/// ll_sdmap_from_vector3: `{X, Y, Z}` as reals.
fn sdmap(v: Vec3) -> Llsd {
    llsd_map! { "X" => v.x as f64, "Y" => v.y as f64, "Z" => v.z as f64 }
}

/// The capability's body: `{HomeLocation: {LocationId, LocationPos,
/// LocationLookAt}}`, the position in region coordinates at the agent's feet.
pub fn request_body(position: Vec3, look_at: Vec3) -> Llsd {
    llsd_map! {
        "HomeLocation" => llsd_map! {
            "LocationId" => START_LOCATION_ID_HOME as i32,
            "LocationPos" => sdmap(position),
            "LocationLookAt" => sdmap(look_at),
        }
    }
}

/// What the capability answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reply {
    /// The home the simulator kept, in the agent's region (it may move it:
    /// a "justified" position, whole metres).
    Set(Vec3),
    /// `success` false or no position: the simulator's alert says why.
    Refused,
    /// The request itself failed (no answer, an HTTP error).
    Failed,
}

/// setStartPositionSuccess: `success` must read "true", and
/// `HomeLocation.LocationPos` hold X, Y and Z, taken as integers
/// (LLSD::asInteger truncates).
pub fn parse_reply(reply: &Llsd) -> Reply {
    let success = match reply.get("success") {
        Llsd::Boolean(b) => *b,
        other => other.to_string_value().starts_with("true"),
    };
    let pos = &reply["HomeLocation"]["LocationPos"];
    if !success || !["X", "Y", "Z"].iter().all(|k| pos.has(k)) {
        return Reply::Refused;
    }
    let whole = |v: &Llsd| match v {
        Llsd::Real(r) => r.trunc() as f32,
        other => other.as_i32() as f32,
    };
    Reply::Set(Vec3::new(whole(&pos["X"]), whole(&pos["Y"]), whole(&pos["Z"])))
}

/// POST to the capability, and what it answered.
pub async fn post(http: &reqwest::Client, url: &str, position: Vec3, look_at: Vec3) -> Reply {
    match caps::post_llsd(http, url, &request_body(position, look_at)).await {
        Ok(reply) => parse_reply(&reply),
        Err(e) => {
            log_failure(&e);
            Reply::Failed
        }
    }
}

fn log_failure(e: &CapsError) {
    log::warn!("home: the HomeLocation request failed ({e})");
}

/// FIRE-19894's fallback when the region has no capability (OpenSim):
/// SetStartLocationRequest, the region's name left for the simulator.
pub fn udp_request(
    agent_id: uuid::Uuid,
    session_id: uuid::Uuid,
    position: Vec3,
    look_at: Vec3,
) -> aurora_msg::msgs::SetStartLocationRequest {
    let mut m = aurora_msg::msgs::SetStartLocationRequest::default();
    m.agent_data.agent_id = agent_id;
    m.agent_data.session_id = session_id;
    m.start_location_data.sim_name = aurora_msg::str_field("");
    m.start_location_data.location_id = START_LOCATION_ID_HOME;
    m.start_location_data.location_pos = position;
    m.start_location_data.location_look_at = look_at;
    m
}

/// A simulator alert about the home location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    /// "Home position set." (HomePositionSet, SetHomePosition).
    Set,
    /// CantSetHomeAtRegion: the region does not allow it.
    NotInRegion,
    /// ListValidHomeLocations: only on our land or at a mainland infohub.
    NotHere,
    /// ResetHomePositionNotLegal: the old home was not valid any more.
    Reset,
}

/// The notification an AlertInfo names (attempt_standard_notification).
pub fn from_alert_info(name: &str) -> Option<Alert> {
    match name {
        "HomePositionSet" | "SetHomePosition" => Some(Alert::Set),
        "CantSetHomeAtRegion" => Some(Alert::NotInRegion),
        "ListValidHomeLocations" => Some(Alert::NotHere),
        "ResetHomePositionNotLegal" => Some(Alert::Reset),
        _ => None,
    }
}

/// process_special_alert_messages: the plain text the simulator sends with
/// AlertMessage or AgentAlertMessage when it agrees.
pub fn from_text(message: &str) -> Option<Alert> {
    (message == "Home position set.").then_some(Alert::Set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_request_names_home_with_the_feet_and_the_look_at() {
        let body = request_body(Vec3::new(128.25, 40.5, 22.0), Vec3::new(0.0, 1.0, 0.0));
        let decoded = aurora_llsd::from_xml(&aurora_llsd::to_xml(&body)).expect("home body");
        let home = &decoded["HomeLocation"];
        assert_eq!(home["LocationId"].as_i32(), 1);
        assert_eq!(home["LocationPos"]["X"].as_f64(), 128.25);
        assert_eq!(home["LocationPos"]["Y"].as_f64(), 40.5);
        assert_eq!(home["LocationPos"]["Z"].as_f64(), 22.0);
        assert_eq!(home["LocationLookAt"]["Y"].as_f64(), 1.0);
        assert_eq!(home.as_map().expect("map").len(), 3);
        assert_eq!(decoded.as_map().expect("map").len(), 1);
    }

    #[test]
    fn the_reply_keeps_whole_metres_and_refuses_without_success_or_position() {
        let pos = llsd_map! { "X" => 33.9, "Y" => 40, "Z" => 22.75 };
        let ok = llsd_map! { "success" => true, "HomeLocation" => llsd_map! { "LocationPos" => pos.clone() } };
        assert_eq!(parse_reply(&ok), Reply::Set(Vec3::new(33.0, 40.0, 22.0)));
        // the Legacy Viewer compares the text with "true"
        let text = llsd_map! { "success" => "true", "HomeLocation" => llsd_map! { "LocationPos" => pos.clone() } };
        assert_eq!(parse_reply(&text), Reply::Set(Vec3::new(33.0, 40.0, 22.0)));
        let refused = llsd_map! { "success" => false, "HomeLocation" => llsd_map! { "LocationPos" => pos } };
        assert_eq!(parse_reply(&refused), Reply::Refused);
        let no_z = llsd_map! { "success" => true, "HomeLocation" => llsd_map! { "LocationPos" => llsd_map! { "X" => 1, "Y" => 2 } } };
        assert_eq!(parse_reply(&no_z), Reply::Refused);
        assert_eq!(parse_reply(&Llsd::Undef), Reply::Refused);
    }

    #[test]
    fn the_udp_fallback_carries_the_same_place() {
        let (agent, session) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
        let m = udp_request(agent, session, Vec3::new(10.0, 20.0, 30.0), Vec3::X);
        let mut wire = Vec::new();
        aurora_msg::build_packet(&m, 1, true, false, &[], &mut wire);
        let decoded: aurora_msg::msgs::SetStartLocationRequest = aurora_msg::parse_packet(&wire).expect("packet").decode().expect("decode");
        assert_eq!(decoded.agent_data.agent_id, agent);
        assert_eq!(decoded.agent_data.session_id, session);
        assert_eq!(decoded.start_location_data.location_id, START_LOCATION_ID_HOME);
        assert_eq!(decoded.start_location_data.location_pos, Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(decoded.start_location_data.location_look_at, Vec3::X);
        assert_eq!(aurora_msg::field_str(&decoded.start_location_data.sim_name), "");
    }

    #[test]
    fn the_simulator_alerts_are_read_by_name_and_by_text() {
        assert_eq!(from_alert_info("HomePositionSet"), Some(Alert::Set));
        assert_eq!(from_alert_info("CantSetHomeAtRegion"), Some(Alert::NotInRegion));
        assert_eq!(from_alert_info("ListValidHomeLocations"), Some(Alert::NotHere));
        assert_eq!(from_alert_info("ResetHomePositionNotLegal"), Some(Alert::Reset));
        assert_eq!(from_alert_info("RegionRestartMinutes"), None);
        assert_eq!(from_text("Home position set."), Some(Alert::Set));
        assert_eq!(from_text("Home position set"), None);
    }

    /// One request to a local capability, its body, and the reply sent back.
    async fn serve_once(status: &'static str, reply: Llsd) -> (String, tokio::task::JoinHandle<Llsd>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}/home", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("request");
            let mut bytes = Vec::new();
            let body = loop {
                let mut buffer = [0; 4096];
                let len = socket.read(&mut buffer).await.expect("read");
                assert!(len > 0);
                bytes.extend_from_slice(&buffer[..len]);
                if let Some(start) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..start]).to_ascii_lowercase();
                    assert!(headers.starts_with("post /home "), "{headers}");
                    let length = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                        .unwrap_or(0);
                    if bytes.len() >= start + 4 + length {
                        break bytes[start + 4..start + 4 + length].to_vec();
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
            aurora_llsd::from_xml(&body).expect("request body")
        });
        (url, server)
    }

    #[tokio::test]
    async fn the_capability_is_posted_and_its_answers_and_errors_read() {
        let http = reqwest::Client::new();
        let kept = llsd_map! {
            "success" => true,
            "HomeLocation" => llsd_map! { "LocationPos" => llsd_map! { "X" => 128, "Y" => 64, "Z" => 25 } },
        };
        let (url, server) = serve_once("200 OK", kept).await;
        let reply = post(&http, &url, Vec3::new(128.4, 64.6, 25.2), Vec3::X).await;
        assert_eq!(reply, Reply::Set(Vec3::new(128.0, 64.0, 25.0)));
        let sent = server.await.expect("server");
        assert_eq!(sent["HomeLocation"]["LocationId"].as_i32(), 1);
        assert!((sent["HomeLocation"]["LocationPos"]["Y"].as_f64() - 64.6).abs() < 1e-4);

        let (url, server) = serve_once("200 OK", llsd_map! { "success" => false }).await;
        assert_eq!(post(&http, &url, Vec3::ONE, Vec3::X).await, Reply::Refused);
        server.await.expect("server");

        let (url, server) = serve_once("500 Internal Server Error", Llsd::Undef).await;
        assert_eq!(post(&http, &url, Vec3::ONE, Vec3::X).await, Reply::Failed);
        server.await.expect("server");
    }
}
