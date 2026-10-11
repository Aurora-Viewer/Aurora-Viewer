//! AURORA_DEMO_GLYPHS: residents whose group title, display name and chat
//! line use the characters residents really put there (mathematical "fancy"
//! letters, symbols, Georgian hearts, Japanese, Korean, Chinese, Arabic,
//! Hebrew, emoji…), and a prim with such a hover text. Everything the
//! embedded fonts lack must come from the system's fonts
//! (`ui::fonts::fallback`) instead of showing squares.

use super::{HANDLE, IDLE_ANIM, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use uuid::Uuid;

/// One resident of the scenario.
pub struct Sample {
    /// Group title, the small line above the name on the tag.
    pub title: &'static str,
    pub display_name: &'static str,
    /// What the resident says in local chat.
    pub chat: &'static str,
}

pub const SAMPLES: &[Sample] = &[
    // the characters of the report (#127): a title wrapped in Georgian "hearts"
    Sample {
        title: "ღAurora Friendღ",
        display_name: "ნინო ღ Նարե",
        chat: "Georgian ქართული ღ · Armenian Հայերեն",
    },
    Sample {
        title: "𝕍𝕀ℙ 𝔐𝔢𝔪𝔟𝔢𝔯",
        display_name: "𝓛𝓾𝓷𝓪 𝐖𝐨𝐥𝐟",
        chat: "Math letters: 𝐁𝐨𝐥𝐝 𝑰𝒕𝒂𝒍𝒊𝒄 𝒮𝒸𝓇𝒾𝓅𝓉 𝓢𝓬𝓻𝓲𝓹𝓽 𝔉𝔯𝔞𝔨𝔱𝔲𝔯 𝔻𝕠𝕦𝕓𝕝𝕖 𝖲𝖺𝗇𝗌 𝗕𝗼𝗹𝗱 𝙼𝚘𝚗𝚘",
    },
    Sample {
        title: "★彡 ♛ Queen ♛ 彡★",
        display_name: "꧁༺ Ⓝⓞⓥⓐ ༻꧂",
        chat: "Symbols: ░▒▓█ ╔═╗ → ⇒ ➳ ✞ ☠ ⚡ ⎝⎠ ◆ ⠁⠃ ①② ⅓ ™ ∞ ✿ ❀ ♪",
    },
    // already drawn by the embedded fonts: must not change
    Sample {
        title: "Русский · Ελληνικά",
        display_name: "Фиолетовый Волк",
        chat: "Привет, мир! Γειά σου κόσμε! Ça va ? Žluťoučký kůň",
    },
    Sample {
        title: "日本語 한국어 中文",
        display_name: "紫の狼ツ 보라늑대",
        chat: "こんにちは世界 · 안녕하세요 · 你好，世界 · 繁體中文 · ｆｕｌｌｗｉｄｔｈ",
    },
    Sample {
        title: "العربية · עברית",
        display_name: "ذئب بنفسجي",
        chat: "Right to left: مرحبا بالعالم · שלום עולם · ۞ ۩",
    },
    Sample {
        title: "ᗩᑌᖇOᖇᗩ ᏔᎾᏞᎰ",
        display_name: "ᴀᴜʀᴏʀᴀ ωσℓƒ ツ",
        chat: "Other scripts: नमस्ते ॐ · สวัสดี ๑ · ஐ · ༒ · ꋬ · ጀ · ᚱᚢᚾ · ᬅᬓ",
    },
    Sample {
        title: "🐺 Wolf Pack 🌙",
        display_name: "Luna ✨🦊",
        chat: "Emoji: 😀 🎉 ❤ 👍 🫠 🐺 · not composed yet: 🇫🇷 👍🏽 👩‍🚀",
    },
];

/// llSetText of the scenario's prim.
pub const HOVER_TEXT: &str = "llSetText ✿ 東京 ღ 𝓗𝓸𝓿𝓮𝓻\n░▒▓ → ★ ♛";

/// `AURORA_DEMO_GLYPHS=1`, or `2` for the chat lines in reverse order:
/// Some(reversed).
pub fn scenario() -> Option<bool> {
    std::env::var("AURORA_DEMO_GLYPHS").ok().map(|v| v.trim() == "2")
}

fn id(k: usize) -> Uuid {
    Uuid::from_u128(0xA0E0_61F0_0000_0000_0000_0000_0000_0000 | k as u128)
}

fn legacy_first(k: usize) -> String {
    format!("Glyph{}", k + 1)
}

/// The scenario's resident behind this id: (legacy first name, sample).
pub fn resident(avatar: &Uuid) -> Option<(String, &'static Sample)> {
    let k = (0..SAMPLES.len()).find(|k| id(*k) == *avatar)?;
    Some((legacy_first(k), &SAMPLES[k]))
}

/// The residents in two rows in front of the start position, facing it, each
/// saying its line (last resident first with `reversed`: the chat window
/// shows the last few lines); the prim with the hover text on the right.
pub fn events(reversed: bool) -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    // start position and view direction of the demo (see `events`)
    let (start, ahead) = (Vec3::new(132.0, 126.0, 0.0), Vec3::new(0.8, 0.4, 0.0).normalize());
    let right = Vec3::new(ahead.y, -ahead.x, 0.0);
    // meters to the right of the view axis: a gap in the middle for the
    // demo's own resident; the far row hovers so its tags sit a tier above
    const NEAR: [f32; 4] = [-5.0, -2.6, 2.6, 5.0];
    const FAR: [f32; 4] = [-5.6, -2.0, 2.0, 5.6];
    let mut objects = Vec::new();
    let mut events = Vec::new();
    let mut lines = Vec::new();
    for (k, sample) in SAMPLES.iter().enumerate() {
        let (far, slot) = (k % 2 == 1, (k / 2) % 4);
        let p = start + ahead * if far { 11.5 } else { 7.5 } + right * if far { FAR[slot] } else { NEAR[slot] };
        let mut a = prim(
            120_000 + k as u32,
            Vec3::new(p.x, p.y, floor_at(p.x, p.y) + 0.84 + if far { 1.8 } else { 0.0 }),
            Quat::from_rotation_z(ahead.y.atan2(ahead.x) + std::f32::consts::PI),
            Vec3::new(0.45, 0.6, 1.9),
            boxp,
            te([1.0; 4], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        a.pcode = LL_PCODE_LEGACY_AVATAR;
        a.full_id = id(k);
        a.name_values = format!(
            "FirstName STRING RW SV {}\nLastName STRING RW SV Resident\nTitle STRING RW SV {}",
            legacy_first(k),
            sample.title
        );
        objects.push(a);
        events.push(NetEvent::AvatarAnimations {
            avatar: id(k),
            anims: vec![(IDLE_ANIM, 1)],
            sources: Vec::new(),
        });
        lines.push(NetEvent::Chat(aurora_net::ChatMessage {
            from_name: format!("{} Resident", legacy_first(k)),
            source_id: id(k),
            owner_id: Uuid::nil(),
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: aurora_net::ChatType::Normal,
            position: Vec3::ZERO,
            message: sample.chat.into(),
        }));
    }
    if reversed {
        lines.reverse();
    }
    events.extend(lines);
    let p = start + ahead * 5.0 + right * 1.9;
    objects.push(prim(
        120_100,
        Vec3::new(p.x, p.y, floor_at(p.x, p.y) + 0.25),
        Quat::IDENTITY,
        Vec3::splat(0.5),
        boxp,
        te([0.55, 0.36, 0.96, 1.0], 1, false, 0.0),
        ExtraParams::default(),
        HOVER_TEXT,
    ));
    events.insert(0, NetEvent::ObjectUpdates { handle: HANDLE, objects });
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_resident_has_an_avatar_a_name_and_a_line() {
        let events = events(false);
        let Some(NetEvent::ObjectUpdates { objects, .. }) = events.first() else {
            panic!("objects first");
        };
        let avatars: Vec<_> = objects.iter().filter(|o| o.pcode == LL_PCODE_LEGACY_AVATAR).collect();
        assert_eq!(avatars.len(), SAMPLES.len());
        for (avatar, sample) in avatars.iter().zip(SAMPLES) {
            let (first, found) = resident(&avatar.full_id).expect("known resident");
            assert_eq!((found.title, found.display_name), (sample.title, sample.display_name));
            assert!(avatar.name_values.contains(&format!("FirstName STRING RW SV {first}\n")));
            assert!(avatar.name_values.ends_with(sample.title));
        }
        let lines = |reversed| -> Vec<String> {
            super::events(reversed)
                .into_iter()
                .filter_map(|e| match e {
                    NetEvent::Chat(c) => Some(c.message),
                    _ => None,
                })
                .collect()
        };
        let (forward, mut backward) = (lines(false), lines(true));
        assert_eq!(forward, SAMPLES.iter().map(|s| s.chat).collect::<Vec<_>>());
        backward.reverse();
        assert_eq!(forward, backward);
        assert!(resident(&super::super::DEMO_LOUP).is_none());
    }

    #[test]
    fn the_report_s_character_is_in_the_scenario() {
        // #127: a group title wrapped in U+10E6
        assert!(
            SAMPLES
                .iter()
                .any(|s| s.title.starts_with('\u{10E6}') && s.title.ends_with('\u{10E6}'))
        );
    }
}
