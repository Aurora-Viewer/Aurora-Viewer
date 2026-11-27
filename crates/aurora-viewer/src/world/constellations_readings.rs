//! The kana reading of a constellation's name, drawn over its kanji in
//! letters half its size, as a Japanese book prints a reading over a
//! word. Aurora's own: the Legacy Viewer draws no constellations.
//!
//! The data keeps its format: a name whose last bracket group, at its very
//! end, holds only kana is a name followed by its reading
//! (`北極(ほっきょく)`). The reading goes over the run of Han characters
//! that ends the name's head (the text before its first bracket); a nested
//! group of the reading goes over the run that ends the base's group of the
//! same rank (`輔(附北斗)(ほ(ふほくと))`: ほ over 輔, ふほくと over 附北斗).
//! A name of any other shape (a bracket that is not a reading, a reading
//! with no kanji under it, brackets cut by the catalogue's length limit) is
//! drawn as written. Pure functions: no fonts, tested on every system.

use std::ops::Range;

/// A reading's letters, against the name's.
pub const READING_SCALE: f32 = 0.5;
/// The room between a reading's letters and the kanji under them, in the
/// name's font size (the letters' ink, not their line boxes).
pub const READING_GAP: f32 = 0.08;

/// A name with its readings: `base` is drawn on the line, each reading
/// over its run of `base` (a range of its chars).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ruby {
    pub base: String,
    pub spans: Vec<(Range<usize>, String)>,
}

/// A piece of a name: plain text, or a bracket group and what it holds
/// (byte ranges of the name; `at` covers the brackets too).
#[derive(Debug)]
enum Node {
    Text(Range<usize>),
    Group { at: Range<usize>, inside: Vec<Node> },
}

/// The name's readings, or `None` when it carries none (or anything about
/// it is unexpected: it is then drawn as written).
pub fn ruby(name: &str) -> Option<Ruby> {
    let mut chars = name.char_indices().peekable();
    let nodes = nodes(name, &mut chars, None)?;
    let Some(Node::Group { at: last, inside }) = nodes.last() else {
        return None;
    };
    let (reading, inner) = reading(name, inside)?;
    // the base: what comes before the reading, without the space that may
    // part them (`五帝座(紫微垣) (ごていざ)`)
    let base = name[..last.start].trim_end();
    if base.is_empty() {
        return None;
    }
    let mut groups = nodes[..nodes.len() - 1].iter().filter_map(|n| match n {
        Node::Group { at, inside } => Some((at.start, inside)),
        Node::Text(_) => None,
    });
    let head_end = groups.clone().next().map_or(last.start, |(start, _)| start);
    let mut spans = vec![(han_run(name, 0..head_end)?, reading)];
    // a nested reading: one for each of the base's groups, over the run
    // that ends it
    if !inner.is_empty() {
        for text in inner {
            let (_, inside) = groups.next()?;
            let [Node::Text(r)] = inside.as_slice() else {
                return None;
            };
            spans.push((han_run(name, r.clone())?, text));
        }
        if groups.next().is_some() {
            return None;
        }
    }
    Some(Ruby {
        base: base.to_owned(),
        spans,
    })
}

/// The name's pieces up to the bracket that closes `open` (the end of the
/// name at the top). `None` for an unbalanced or mismatched bracket.
fn nodes(name: &str, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, open: Option<char>) -> Option<Vec<Node>> {
    let mut out = Vec::new();
    let mut text: Option<usize> = None;
    while let Some((i, c)) = chars.next() {
        match c {
            '(' | '（' => {
                if let Some(t) = text.take() {
                    out.push(Node::Text(t..i));
                }
                let inside = nodes(name, chars, Some(c))?;
                let end = chars.peek().map_or(name.len(), |(j, _)| *j);
                out.push(Node::Group { at: i..end, inside });
            }
            ')' | '）' => {
                // a bracket closes only the kind that opened it
                let want = match open? {
                    '(' => ')',
                    _ => '）',
                };
                if c != want {
                    return None;
                }
                if let Some(t) = text.take() {
                    out.push(Node::Text(t..i));
                }
                return Some(out);
            }
            _ => {
                text.get_or_insert(i);
            }
        }
    }
    // the end of the name: right only at the top
    if open.is_some() {
        return None;
    }
    if let Some(t) = text {
        out.push(Node::Text(t..name.len()));
    }
    Some(out)
}

