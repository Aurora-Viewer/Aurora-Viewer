//! The chat bar as a command line (Firestorm's « Commandes » preferences).
//!
//! Port of cmd_line_chat (indra/newview/chatbar_as_cmdline.cpp, Firestorm):
//! local chat whose first word is one of the configured command names is
//! not sent, it runs a viewer command instead (`gtp 128 128 30`, `dd 256`,
//! `calc 2+2`…). [`parse`] decides, from the text alone, whether the line is
//! a command and which one; the app runs it (`App::run_chat_command`).
//! Like Firestorm, a command with missing arguments is sometimes sent as
//! ordinary chat (`gtp 12`, `calc`): [`parse`] returns `None` for those.

pub mod calc;
pub mod dice;
mod scan;

use glam::Vec3;
use scan::Scanner;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The FSCmdLine* settings (defaults of Firestorm's settings.xml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatCommandSettings {
    /// FSCmdLine: use the chat bar as a command line.
    pub enabled: bool,
    pub calc: String,
    pub draw_distance: String,
    pub bandwidth: String,
    pub copy_cam: String,
    pub paste_cam: String,
    pub ao: String,
    pub clear_chat: String,
    pub media: String,
    pub music: String,
    pub rez_platform: String,
    /// FSCmdLinePlatformSize: platform width (m) when none is given.
    pub platform_size: f32,
    pub key_to_name: String,
    pub roll_dice: String,
    pub track_pos: String,
    pub pos: String,
    pub ground: String,
    pub height: String,
    /// FSCmdTeleportToCam.
    pub teleport_to_cam: String,
    pub offer_tp: String,
    pub tp2: String,
    pub teleport_home: String,
    pub map_to: String,
    /// FSCmdLineMapToKeepPos: `mapto` keeps the current local position.
    pub map_to_keep_pos: bool,
    /// FSCmdLineAnnounceToChannel / FSCmdLineAnnounceChannel: also whisper
    /// the command output on a script channel.
    pub announce_to_channel: bool,
    pub announce_channel: i32,
}

impl Default for ChatCommandSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            calc: "calc".into(),
            draw_distance: "dd".into(),
            bandwidth: "bw".into(),
            copy_cam: "cpcampos".into(),
            paste_cam: "pstcampos".into(),
            ao: "cao".into(),
            clear_chat: "clrchat".into(),
            media: "/media".into(),
            music: "/music".into(),
            rez_platform: "rezplat".into(),
            platform_size: 30.0,
            key_to_name: "key2name".into(),
            roll_dice: "rolld".into(),
            track_pos: "trkpos".into(),
            pos: "gtp".into(),
            ground: "flr".into(),
            height: "gth".into(),
            teleport_to_cam: "tp2cam".into(),
            offer_tp: "offertp".into(),
            tp2: "tp2".into(),
            teleport_home: "tph".into(),
            map_to: "mapto".into(),
            map_to_keep_pos: false,
            announce_to_channel: false,
            announce_channel: 362_395,
        }
    }
}

impl ChatCommandSettings {
    pub fn sanitize(&mut self) {
        self.platform_size = self.platform_size.clamp(5.0, 64.0);
    }
}

/// A number typed as `12` (absolute) or `+12` / `-12` (relative to the
/// current value): parseAbsoluteOrRelativeF32.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Amount {
    pub value: f32,
    pub relative: bool,
}

impl Amount {
    fn parse(token: &str) -> Option<Amount> {
        let value = leading_f32(token)?;
        Some(Amount {
            value,
            relative: token.starts_with(['+', '-']),
        })
    }

    pub fn apply(self, base: f32) -> f32 {
        if self.relative { base + self.value } else { self.value }
    }
}

/// `sscanf("%f")`: the float at the start of `s`, ignoring what follows.
fn leading_f32(s: &str) -> Option<f32> {
    Scanner::new(s).float()
}

/// A camera view `<x, y, z>|<fx, fy, fz>|roll` in region coordinates
/// (CmdlineCameraSpec).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraSpec {
    pub pos: Vec3,
    pub focus: Option<Vec3>,
    pub roll: Option<f32>,
}

