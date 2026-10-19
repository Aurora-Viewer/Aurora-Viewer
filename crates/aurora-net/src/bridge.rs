//! Private HTTPS commands to Aurora's worn LSL bridge. As FSLSLBridge's
//! viewerToLSL, the request runs off the frame thread; endpoint credentials
//! never enter a log, cache or saved preference.
use std::{fmt, sync::OnceLock, time::Duration};
use uuid::Uuid;

pub mod query;

#[derive(Clone)]
pub struct Endpoint {
    pub url: String,
    pub token: Uuid,
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Endpoint(<private>)")
    }
}

/// Only Second Life simulator HTTP-in URLs are accepted. In particular a
/// chat message cannot turn this client into a general HTTP requester.
pub fn valid_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else { return false };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.ends_with(".lindenlab.com") || host.ends_with(".secondlife.com") || host.ends_with(".secondlife.io"))
        && url
            .path()
            .strip_prefix("/cap/")
            .is_some_and(|key| Uuid::parse_str(key.strip_suffix('/').unwrap_or(key)).is_ok())
}

#[derive(Debug, Clone)]
pub struct Command {
    pub endpoint: Endpoint,
    pub sequence: u32,
    pub enabled: bool,
    pub relock: bool,
    pub motion: Motion,
}

/// Motor configuration; LSL derives direction from its current control events.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Motion {
    /// 0 released, 1 run, 2 flight, 3 surface swim, 4 water exit,
    /// 5 bounded physical passage, 6 underwater swim, 7 dive, 8 surface,
    /// 9 bounded shore jump, 10 water takeoff, 11 observed dive jump,
    /// 12 observed cannonball, 13 water walking. Observer modes leave gravity alone.
    pub mode: u8,
    /// Motors: [speed m/s (run: multiplier 1..5), avatar height m, auxiliary].
    /// Auxiliary: swim float-height offset m (7/8/10: water-relative goal),
    /// exit climb m/s, otherwise zero. Passage/shore jump: region-space
    /// displacement, never a teleport destination.
    pub parameters: [f32; 3],
    /// Nonzero idempotency key for one physical passage/jump; zero for motors.
    pub action: u32,
}
impl Motion {
    pub const fn motor(mode: u8, speed: f32, height: f32) -> Self {
        Self {
            mode,
            parameters: [speed, height, 0.0],
            action: 0,
        }
    }
    pub fn passage(offset: [f32; 3]) -> Option<Self> {
        let motion = Self {
            mode: 5,
            parameters: offset,
            action: ((Uuid::new_v4().as_u128() as u32) & i32::MAX as u32).max(1),
        };
        motion.valid().then_some(motion)
    }
    pub fn shore_jump(offset: [f32; 3]) -> Option<Self> {
        let motion = Self {
            mode: 9,
            parameters: offset,
            action: ((Uuid::new_v4().as_u128() as u32) & i32::MAX as u32).max(1),
        };
        motion.valid().then_some(motion)
    }
    pub fn water_jump(cannonball: bool, height: f32, action: u32) -> Option<Self> {
        let motion = Self {
            mode: if cannonball { 12 } else { 11 },
            parameters: [0.0, height, 0.0],
            action,
        };
        motion.valid().then_some(motion)
    }
    pub fn valid(self) -> bool {
        let [speed, height, climb] = self.parameters;
        if !self.parameters.iter().all(|v| v.is_finite()) {
            return false;
        }
        match self.mode {
            0 => self.parameters == [0.0; 3] && self.action == 0,
            5 => {
                let distance2 = self.parameters.iter().map(|v| v * v).sum::<f32>();
                (0.25..=64.0).contains(&distance2) && climb.abs() <= 1.5 && self.action > 0 && self.action <= i32::MAX as u32
            }
            9 => {
                let distance2 = self.parameters.iter().map(|v| v * v).sum::<f32>();
                (0.25..=16.0).contains(&distance2) && (0.1..=3.5).contains(&climb) && self.action > 0 && self.action <= i32::MAX as u32
            }
            11 | 12 => speed == 0.0 && climb == 0.0 && (0.6..=3.0).contains(&height) && (1..=i32::MAX as u32).contains(&self.action),
            1 | 2 | 3 | 4 | 6 | 7 | 8 | 10 | 13 => {
                let range = match self.mode {
                    1 => 1.0..=5.0,
                    // x5 downward flight is 114.35 m/s; leave rounding headroom.
                    2 => 1.0..=120.0,
                    _ => 0.5..=8.0,
                };
                self.action == 0
                    && range.contains(&speed)
                    && (0.6..=3.0).contains(&height)
                    && if self.mode == 4 {
                        (0.0..=4.5).contains(&climb)
                    } else if self.mode == 7 {
                        (-4.0..=0.5).contains(&climb)
                    } else if self.mode == 8 {
                        (-2.0..=0.5).contains(&climb)
                    } else if self.mode == 10 {
                        (0.4..=3.0).contains(&climb)
                    } else if matches!(self.mode, 3 | 6) {
                        (-0.5..=0.5).contains(&climb)
                    } else {
                        climb == 0.0
                    }
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MotorReport {
    pub mode: u8,
    /// 0 idle, 1 no controls, 2 airborne, 3 locked, 4 target reached,
    /// 5 force requested, 6 invalid mass, 7 watchdog, 8 vertical input, 9 seated,
    /// 10 gradual target ramp, 11 thrust limit reached below target,
    /// 12 water jump armed, 13 airborne, 14 water contact observed.
    pub run_state: u8,
    pub speed_cm: u32,
    pub target_cm: u32,
    pub pushes: u32,
    pub regulation: Option<RunRegulation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunRegulation {
    /// Requested force divided by MKS mass, in thousandths, not measured acceleration.
    pub power_milli: u32,
    pub ramp_cm: u32,
    pub restarts: u32,
}

/// Simulator observations for one physical passage, independent of command
/// delivery and motor ownership. No position, object identity or endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassageReport {
    pub action: u32,
    /// 1 accepted, 2 motor started, 3 reached, 4 not reached before expiry,
    /// 5 cancelled, 6 held by Movelock, 7 invalid mass.
    pub state: u8,
    pub progress_cm: u32,
    pub remaining_cm: u32,
    /// Highest sampled forward velocity, not a physics-frame maximum.
    pub speed_cm: u32,
    /// Legacy v1.15 impulse requests; the remote-anchor motor reports zero.
    pub pushes: u8,
}
impl PassageReport {
    pub fn finished(self) -> bool {
        self.state >= 3
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ack {
    pub sequence: u32,
    pub enabled: bool,
    pub locked: bool,
    pub moving: bool,
    pub seated: bool,
    pub motor: Option<MotorReport>,
    pub passage: Option<PassageReport>,
}

impl Ack {
    pub fn parse(value: &str) -> Option<Self> {
        let fields: Vec<_> = value.trim().split('|').collect();
        if !matches!(fields.len(), 5 | 10 | 13 | 19) {
            return None;
        }
        let flag = |text| match text {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
        Some(Self {
            sequence: fields[0].parse::<u32>().ok().filter(|n| *n > 0 && *n <= i32::MAX as u32)?,
            enabled: flag(fields[1])?,
            locked: flag(fields[2])?,
            moving: flag(fields[3])?,
            seated: flag(fields[4])?,
            motor: if fields.len() >= 10 {
                Some(MotorReport {
                    mode: fields[5].parse::<u8>().ok().filter(|mode| *mode <= 13)?,
                    run_state: fields[6].parse::<u8>().ok().filter(|state| *state <= 14)?,
                    speed_cm: fields[7].parse::<u32>().ok().filter(|speed| *speed <= 1_000_000)?,
                    target_cm: fields[8].parse::<u32>().ok().filter(|target| *target <= 2_565)?,
                    pushes: fields[9].parse::<u32>().ok().filter(|count| *count <= i32::MAX as u32)?,
                    regulation: if fields.len() >= 13 {
                        Some(RunRegulation {
                            power_milli: fields[10].parse::<u32>().ok().filter(|power| *power <= 400_000)?,
                            ramp_cm: fields[11].parse::<u32>().ok().filter(|speed| *speed <= 2_565)?,
                            restarts: fields[12].parse::<u32>().ok().filter(|count| *count <= i32::MAX as u32)?,
                        })
                    } else {
                        None
                    },
                })
            } else {
                None
            },
            passage: if fields.len() == 19 {
                let action = fields[13].parse::<u32>().ok().filter(|n| *n <= i32::MAX as u32)?;
                let state = fields[14].parse::<u8>().ok().filter(|state| *state <= 7)?;
                let distance = |field: &str| field.parse::<u32>().ok().filter(|n| *n <= 1_000_000);
                let progress_cm = distance(fields[15])?;
                let remaining_cm = distance(fields[16])?;
                let speed_cm = distance(fields[17])?;
                let pushes = fields[18].parse::<u8>().ok().filter(|n| *n <= 12)?;
                if action == 0 {
                    if state != 0 || progress_cm != 0 || remaining_cm != 0 || speed_cm != 0 || pushes != 0 {
                        return None;
                    }
                    None
                } else {
                    if state == 0 {
                        return None;
                    }
                    Some(PassageReport {
                        action,
                        state,
                        progress_cm,
                        remaining_cm,
                        speed_cm,
                        pushes,
                    })
                }
            } else {
                None
            },
        })
    }
}

fn transport_failure(error: reqwest::Error) -> String {
    // A reqwest error can contain the private URL. Only classify its flags;
    // never format it, its source chain or the HTTP response body.
    let kind = if error.is_timeout() { "timeout" } else { "connection" };
    log::warn!("Aurora bridge: HTTP {kind} failure");
    if error.is_timeout() {
        aurora_i18n::tr!("bridge-request-timeout").into()
    } else {
        aurora_i18n::tr!("bridge-request-connection").into()
    }
}

fn decode_ack(bytes: &[u8], sequence: u32) -> Result<Ack, String> {
    let ack = std::str::from_utf8(bytes)
        .ok()
        .and_then(Ack::parse)
        .ok_or_else(|| aurora_i18n::tr!("bridge-request-response").to_owned())?;
    if ack.sequence != sequence {
        return Err(aurora_i18n::tr!("bridge-request-sequence").into());
    }
    Ok(ack)
}

fn http() -> Result<&'static reqwest::Client, String> {
    // No redirect may forward the private token to another endpoint.
    static HTTP: OnceLock<Result<reqwest::Client, ()>> = OnceLock::new();
    let http = HTTP
        .get_or_init(|| {
            // Same simulator trust as NetClient::new / Firestorm llcorehttp:
            // Linden's CA is required, and simulator certificates need not
            // match the individual simhost name. Keep certificate-chain
            // verification and forbid redirects carrying the bridge token.
            let mut roots: Vec<_> = webpki_root_certs::TLS_SERVER_ROOT_CERTS
                .iter()
                .filter_map(|cert| reqwest::Certificate::from_der(cert.as_ref()).ok())
                .collect();
            roots.extend(reqwest::Certificate::from_pem_bundle(include_bytes!("../lindenlab.pem")).map_err(|_| ())?);
            reqwest::Client::builder()
                .user_agent(aurora_version::user_agent())
                .redirect(reqwest::redirect::Policy::none())
                .tls_certs_only(roots)
                .tls_danger_accept_invalid_hostnames(true)
                .timeout(Duration::from_secs(8))
                .build()
                .map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| aurora_i18n::tr!("bridge-request-failed").to_owned())?;
    Ok(http)
}

pub async fn send(command: Command) -> Result<Ack, String> {
    let failed = || aurora_i18n::tr!("bridge-request-failed").to_owned();
    if !valid_url(&command.endpoint.url)
        || command.endpoint.token.is_nil()
        || command.sequence == 0
        || command.sequence > i32::MAX as u32
        || !command.motion.valid()
    {
        return Err(failed());
    }
    let http = http()?;
    let mut response = http
        .post(&command.endpoint.url)
        .timeout(Duration::from_secs(if command.motion.mode == 0 { 8 } else { 2 }))
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(format!(
            "lock|{}|{}|{}|{}|{}|{:.2}|{:.2}|{:.2}|{}",
            command.endpoint.token,
            command.sequence,
            u8::from(command.enabled),
            u8::from(command.relock),
            command.motion.mode,
            command.motion.parameters[0],
            command.motion.parameters[1],
            command.motion.parameters[2],
            command.motion.action
        ))
        .send()
        .await
        .map_err(transport_failure)?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        log::warn!("Aurora bridge: HTTP status {status}");
        return Err(aurora_i18n::tr!("bridge-request-http", status = status.to_string()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_failure)? {
        if bytes.len() + chunk.len() > 256 {
            return Err(aurora_i18n::tr!("bridge-request-response").into());
        }
        bytes.extend_from_slice(&chunk);
    }
    decode_ack(&bytes, command.sequence)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn water_jump_observers_are_bounded_and_cannot_carry_thrust() {
        for cannonball in [false, true] {
            let motion = Motion::water_jump(cannonball, 1.8, 37).expect("observer");
            assert!(motion.valid());
            assert_eq!(motion.parameters, [0.0, 1.8, 0.0]);
            for action in [0, u32::MAX] {
                assert!(Motion::water_jump(cannonball, 1.8, action).is_none());
            }
            for height in [0.0, 3.1, f32::NAN] {
                assert!(Motion::water_jump(cannonball, height, 37).is_none());
            }
            for axis in [0, 2] {
                let mut invalid = motion;
                invalid.parameters[axis] = 1.0;
                assert!(!invalid.valid());
            }
            for state in [12, 13, 14] {
                let ack = Ack::parse(&format!("42|0|0|1|0|{}|{state}|400|0|0", motion.mode)).expect("observer feedback");
                assert_eq!(ack.motor.unwrap().run_state, state);
            }
        }
    }

    #[test]
    fn command_replies_distinguish_malformed_state_from_a_wrong_sequence() {
        assert_eq!(decode_ack(b"42|1|0|1|0", 42).expect("current command").sequence, 42);
        for bytes in [b"broken".as_slice(), b"42|1|0".as_slice(), b"\xff".as_slice()] {
            assert_eq!(
                decode_ack(bytes, 42).expect_err("malformed"),
                aurora_i18n::tr!("bridge-request-response")
            );
        }
        assert_eq!(
            decode_ack(b"41|1|0|1|0", 42).expect_err("old reply"),
            aurora_i18n::tr!("bridge-request-sequence")
        );
    }

    #[test]
    fn transport_diagnostics_never_include_the_private_url_from_reqwest() {
        let error = reqwest::Client::new()
            .get("not a URL")
            .build()
            .expect_err("invalid builder")
            .with_url(reqwest::Url::parse("https://private.example/synthetic-private-path").expect("synthetic URL"));
        let message = transport_failure(error);
        assert_eq!(message, aurora_i18n::tr!("bridge-request-connection"));
        assert!(!message.contains("private.example") && !message.contains("synthetic-private-path"));
    }

    #[test]
    fn bridge_endpoint_and_debug_reject_disclosure_and_foreign_urls() {
        let path = "/cap/11111111-1111-4111-8111-111111111111";
        assert!(valid_url(&format!("https://sim.example.agni.lindenlab.com:12043{path}")));
        for host in ["simhost-example.agni.secondlife.io", "simhost-example.aditi.secondlife.io"] {
            assert!(valid_url(&format!("https://{host}:12043{path}")));
            assert!(valid_url(&format!("https://{host}:12043{path}/")));
        }
        for host in [
            "http://sim.agni.lindenlab.com",
            "https://localhost",
            "https://127.0.0.1",
            "https://lindenlab.com.evil.example",
            "https://secondlife.io.evil.example",
            "https://agni-secondlife.io",
            "https://secondlife.io",
            "https://user@simhost-example.agni.secondlife.io",
            "https://user@sim.agni.lindenlab.com",
        ] {
            assert!(!valid_url(&format!("{host}{path}")));
        }
        assert!(!valid_url("https://sim.agni.lindenlab.com/cap/x"));
        for tail in ["/extra", "//", "?query", "#fragment"] {
            assert!(!valid_url(&format!("https://simhost-example.agni.secondlife.io:12043{path}{tail}")));
        }
        let endpoint = Endpoint {
            url: format!("https://sim.agni.lindenlab.com{path}"),
            token: Uuid::new_v4(),
        };
        let debug = format!("{endpoint:?}");
        assert!(!debug.contains(&endpoint.url) && !debug.contains(&endpoint.token.to_string()));
    }
    #[test]
    fn acknowledgements_require_all_fields_and_a_valid_sequence() {
        let ack = Ack::parse("42|1|0|1|0").expect("moving reply");
        assert!(ack.enabled && ack.moving && !ack.locked);
        for bad in ["0|1|1|0|0", "1|1|1|0", "1|2|1|0|0", "1|1|1|0|0|extra", "2147483648|1|1|0|0"] {
            assert!(Ack::parse(bad).is_none());
        }
    }
    #[test]
    fn passage_feedback_distinguishes_motor_start_from_actual_progress_and_completion() {
        let base = "42|0|0|0|0|0|0|0|0|0|0|0|0";
        assert_eq!(Ack::parse(&format!("{base}|0|0|0|0|0|0")).expect("idle").passage, None);
        for state in 1..=7 {
            let report = Ack::parse(&format!("{base}|123|{state}|10|140|50|2"))
                .expect("passage status")
                .passage
                .expect("one action");
            assert_eq!(report.action, 123);
            assert_eq!((report.progress_cm, report.remaining_cm, report.speed_cm), (10, 140, 50));
            assert_eq!(report.finished(), state >= 3);
            assert_eq!(report.pushes, 2);
        }
        for suffix in [
            "0|2|0|0|0",
            "123|0|0|0|0",
            "0|0|1|0|0",
            "0|0|0|1|0",
            "0|0|0|0|1",
            "2147483648|2|0|0|0",
            "123|8|0|0|0",
            "123|2|-1|0|0",
            "123|2|0|NaN|0",
            "123|2|1000001|0|0",
            "123|2|0|1000001|0",
            "123|2|0|0|1000001",
            "123|2|0|0",
        ] {
            assert!(Ack::parse(&format!("{base}|{suffix}|0")).is_none(), "{suffix}");
        }
        assert!(Ack::parse(&format!("{base}|123|2|10|140|50|13")).is_none());
        assert!(Ack::parse(&format!("{base}|0|0|0|0|0|1")).is_none());
    }
    #[test]
    fn motor_settings_and_physical_passage_have_distinct_bounded_payloads() {
        for mode in [2, 3, 6, 13] {
            assert!(Motion::motor(mode, 8.0, 1.7).valid());
        }
        for factor in [1.0, 2.0, 5.0] {
            assert!(Motion::motor(1, factor, 1.7).valid());
        }
        for speed in [16.0 * 5.0, 22.87 * 5.0] {
            assert!(Motion::motor(2, speed, 1.7).valid());
        }
        for speed in [0.9, 120.1, f32::NAN, f32::INFINITY] {
            assert!(!Motion::motor(2, speed, 1.7).valid());
        }
        for mode in [3, 6] {
            for offset in [-0.5, 0.0, 0.5] {
                let mut motion = Motion::motor(mode, 2.0, 1.7);
                motion.parameters[2] = offset;
                assert!(motion.valid());
            }
            for offset in [-0.51, 0.51, f32::NAN] {
                let mut motion = Motion::motor(mode, 2.0, 1.7);
                motion.parameters[2] = offset;
                assert!(!motion.valid());
            }
        }
        assert!(Motion::passage([3.0, 0.0, 0.2]).is_some());
        for offset in [[0.0; 3], [8.1, 0.0, 0.0], [1.0, 0.0, 2.0], [f32::NAN, 0.0, 0.0]] {
            assert!(Motion::passage(offset).is_none());
        }
        for mode in [7, 8] {
            let mut motion = Motion::motor(mode, 5.0, 1.7);
            for offset in [-1.25, -0.45, 0.25] {
                motion.parameters[2] = offset;
                assert!(motion.valid());
            }
            motion.parameters[2] = 0.51;
            assert!(!motion.valid());
        }
        let mut takeoff = Motion::motor(10, 5.0, 1.7);
        takeoff.parameters[2] = 1.0;
        assert!(takeoff.valid());
        let mut deep_dive = Motion::motor(7, 5.0, 3.0);
        deep_dive.parameters[2] = -2.6;
        assert!(deep_dive.valid());
        deep_dive.mode = 8;
        assert!(!deep_dive.valid(), "the deeper allowance only belongs to descent");
        for speed in [0.49, 8.01, f32::NAN] {
            assert!(!Motion::motor(13, speed, 1.7).valid());
        }
        assert!(Motion::shore_jump([1.0, 0.0, 2.0]).is_some());
        assert!(Motion::shore_jump([4.1, 0.0, 0.5]).is_none());
        assert!(Motion::shore_jump([1.0, 0.0, 3.6]).is_none());
        for motion in [
            Motion::motor(11, 2.0, 1.7),
            Motion::motor(1, f32::NAN, 1.7),
            Motion::motor(1, 0.9, 1.7),
            Motion::motor(1, 5.1, 1.7),
            Motion::motor(2, 121.0, 1.7),
            Motion::motor(3, 9.0, 1.7),
        ] {
            assert!(!motion.valid());
        }
    }

    #[test]
    fn motor_feedback_distinguishes_received_settings_from_actual_script_thrust_and_speed() {
        let received = Ack::parse("42|0|0|1|0|1|5|540|2565|17").expect("motor feedback");
        assert_eq!(
            received.motor,
            Some(MotorReport {
                mode: 1,
                run_state: 5,
                speed_cm: 540,
                target_cm: 2565,
                pushes: 17,
                regulation: None,
            })
        );
        assert_eq!(Ack::parse("42|0|0|1|0").expect("legacy status").motor, None);
        let ramping = Ack::parse("42|0|0|1|0|1|10|610|1026|17").expect("gradual acceleration feedback");
        assert_eq!(ramping.motor.map(|report| report.run_state), Some(10));
        for invalid in [
            "42|0|0|1|0|1|5|NaN|2565|17",
            "42|0|0|1|0|1|5|-1|2565|17",
            "42|0|0|1|0|14|5|540|2565|17",
            "42|0|0|1|0|1|15|540|2565|17",
            "42|0|0|1|0|1|5|540|2566|17",
            "42|0|0|1|0|1|5|540|2565|2147483648",
            "42|0|0|1|0|1|5|540|2565",
        ] {
            assert!(Ack::parse(invalid).is_none());
        }
    }

    #[test]
    fn run_regulation_feedback_bounds_force_ramp_and_restart_counters() {
        let report = Ack::parse("42|0|0|1|0|1|11|700|2565|17|400000|2565|3")
            .expect("saturated motor feedback")
            .motor
            .expect("motor report");
        assert_eq!(report.run_state, 11);
        assert_eq!(
            report.regulation,
            Some(RunRegulation {
                power_milli: 400_000,
                ramp_cm: 2_565,
                restarts: 3
            })
        );
        for invalid in [
            "42|0|0|1|0|1|5|700|2565|17|400001|2565|3",
            "42|0|0|1|0|1|5|700|2565|17|NaN|2565|3",
            "42|0|0|1|0|1|5|700|2565|17|0|2566|3",
            "42|0|0|1|0|1|5|700|2565|17|0|2565|-1",
            "42|0|0|1|0|1|5|700|2565|17|0|2565|2147483648",
            "42|0|0|1|0|1|5|700|2565|17|0|2565",
        ] {
            assert!(Ack::parse(invalid).is_none());
        }
    }
}
