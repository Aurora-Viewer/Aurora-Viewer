//! Aurora Weather carried inside a sky's own settings, past every name.
//!
//! The server keeps no name of a day's key frames (LLSettingsDay::getSettings,
//! indra/llinventory/llsettingsdaycycle.cpp, clones them without
//! SETTING_NAME; its validation drops unknown keys), and an inline day's own
//! name is cut to 32 bytes. It does keep the sky's numbers to the bit as
//! 32-bit floats (measured on the grid, 2026-11-13: the low 12 bits of the
//! fields below came back untouched, inline and in a region's day). So the
//! sky's code is written there too: 12 bits in the lowest bits of each of
//! five fields, a change of at most a tenth of a percent of the value,
//! nothing anyone sees.
//!
//! The 60 bits are the code's data (`SkyWeather::bits`: 32, or 45 with an
//! aurora) and a 15-bit check of its own (salted apart from the names'): a
//! sky that never had a mark reads as weather about once in 65,000 (its
//! version must match too). Inside Aurora a sky's name stays the one place
//! its weather is set: a mark read without a code on the name puts the code
//! back on it (`name_from_mark`), and whatever Aurora sends or saves is
//! marked from its name, or cleared without one (`sync_to_name`).
//! Another viewer moving one of these five sliders wipes the mark (that sky
//! is plain there on), which only Aurora writes again.

use aurora_llsd::Llsd;

use super::region::SkyWeather;

/// The fields that may carry it, in order: those the server kept to the
/// bit (dome radius, star and moon brightness did not, every time).
const FIELDS: [&str; 9] = [
    "cloud_scale",
    "cloud_shadow",
    "dome_offset",
    "gamma",
    "max_y",
    "moon_scale",
    "sun_arc_radians",
    "sun_scale",
    "reflection_probe_ambiance",
];
/// How many fields take a part, and how many bits each.
const USED: usize = 5;
const PART: u32 = 12;
const PART_MASK: u32 = (1 << PART) - 1;
/// The data: `SkyWeather::bits`, padded to 45 bits; then the check.
const DATA: u32 = 45;
const CHECK: u32 = USED as u32 * PART - DATA;
const SALT: u64 = 0x5EED_A0B0_7A11_0C0D;
/// A float's magnitude below this (zero, the subnormals) takes no part: the
/// next one does (and a mark keeps a carrier's at least this).
const SMALLEST: u32 = 0x0080_0000 + (2 << PART);

fn check(data: u64) -> u64 {
    (data ^ SALT).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - CHECK)
}

/// The fields of `sky` that carry the mark, if it has enough of them.
fn carriers(sky: &Llsd) -> Option<[&'static str; USED]> {
    let mut out = [""; USED];
    let mut n = 0;
    for field in FIELDS {
        if let Llsd::Real(v) = sky[field]
            && v.is_finite()
            && (v as f32).to_bits() & 0x7FFF_FFFF >= SMALLEST
        {
            out[n] = field;
            n += 1;
            if n == USED {
                return Some(out);
            }
        }
    }
    None
}

fn real(sky: &Llsd, field: &str) -> f32 {
    match sky[field] {
        Llsd::Real(v) => v as f32,
        _ => 0.0,
    }
}

/// The weather marked in `sky`'s settings, if it carries a mark.
pub fn read(sky: &Llsd) -> Option<SkyWeather> {
    if !sky.is_map() || (sky.has("type") && sky["type"].as_str() != "sky") {
        return None;
    }
    let fields = carriers(sky)?;
    let mut all = 0u64;
    for (i, field) in fields.iter().enumerate() {
        all |= u64::from(real(sky, field).to_bits() & PART_MASK) << (i as u32 * PART);
    }
    let data = all & ((1 << DATA) - 1);
    if all >> DATA != check(data) {
        return None;
    }
    SkyWeather::from_bits(data)
}