/// parseCameraPositionString: the focus and the roll are optional.
pub fn parse_camera(params: &str) -> Option<CameraSpec> {
    let mut parts = params.splitn(3, '|');
    let pos = parse_bracket_vec3(parts.next()?)?;
    let focus = match parts.next() {
        Some(f) => Some(parse_bracket_vec3(f)?),
        None => None,
    };
    let roll = match parts.next() {
        Some(r) => Some(leading_f32(r.trim())?),
        None => None,
    };
    Some(CameraSpec { pos, focus, roll })
}

/// parseBracketVector3: `<x, y, z>` (commas or spaces).
fn parse_bracket_vec3(segment: &str) -> Option<Vec3> {
    let t = segment.trim();
    if t.len() < 5 {
        return None;
    }
    let inner = t.strip_prefix('<')?.strip_suffix('>')?.replace(',', " ");
    let mut s = Scanner::new(&inner);
    Some(Vec3::new(s.float()?, s.float()?, s.float()?))
}

/// formatCameraPositionString.
pub fn format_camera(pos: Vec3, focus: Vec3, roll: f32) -> String {
    format!(
        "<{:.3}, {:.3}, {:.3}>|<{:.3}, {:.3}, {:.3}>|{roll:.3}",
        pos.x, pos.y, pos.z, focus.x, focus.y, focus.z
    )
}

/// cmdline_partial_name2key: the first avatar (in radar order) whose user
/// name ("jane doe", "jane" for a Resident) contains `partial`, ignoring
/// case; a `.` in the typed name stands for a space.
pub fn partial_name_match<'a>(partial: &str, candidates: impl IntoIterator<Item = (Uuid, &'a str)>) -> Option<Uuid> {
    let partial = partial.to_lowercase().replace('.', " ");
    candidates
        .into_iter()
        .find(|(_, user_name)| user_name.to_lowercase().contains(&partial))
        .map(|(id, _)| id)
}

/// A recognised command, to be run by the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Lines to report in the local chat (calc, rolld, usage texts…).
    Report(Vec<String>),
    /// `gtp x y [z]`: teleport within the region (z = current height).
    TeleportLocal {
        x: f32,
        y: f32,
        z: Option<f32>,
    },
    /// `dd [+-]meters`.
    DrawDistance(Amount),
    TeleportToCamera,
    /// `/media url type`: play a media URL as the parcel media.
    Media {
        url: String,
        mime: String,
    },
    /// `/music url`: play a music stream.
    Music(String),
    /// A command Aurora can't do yet (no AO, no bandwidth setting, no hover
    /// height): said in the chat instead of sending the line.
    Unavailable(&'static str),
    /// `key2name uuid`: tell the avatar name once known.
    KeyToName(Uuid),
    /// `/touch uuid`, `/siton uuid`, `/standup`.
    Touch(Uuid),
    SitOn(Uuid),
    StandUp,
    /// `offertp uuid`.
    OfferTeleport(Uuid),
    /// `flr [offset]`: teleport to the ground (offset ≥ 0 above it).
    Ground {
        offset: f32,
    },
    /// `gth [+-]z`.
    Height(Amount),
    TeleportHome,
    /// `rezplat [width]`: a flat box under the avatar.
    RezPlatform(Option<f32>),
    /// `mapto region[|x y z]`: `pos` is `None` without `|` (then the current
    /// position or 128, 128, 0 per FSCmdLineMapToKeepPos).
    MapTo {
        region: String,
        pos: Option<(i32, i32, i32)>,
    },
    /// `tp2 name`: teleport to a nearby avatar found by partial name.
    TeleportToAvatar(String),
    /// `trkpos <x, y, z>` (region coordinates) or `trkpos name`.
    TrackPosition(Vec3),
    TrackAvatar(String),
    ClearChat,
    CopyCamera,
    PasteCamera(CameraSpec),
}