/// A reading group's text: its kana, and the kana of each group it nests
/// (`ほ(ふほくと)`). `None` for anything else.
fn reading(name: &str, inside: &[Node]) -> Option<(String, Vec<String>)> {
    let (Node::Text(first), rest) = inside.split_first()? else {
        return None;
    };
    let outer = kana(&name[first.clone()])?;
    let mut inner = Vec::new();
    for n in rest {
        let Node::Group { inside, .. } = n else {
            return None;
        };
        let [Node::Text(r)] = inside.as_slice() else {
            return None;
        };
        inner.push(kana(&name[r.clone()])?);
    }
    Some((outer, inner))
}

/// `s` when it is only kana (and not empty).
fn kana(s: &str) -> Option<String> {
    let is_kana =
        |c: char| matches!(c, '\u{3041}'..='\u{3096}' | '\u{3099}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}');
    (!s.is_empty() && s.chars().all(is_kana)).then(|| s.to_owned())
}

fn is_han(c: char) -> bool {
    matches!(
        c,
        '\u{3005}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{3134F}'
    )
}

/// The run of Han characters that ends `name[bytes]` (spaces after it
/// left out), as a range of the name's chars; `None` when it ends with
/// something else (katakana, a letter).
fn han_run(name: &str, bytes: Range<usize>) -> Option<Range<usize>> {
    let part = name.get(bytes.clone())?.trim_end();
    let run = part.chars().rev().take_while(|c| is_han(*c)).count();
    if run == 0 {
        return None;
    }
    let end = name[..bytes.start].chars().count() + part.chars().count();
    Some(end - run..end)
}

/// Where the readings go over their runs, in the base's layout units
/// (`runs`: each run's left and right edge on the line, `widths`: its
/// reading's width, `base_w`: the base's width). Each reading is centred
/// over its run, and may overhang it. Returns each reading's left edge and
/// the width a picture centred on the base needs for them all.
pub fn place(runs: &[[f32; 2]], widths: &[f32], base_w: f32) -> (Vec<f32>, f32) {
    let centre = base_w * 0.5;
    let mut half = centre;
    let lefts = runs
        .iter()
        .zip(widths)
        .map(|(r, w)| {
            let left = (r[0] + r[1] - w) * 0.5;
            half = half.max(centre - left).max(left + w - centre);
            left
        })
        .collect();
    (lefts, 2.0 * half)
}