/// `v` with `part` in its low bits, its magnitude never more than `v`'s
/// (a value at its maximum stays within it; one at a minimum other than
/// zero may be clamped back up by the server, and its mark lost).
fn marked(v: f32, part: u32) -> f32 {
    let bits = v.to_bits();
    let sign = bits & 0x8000_0000;
    let mag = bits & 0x7FFF_FFFF;
    let mut base = mag & !PART_MASK;
    if base | part > mag {
        base -= 1 << PART;
    }
    f32::from_bits(sign | base | part)
}

/// Mark `sky`'s settings with `weather`, or clear a mark it has (None).
/// False when the sky has too few fields to carry one (its name still does).
pub fn write(sky: &mut Llsd, weather: Option<&SkyWeather>) -> bool {
    let Some(fields) = carriers(sky) else {
        return false;
    };
    let all = match weather {
        Some(w) => {
            let (data, _) = w.bits();
            data | (check(data) << DATA)
        }
        // cleared: only a mark that reads is touched, its first part
        // changed so that its check no longer holds
        None if read(sky).is_some() => {
            let v = real(sky, fields[0]);
            let part = (v.to_bits() & PART_MASK) ^ 1;
            sky.insert(fields[0], f64::from(marked(v, part)));
            return true;
        }
        None => return true,
    };
    for (i, field) in fields.iter().enumerate() {
        let part = ((all >> (i as u32 * PART)) as u32) & PART_MASK;
        let v = marked(real(sky, field), part);
        sky.insert(*field, f64::from(v));
    }
    true
}

/// Mark `sky` with the weather of its name's code, or clear its mark when
/// its name has none: what Aurora saves or sends of a sky.
pub fn sync_to_name(sky: &mut Llsd) {
    if sky["type"].as_str() != "sky" {
        return;
    }
    let weather = SkyWeather::from_name(sky["name"].as_str());
    write(sky, weather.as_ref());
}

