//! Demo simulator for World › Set Home to Here (`AURORA_DEMO_HOME`) and the
//! account settings of Preferences › Privacy (`AURORA_DEMO_USERINFO`): the
//! answers a Second Life region gives, without a grid.

use aurora_net::NetEvent;
use aurora_net::home::{Alert, Reply};
use aurora_net::user_info::{self, UserInfo};
use glam::Vec3;
use std::sync::Mutex;

/// What the demo region answers to Set Home to Here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The capability keeps the place (whole metres), then "Home position set."
    Set,
    /// Not our land: ListValidHomeLocations.
    NotHere,
    /// CantSetHomeAtRegion.
    NotInRegion,
    /// The capability does not answer.
    Failed,
    /// A region without the capability: the alert alone.
    Udp,
    /// A region without REGION_FLAGS_ALLOW_SET_HOME: the entry is greyed.
    Forbidden,
}

/// `AURORA_DEMO_HOME=<outcome>[:set]`; `:set` asks for it at frame 700.
pub fn scene() -> (Outcome, bool) {
    let value = std::env::var("AURORA_DEMO_HOME").unwrap_or_default();
    let (outcome, auto) = match value.split_once(':') {
        Some((o, rest)) => (o.to_owned(), rest == "set"),
        None => (value, false),
    };
    let outcome = match outcome.as_str() {
        "refused" => Outcome::NotHere,
        "region" => Outcome::NotInRegion,
        "fail" => Outcome::Failed,
        "udp" => Outcome::Udp,
        "forbidden" => Outcome::Forbidden,
        _ => Outcome::Set,
    };
    (outcome, auto)
}

/// The demo region's flag for the World menu.
pub fn region_flag() -> u32 {
    if scene().0 == Outcome::Forbidden {
        0
    } else {
        aurora_net::home::REGION_ALLOW_SET_HOME
    }
}

/// The answers to SetHomeLocation, in the order a region sends them.
pub fn reply(position: Vec3) -> Vec<NetEvent> {
    match scene().0 {
        Outcome::Set => vec![
            NetEvent::HomeLocation(Reply::Set(position.trunc())),
            NetEvent::HomeAlert(Alert::Set),
        ],
        Outcome::NotHere => vec![NetEvent::HomeLocation(Reply::Refused), NetEvent::HomeAlert(Alert::NotHere)],
        Outcome::NotInRegion | Outcome::Forbidden => {
            vec![NetEvent::HomeLocation(Reply::Refused), NetEvent::HomeAlert(Alert::NotInRegion)]
        }
        Outcome::Failed => vec![NetEvent::HomeLocation(Reply::Failed)],
        Outcome::Udp => vec![NetEvent::HomeAlert(Alert::Set)],
    }
}

/// `AURORA_DEMO_USERINFO=hidden|liaison|opensim|fail|savefail`.
fn user_info_scene() -> String {
    std::env::var("AURORA_DEMO_USERINFO").unwrap_or_default()
}

/// The account behaves as on another grid (the e-mail checkbox).
pub fn opensim() -> bool {
    user_info_scene() == "opensim"
}

/// What the demo grid keeps of the account.
static ACCOUNT: Mutex<Option<UserInfo>> = Mutex::new(None);

fn account() -> UserInfo {
    let start = || UserInfo {
        directory_visibility: match user_info_scene().as_str() {
            "hidden" => user_info::VISIBILITY_HIDDEN.into(),
            "liaison" => "liaison".into(),
            _ => user_info::VISIBILITY_DEFAULT.into(),
        },
        im_via_email: true,
        email: "resident@example.com".into(),
    };
    ACCOUNT
        .lock()
        .map(|mut a| a.get_or_insert_with(start).clone())
        .unwrap_or_else(|_| start())
}

/// The answer to RequestUserInfo, or to an UpdateUserInfo that fails.
pub fn user_info_reply(cmd: &aurora_net::NetCommand) -> Option<Vec<NetEvent>> {
    match cmd {
        aurora_net::NetCommand::RequestUserInfo => Some(vec![NetEvent::UserInfo(if user_info_scene() == "fail" {
            Err(user_info::Error::Http("status 502".into()))
        } else {
            Ok(account())
        })]),
        aurora_net::NetCommand::UpdateUserInfo {
            directory_visibility,
            im_via_email,
        } => {
            if user_info_scene() == "savefail" {
                return Some(vec![NetEvent::UserInfoUpdateFailed(user_info::Error::Refused("demo".into()))]);
            }
            let mut a = account();
            a.directory_visibility = directory_visibility.clone();
            if let Some(on) = im_via_email {
                a.im_via_email = *on;
            }
            log::info!("demo user info: saved {} (e-mail {:?})", a.directory_visibility, im_via_email);
            if let Ok(mut kept) = ACCOUNT.lock() {
                *kept = Some(a);
            }
            Some(Vec::new())
        }
        _ => None,
    }
}
