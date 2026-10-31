//! AURORA_DEMO_AA_SWITCH=<frame>:<mode>[,<frame>:<mode>…]: the
//! anti-aliasing changed at those frames, as Preferences › Graphics would
//! (modes of `AURORA_AA`: 0 none, 1 FXAA, 2 to 4 MSAA 2x / 4x / 8x, 5 TAA,
//! 6 SMAA). A change of MSAA samples compiles the scene's pipelines on a
//! worker (#1746): the frames go on meanwhile, `AURORA_PROFILE` shows no
//! long frame and the log gives `pipelines for <n> sample(s) ready in … ms`.

/// The switches, sorted by frame; empty when the variable is unset or
/// unreadable (a bad entry is skipped and logged).
pub fn from_env() -> Vec<(u64, u8)> {
    std::env::var("AURORA_DEMO_AA_SWITCH").map(|v| parse(&v)).unwrap_or_default()
}

fn parse(v: &str) -> Vec<(u64, u8)> {
    let mut out: Vec<(u64, u8)> = v
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .filter_map(|e| {
            let parsed = e
                .split_once(':')
                .and_then(|(f, m)| Some((f.trim().parse().ok()?, m.trim().parse().ok().filter(|m: &u8| *m <= 6)?)));
            if parsed.is_none() {
                log::warn!("AURORA_DEMO_AA_SWITCH: `{e}` skipped (expected <frame>:<mode 0-6>)");
            }
            parsed
        })
        .collect();
    out.sort_by_key(|(frame, _)| *frame);
    out
}

/// The mode to switch to on `frame`, taken from `switches`.
pub fn due(switches: &mut Vec<(u64, u8)>, frame: u64) -> Option<u8> {
    let mut mode = None;
    while switches.first().is_some_and(|(at, _)| *at <= frame) {
        mode = Some(switches.remove(0).1);
    }
    mode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sorted_and_skips_bad_entries() {
        assert_eq!(parse("900:4, 700:2,x:3,800:9,1000:"), vec![(700, 2), (900, 4)]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn due_takes_the_last_passed_switch() {
        let mut s = vec![(700, 2), (710, 4), (900, 0)];
        assert_eq!(due(&mut s, 600), None);
        assert_eq!(due(&mut s, 720), Some(4));
        assert_eq!(s, vec![(900, 0)]);
        assert_eq!(due(&mut s, 900), Some(0));
        assert!(s.is_empty());
    }
}