/// The picture's frame around its glyphs (`[x0, y0, x1, y1]`, pixels):
/// `all` the ink of the base and its readings, `base` the base's. The base
/// stays centred across (a reading that overhangs on one side widens the
/// other as much); up and down, the readings above it make `lift`, the
/// rows the picture's centre lies above the base's centre, doubled (the
/// shader puts the base's centre on the label point). Without readings:
/// `all`, 0.
pub fn frame(base: [i32; 4], all: [i32; 4]) -> ([i32; 4], u32) {
    let [mut x0, mut y0, mut x1, y1] = all;
    let dx = (x0 + x1) - (base[0] + base[2]);
    if dx > 0 {
        x0 -= dx;
    } else {
        x1 -= dx;
    }
    let lift = (base[1] + base[3]) - (y0 + y1);
    if lift < 0 {
        // ink lower than the base (not a reading's place): room above
        // instead, the base centred
        y0 += lift;
    }
    ([x0, y0, x1, y1], lift.max(0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(name: &str) -> Option<(String, Vec<(String, String)>)> {
        let r = ruby(name)?;
        let chars: Vec<char> = r.base.chars().collect();
        let s = r
            .spans
            .iter()
            .map(|(run, text)| (chars[run.clone()].iter().collect(), text.clone()))
            .collect();
        Some((r.base, s))
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_owned(), b.to_owned())
    }

    #[test]
    fn a_kana_group_at_the_end_reads_the_kanji_before_it() {
        assert_eq!(spans("北極(ほっきょく)"), Some(("北極".into(), vec![pair("北極", "ほっきょく")])));
        assert_eq!(
            spans("大熊座（おおぐまざ）"),
            Some(("大熊座".into(), vec![pair("大熊座", "おおぐまざ")]))
        );
        assert_eq!(spans("水星(すいせい)"), Some(("水星".into(), vec![pair("水星", "すいせい")])));
        // the base's own group stays on the line, the space before the
        // reading goes
        assert_eq!(
            spans("五帝座(紫微垣) (ごていざ)"),
            Some(("五帝座(紫微垣)".into(), vec![pair("五帝座", "ごていざ")]))
        );
        // katakana, ー and ・ are kana; 々 is a kanji
        assert_eq!(
            spans("人々(ヒト・ビートー)"),
            Some(("人々".into(), vec![pair("人々", "ヒト・ビートー")]))
        );
    }

    #[test]
    fn a_reading_sits_over_the_kanji_run_only() {
        assert_eq!(
            spans("ハーシェルの望遠鏡座(ぼうえんきょうざ)"),
            Some(("ハーシェルの望遠鏡座".into(), vec![pair("望遠鏡座", "ぼうえんきょうざ")]))
        );
        assert_eq!(
            spans("テーブル山座(さんざ)"),
            Some(("テーブル山座".into(), vec![pair("山座", "さんざ")]))
        );
        assert_eq!(
            spans("ジョージの琴座(ことざ)"),
            Some(("ジョージの琴座".into(), vec![pair("琴座", "ことざ")]))
        );
        assert_eq!(spans("狼の顎(あご)"), Some(("狼の顎".into(), vec![pair("顎", "あご")])));
        // the runs are chars of the base
        assert_eq!(ruby("狼の顎(あご)").map(|r| r.spans[0].0.clone()), Some(2..3));
    }

    #[test]
    fn a_nested_reading_goes_over_the_base_group_of_its_rank() {
        assert_eq!(
            spans("輔(附北斗)(ほ(ふほくと))"),
            Some(("輔(附北斗)".into(), vec![pair("輔", "ほ"), pair("附北斗", "ふほくと")]))
        );
        assert_eq!(
            spans("三台(上台)(さんだい(じょうだい))"),
            Some(("三台(上台)".into(), vec![pair("三台", "さんだい"), pair("上台", "じょうだい")]))
        );
        assert_eq!(ruby("輔(附北斗)(ほ(ふほくと))").map(|r| r.spans[1].0.clone()), Some(2..5));
        // a nested reading with no group of its rank, or one too few
        assert_eq!(ruby("輔(ほ(ふほくと))"), None);
        assert_eq!(ruby("輔(附北斗)(甲)(ほ(ふほくと))"), None);
        // its group holds no kanji to go over, or more than text
        assert_eq!(ruby("輔(ほくと)(ほ(ふほくと))"), None);
        assert_eq!(ruby("輔(附(北))(ほ(ふほくと))"), None);
        // the nested reading's own shape
        assert_eq!(ruby("輔(附北斗)((ふほくと)ほ)"), None);
        assert_eq!(ruby("輔(附北斗)(ほ(ふほくと)ほ)"), None);
        assert_eq!(ruby("輔(附北斗)(ほ(ふ(ほ)くと))"), None);
    }

    #[test]
    fn other_brackets_draw_as_written() {
        for name in [
            "太微垣(たいびえん：太微垣)",
            "杵(箕宿)",
            "Dog(犬)",
            "Triangle(三角形)",
            "Celestial Dog (In Dipper Mansion)",
            "三階(上階)",
            "Orion",
            "おおぐま座",
            "",
            // no kanji before the reading
            "ポラリス(ぽらりす)",
            "(ほっきょく)",
            "北極 ポラリス(ほっきょく)",
            // something after the reading
            "北極(ほっきょく)星",
            "北極(ほっきょく) ",
            // empty, unbalanced, mismatched, cut by the length limit
            "北極()",
            "北極(（ほっきょく）",
            "北極(ほっきょく",
            "北極ほっきょく)",
            "北極(ほっきょく）",
            "北極（ほっきょく)",
            "輔(附北斗)(ほ(ふほくと)",
            "輔(附北斗)(ほ(ふほくと)))",
            "輔(附北斗(ほ(ふほくと))",
        ] {
            assert_eq!(ruby(name), None, "{name}");
        }
        // a space between the head and the reading is not part of the run
        assert_eq!(spans("北極 (ほっきょく)"), Some(("北極".into(), vec![pair("北極", "ほっきょく")])));
        // every cut of a nested reading name (the catalogue's length
        // limit): no panic, no reading
        let name = "輔(附北斗)(ほ(ふほくと))";
        for n in 0..name.chars().count() {
            let cut: String = name.chars().take(n).collect();
            assert_eq!(ruby(&cut), None, "{cut}");
        }
    }

    #[test]
    fn readings_are_centred_over_their_runs_and_may_overhang() {
        // 北極(ほっきょく): two em of kanji, five half-em letters
        let (lefts, wide) = place(&[[0.0, 2.0]], &[2.5], 2.0);
        assert_eq!(lefts, [-0.25]);
        assert_eq!(wide, 2.5, "the overhang on both sides");
        // a short reading: within the base
        let (lefts, wide) = place(&[[4.0, 6.0]], &[1.5], 6.0);
        assert_eq!(lefts, [4.25]);
        assert_eq!(wide, 6.0);
        // over the last kanji, overhanging on its right: the picture as
        // wide on the left, so the base stays centred
        let (lefts, wide) = place(&[[2.0, 3.0]], &[2.0], 3.0);
        assert_eq!(lefts, [1.5]);
        assert_eq!(wide, 4.0);
        // two runs
        let (lefts, _) = place(&[[0.0, 1.0], [2.0, 5.0]], &[0.5, 2.0], 5.0);
        assert_eq!(lefts, [0.25, 2.5]);
    }

    #[test]
    fn the_frame_keeps_the_base_centred_and_lifts_it() {
        let base = [0, 40, 100, 100];
        assert_eq!(frame(base, base), (base, 0), "no reading: as before");
        // (`all` holds the base: the union of its ink and the readings')
        // a reading 30 rows above, within the base's width
        assert_eq!(frame(base, [0, 10, 100, 100]), ([0, 10, 100, 100], 30));
        // overhanging on the right by 20: as much room on the left
        assert_eq!(frame(base, [0, 10, 120, 100]), ([-20, 10, 120, 100], 30));
        // overhanging on the left
        assert_eq!(frame(base, [-6, 10, 100, 100]), ([-6, 10, 106, 100], 30));
        // ink below the base: room above, no lift
        assert_eq!(frame(base, [0, 40, 100, 110]), ([0, 30, 100, 110], 0));
        // the label point, half the picture plus half the lift down, is
        // the base's centre (doubled, in rows from the frame's top)
        for all in [[0, 10, 100, 100], [0, 13, 100, 100], [0, 40, 100, 110]] {
            let ([_, y0, _, y1], lift) = frame(base, all);
            assert_eq!((y1 - y0) + lift as i32, base[1] + base[3] - 2 * y0, "{all:?}");
        }
    }

    #[test]
    fn a_name_and_its_reading_fit_a_picture_at_the_largest_size() {
        // the largest name font, with ink as tall as 1.5 of its em for the
        // name and for the reading (both more than any font draws), the gap
        // and the pad: within the 512 rows a name's picture may have
        let em = super::super::name_em_texels(*super::super::FONT_RANGE.end());
        let rows = 1.5 * em + 1.5 * em * READING_SCALE + READING_GAP * em + 12.0;
        assert!(rows < 512.0, "{rows}");
    }
}