/// A marked sky whose name has no code takes its code on its name (after
/// what is left of it, or alone): what the editors and the inventory's tag
/// read.
pub fn name_from_mark(sky: &mut Llsd) {
    if SkyWeather::from_name(sky["name"].as_str()).is_some() {
        return;
    }
    let Some(weather) = read(sky) else {
        return;
    };
    let name = weather.name_with(sky["name"].as_str());
    sky.insert("name", name.trim_start());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::region::AuroraSky;

    fn sky() -> Llsd {
        aurora_llsd::llsd_map! {
            "type" => "sky",
            "name" => "Evening",
            "cloud_scale" => 0.42f64,
            "cloud_shadow" => 0.27f64,
            "dome_offset" => 0.96f64,
            "gamma" => 1.0f64,
            "max_y" => 1605.0f64,
            "moon_scale" => 1.0f64,
            "sun_arc_radians" => 0.00045f64,
            "sun_scale" => 1.0f64,
        }
    }

    fn storm() -> SkyWeather {
        SkyWeather {
            chance: 0.8,
            amount: (0.4, 1.0),
            wind: (0.2, 0.6),
            wind_from: Some(225.0),
            thunder: true,
            ..SkyWeather::CLEAR
        }
    }

    #[test]
    fn a_sky_carries_its_weather_with_and_without_an_aurora() {
        for w in [
            storm(),
            SkyWeather {
                aurora: Some(AuroraSky::default()),
                ..storm()
            },
            SkyWeather::CLEAR,
        ] {
            let mut s = sky();
            assert!(write(&mut s, Some(&w)));
            assert_eq!(read(&s).map(|r| r.code()), Some(w.code()));
        }
    }

    #[test]
    fn the_mark_cannot_be_seen_nor_pass_a_limit() {
        let before = sky();
        let mut s = before.clone();
        write(&mut s, Some(&storm()));
        for field in FIELDS {
            if let (Llsd::Real(a), Llsd::Real(b)) = (&before[field], &s[field]) {
                assert!((a - b).abs() <= a.abs() * 1e-3, "{field}: {a} -> {b}");
                // never further from zero: a value at its maximum stays in
                assert!(b.abs() <= a.abs(), "{field}: {a} -> {b}");
            }
        }
    }

    #[test]
    fn a_plain_sky_reads_as_none() {
        assert_eq!(read(&sky()), None);
        // nor a thousand skies of other numbers
        let found = (0..1000u32)
            .filter(|i| {
                let mut s = sky();
                for (k, field) in FIELDS.iter().enumerate() {
                    let v = 0.1 + f64::from((i * 7919 + k as u32 * 104_729) % 10_000) / 7_000.0;
                    s.insert(*field, v);
                }
                read(&s).is_some()
            })
            .count();
        assert_eq!(found, 0);
    }

    #[test]
    fn zero_and_missing_fields_are_passed_over_alike() {
        let mut s = sky();
        s.insert("cloud_shadow", 0.0f64);
        if let Llsd::Map(m) = &mut s {
            m.remove("gamma");
        }
        assert!(write(&mut s, Some(&storm())));
        assert_eq!(s["cloud_shadow"], Llsd::Real(0.0));
        assert_eq!(read(&s).map(|r| r.code()), Some(storm().code()));
        // too few left: nothing written, the name still carries it
        let mut bare = aurora_llsd::llsd_map! {"type" => "sky", "gamma" => 1.0f64};
        assert!(!write(&mut bare, Some(&storm())));
        assert_eq!(bare["gamma"], Llsd::Real(1.0));
    }

    #[test]
    fn the_name_sets_it_and_clears_it() {
        let mut s = sky();
        s.insert("name", storm().name_with("Evening").as_str());
        sync_to_name(&mut s);
        assert_eq!(read(&s).map(|r| r.code()), Some(storm().code()));
        // the code taken off the name ("This sky sets the weather" off)
        s.insert("name", "Evening");
        sync_to_name(&mut s);
        assert_eq!(read(&s), None);
    }

    #[test]
    fn a_day_keeps_each_key_frames_weather_through_the_server() {
        use crate::weather::region::{DayNames, RegionDay};
        // as Aurora sends it: the stormy sky named with its code
        let mut stormy = sky();
        stormy.insert("name", storm().name_with("Evening").as_str());
        sync_to_name(&mut stormy);
        // as the server gives it back: no frame names, keys renamed
        if let Llsd::Map(m) = &mut stormy {
            m.remove("name");
        }
        let day = aurora_llsd::llsd_map! {
            "type" => "daycycle",
            "name" => "Day",
            "tracks" => Llsd::Array(vec![
                Llsd::Array(vec![aurora_llsd::llsd_map! {"key_keyframe" => 0.0f64, "key_name" => "1"}]),
                Llsd::Array(vec![
                    aurora_llsd::llsd_map! {"key_keyframe" => 0.25f64, "key_name" => "2"},
                    aurora_llsd::llsd_map! {"key_keyframe" => 0.75f64, "key_name" => "3"},
                ]),
            ]),
            "frames" => aurora_llsd::llsd_map! {
                "1" => aurora_llsd::llsd_map! {"type" => "water"},
                "2" => stormy,
                "3" => sky(),
            },
        };
        let read = RegionDay::read_named(&day, &DayNames::default(), 0, 14_400.0, 0.0).expect("a day");
        // at each key frame, its own sky's weather
        assert_eq!(read.state_at(0.25).code(), storm().code());
        assert_eq!(read.state_at(0.75).code(), SkyWeather::CLEAR.code());
    }

    #[test]
    fn a_marked_sky_without_a_name_takes_its_code_back() {
        let mut s = sky();
        write(&mut s, Some(&storm()));
        if let Llsd::Map(m) = &mut s {
            m.remove("name");
        }
        name_from_mark(&mut s);
        assert_eq!(SkyWeather::from_name(s["name"].as_str()).map(|w| w.code()), Some(storm().code()));
        // a name that has a code keeps it
        let mut t = sky();
        write(&mut t, Some(&storm()));
        t.insert("name", SkyWeather::CLEAR.name_with("Noon").as_str());
        name_from_mark(&mut t);
        assert_eq!(SkyWeather::from_name(t["name"].as_str()), Some(SkyWeather::CLEAR));
    }
}