const AO_UNAVAILABLE: &str = "Aurora n'a pas encore d'animation overrider (AO).";
const BANDWIDTH_UNAVAILABLE: &str = "Aurora ne règle pas encore la bande passante maximale.";
const HOVER_UNAVAILABLE: &str = "Aurora ne règle pas encore la hauteur de survol de l'avatar.";
const CAMERA_PARSE_ERROR: &str =
    "Position de la caméra incorrecte. Attendu : <x, y, z> ou <x, y, z>|<fx, fy, fz>|roll (coordonnées de la région).";
const TRACK_USAGE: &str = "Suivre une position ou un avatar sur la carte du monde (utilisation : cmd <x, y, z> ou cmd nom).";

/// The command typed in `text` (local chat, already trimmed), or `None` when
/// the line must be sent as chat. `roll(n)` returns a random number in
/// `0..n` (calc's RAND and rolld).
pub fn parse(text: &str, s: &ChatCommandSettings, mut roll: impl FnMut(u32) -> u32) -> Option<Command> {
    if !s.enabled {
        return None;
    }
    let mut i = Scanner::new(text);
    let command = i.word()?;
    let is = |name: &str| command == name;
    // Text after "command " (the C++ `substr(command.length() + 1)`).
    let rest = || text.get(command.len() + 1..).filter(|r| !r.is_empty());

    if is(&s.pos) {
        let (x, y) = (i.float()?, i.float()?);
        return Some(Command::TeleportLocal { x, y, z: i.float() });
    }
    if is(&s.draw_distance) {
        return i.word().and_then(Amount::parse).map(Command::DrawDistance);
    }
    if is(&s.teleport_to_cam) {
        return Some(Command::TeleportToCamera);
    }
    if is(&s.media) {
        let (url, mime) = (i.word()?, i.word()?);
        return Some(Command::Media {
            url: url.into(),
            mime: mime.into(),
        });
    }
    if is(&s.music) {
        return i.word().map(|url| Command::Music(url.into()));
    }
    if is(&s.bandwidth) {
        return i.int().map(|_| Command::Unavailable(BANDWIDTH_UNAVAILABLE));
    }
    if is(&s.ao) {
        return Some(Command::Unavailable(AO_UNAVAILABLE));
    }
    if is(&s.key_to_name) {
        return Some(i.uuid().map_or(Command::Report(Vec::new()), Command::KeyToName));
    }
    match command {
        "/touch" => return Some(i.uuid().map_or(Command::Report(Vec::new()), Command::Touch)),
        "/siton" => return Some(i.uuid().map_or(Command::Report(Vec::new()), Command::SitOn)),
        "/standup" => return Some(Command::StandUp),
        "/zoffset_up" | "/zoffset_down" | "/zoffset_reset" | "/zoffset" => return Some(Command::Unavailable(HOVER_UNAVAILABLE)),
        _ => {}
    }
    if is(&s.offer_tp) {
        return Some(i.uuid().map_or(Command::Report(Vec::new()), Command::OfferTeleport));
    }
    if is(&s.ground) {
        // parseTerrainOffsetF32, then never below the ground.
        let offset = i.word().and_then(leading_f32).unwrap_or(0.0).max(0.0);
        return Some(Command::Ground { offset });
    }
    if is(&s.height) {
        let token = i.word()?;
        return Some(Amount::parse(token).map_or(Command::Report(Vec::new()), Command::Height));
    }
    if is(&s.teleport_home) {
        return Some(Command::TeleportHome);
    }
    if is(&s.rez_platform) {
        return Some(Command::RezPlatform(i.float()));
    }
    if is(&s.map_to) {
        let Some(arg) = rest() else {
            return Some(Command::Report(Vec::new()));
        };
        return Some(match arg.split_once('|') {
            Some((region, coords)) => {
                let mut c = Scanner::new(coords);
                let pos = match (c.int(), c.int(), c.int()) {
                    (Some(x), Some(y), Some(z)) => (x, y, z),
                    _ => (128, 128, 0),
                };
                Command::MapTo {
                    region: region.trim().into(),
                    pos: Some(pos),
                }
            }
            None => Command::MapTo {
                region: arg.into(),
                pos: None,
            },
        });
    }
    if is(&s.calc) {
        let expr = rest()?.to_uppercase();
        let (expanded, errors) = calc::expand_rand(&expr, &mut roll);
        let mut out: Vec<String> = errors
            .into_iter()
            .map(|r| {
                format!(
                    "'{r}' n'est pas une expression valable pour RAND(min,max). MAX doit être supérieur à MIN, \
                     les deux dans une fourchette de -10000 à 10000."
                )
            })
            .collect();
        out.push(match calc::eval(&expanded) {
            Some(v) => format!("{expr} = {}", calc::format_result(v)),
            None => "Échec du calcul".into(),
        });
        return Some(Command::Report(out));
    }
    if is(&s.tp2) {
        return Some(rest().map_or(Command::Report(Vec::new()), |name| Command::TeleportToAvatar(name.into())));
    }
    if is(&s.track_pos) {
        let arg = rest().map(str::trim).unwrap_or_default();
        return Some(if arg.is_empty() {
            Command::Report(vec![TRACK_USAGE.into()])
        } else if arg.starts_with('<') {
            parse_camera(arg).map_or_else(
                || {
                    Command::Report(vec![
                        "Position incorrecte. Utilisez le format de cpcampos : <x, y, z> ou <x, y, z>|<fx, fy, fz>|roll.".into(),
                    ])
                },
                |spec| Command::TrackPosition(spec.pos),
            )
        } else {
            Command::TrackAvatar(arg.into())
        });
    }
    if is(&s.clear_chat) {
        return Some(Command::ClearChat);
    }
    if is(&s.copy_cam) {
        return Some(Command::CopyCamera);
    }
    if is(&s.paste_cam) {
        let spec = rest().map(str::trim).and_then(parse_camera);
        return Some(spec.map_or_else(|| Command::Report(vec![CAMERA_PARSE_ERROR.into()]), Command::PasteCamera));
    }
    if is(&s.roll_dice) {
        return Some(Command::Report(dice::roll_dice(command, rest().unwrap_or_default(), roll)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(text: &str) -> Option<Command> {
        parse(text, &ChatCommandSettings::default(), |_| 0)
    }

    #[test]
    fn plain_chat_is_not_a_command() {
        assert_eq!(cmd("hello there"), None);
        assert_eq!(cmd("ggtp 1 2"), None);
        let off = ChatCommandSettings {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(parse("gtp 1 2 3", &off, |_| 0), None);
    }

    #[test]
    fn teleports() {
        assert_eq!(
            cmd("gtp 128 64 30"),
            Some(Command::TeleportLocal {
                x: 128.0,
                y: 64.0,
                z: Some(30.0)
            })
        );
        assert_eq!(
            cmd("gtp 128 64"),
            Some(Command::TeleportLocal {
                x: 128.0,
                y: 64.0,
                z: None
            })
        );
        // Missing coordinates: sent as chat, like Firestorm.
        assert_eq!(cmd("gtp 128"), None);
        assert_eq!(
            cmd("gth +10"),
            Some(Command::Height(Amount {
                value: 10.0,
                relative: true
            }))
        );
        assert_eq!(cmd("gth"), None);
        assert_eq!(cmd("flr"), Some(Command::Ground { offset: 0.0 }));
        assert_eq!(cmd("flr -5"), Some(Command::Ground { offset: 0.0 }));
        assert_eq!(cmd("flr 2.5"), Some(Command::Ground { offset: 2.5 }));
        assert_eq!(cmd("tph"), Some(Command::TeleportHome));
        assert_eq!(
            cmd("mapto Da Boom|10 20 30"),
            Some(Command::MapTo {
                region: "Da Boom".into(),
                pos: Some((10, 20, 30))
            })
        );
        assert_eq!(
            cmd("mapto Da Boom|x"),
            Some(Command::MapTo {
                region: "Da Boom".into(),
                pos: Some((128, 128, 0))
            })
        );
        assert_eq!(
            cmd("mapto Ahern"),
            Some(Command::MapTo {
                region: "Ahern".into(),
                pos: None
            })
        );
        assert_eq!(cmd("tp2 jane"), Some(Command::TeleportToAvatar("jane".into())));
    }

    #[test]
    fn settings_commands() {
        assert_eq!(
            cmd("dd 256"),
            Some(Command::DrawDistance(Amount {
                value: 256.0,
                relative: false
            }))
        );
        assert_eq!(
            cmd("dd -32"),
            Some(Command::DrawDistance(Amount {
                value: -32.0,
                relative: true
            }))
        );
        assert_eq!(
            Amount {
                value: -32.0,
                relative: true
            }
            .apply(128.0),
            96.0
        );
        assert_eq!(cmd("dd far"), None);
        assert_eq!(cmd("bw 500"), Some(Command::Unavailable(BANDWIDTH_UNAVAILABLE)));
        assert_eq!(cmd("bw"), None);
        assert_eq!(cmd("cao on"), Some(Command::Unavailable(AO_UNAVAILABLE)));
        assert_eq!(cmd("rezplat"), Some(Command::RezPlatform(None)));
        assert_eq!(cmd("rezplat 12"), Some(Command::RezPlatform(Some(12.0))));
        assert_eq!(
            cmd("/media http://x.test/v.mp4 video/mp4"),
            Some(Command::Media {
                url: "http://x.test/v.mp4".into(),
                mime: "video/mp4".into()
            })
        );
        assert_eq!(cmd("/media http://x.test/v.mp4"), None);
        assert_eq!(cmd("/music http://x.test:8000"), Some(Command::Music("http://x.test:8000".into())));
    }

    #[test]
    fn renamed_command() {
        let s = ChatCommandSettings {
            pos: "goto".into(),
            ..Default::default()
        };
        assert_eq!(parse("gtp 1 2", &s, |_| 0), None);
        assert!(parse("goto 1 2", &s, |_| 0).is_some());
    }

    #[test]
    fn calc_reports() {
        assert_eq!(cmd("calc sin(2+2)"), Some(Command::Report(vec!["SIN(2+2) = 0.0697565".into()])));
        assert_eq!(cmd("calc 2+"), Some(Command::Report(vec!["Échec du calcul".into()])));
        assert_eq!(cmd("calc rand(1,6)*2"), Some(Command::Report(vec!["RAND(1,6)*2 = 2".into()])));
        assert_eq!(cmd("calc"), None);
    }

    #[test]
    fn ids() {
        let id = Uuid::from_u128(0x1234);
        assert_eq!(cmd(&format!("key2name {id}")), Some(Command::KeyToName(id)));
        assert_eq!(cmd("key2name nope"), Some(Command::Report(Vec::new())));
        assert_eq!(cmd(&format!("offertp {id}")), Some(Command::OfferTeleport(id)));
        assert_eq!(cmd(&format!("/siton {id}")), Some(Command::SitOn(id)));
    }

    #[test]
    fn camera_strings() {
        let spec = parse_camera("<1, 2, 3>|<4 5 6>|0.5").expect("valid");
        assert_eq!(spec.pos, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(spec.focus, Some(Vec3::new(4.0, 5.0, 6.0)));
        assert_eq!(spec.roll, Some(0.5));
        assert_eq!(parse_camera("<1, 2, 3>").map(|s| s.focus), Some(None));
        assert_eq!(parse_camera("1, 2, 3"), None);
        assert_eq!(parse_camera("<1, 2>"), None);
        assert_eq!(
            format_camera(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0), 0.0),
            "<1.000, 2.000, 3.000>|<4.000, 5.000, 6.000>|0.000"
        );
        assert_eq!(
            cmd("trkpos <10, 20, 30>"),
            Some(Command::TrackPosition(Vec3::new(10.0, 20.0, 30.0)))
        );
        assert_eq!(cmd("trkpos Jane"), Some(Command::TrackAvatar("Jane".into())));
        assert!(matches!(cmd("pstcampos nope"), Some(Command::Report(_))));
    }

    #[test]
    fn partial_names() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let list = [(a, "jane doe"), (b, "bob")];
        assert_eq!(partial_name_match("Jane.D", list), Some(a));
        assert_eq!(partial_name_match("BO", list), Some(b));
        assert_eq!(partial_name_match("carl", list), None);
    }
}
