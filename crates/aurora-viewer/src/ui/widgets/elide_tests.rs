//! Names longer than their room, through real egui frames: the painted
//! shapes are what the user would see.

use super::*;

/// A display name with its username, as the lists show complete names.
pub(crate) const LONG: &str = "Maximilienne ♪ the Wandering Northern Aurora Wolf (maximilienne.wolfeschlegelsteinhausen)";
/// Wide and accented letters.
pub(crate) const WIDE: &str = "ŴŴŴ ÆØÅ MMMMWWWWMMMM ŒŒŒŒ (wwwmmmwwwmmmwwwmmm)";
pub(crate) const SHORT: &str = "Nova";

/// Frames of one egui context, 1/60 s apart.
pub(crate) struct Frames {
    pub ctx: egui::Context,
    frame: usize,
}

impl Frames {
    pub fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            frame: 0,
        }
    }

    /// Run one frame of `body` in a 1280 × 900 screen; its texts are returned.
    pub fn run(&mut self, events: Vec<egui::Event>, body: impl FnMut(&mut egui::Ui)) -> Vec<Text> {
        self.run_with_output(events, body).0
    }

    /// `run`, with what the frame asks of the platform (a copy...).
    pub fn run_with_output(&mut self, events: Vec<egui::Event>, body: impl FnMut(&mut egui::Ui)) -> (Vec<Text>, egui::PlatformOutput) {
        let (shapes, platform) = self.frame_output(events, body);
        let mut texts = Vec::new();
        for shape in &shapes {
            collect_texts(shape, &mut texts);
        }
        (texts, platform)
    }

    /// The same frame, for what is painted besides texts: its shapes, the
    /// groups of shapes opened.
    pub fn shapes(&mut self, events: Vec<egui::Event>, body: impl FnMut(&mut egui::Ui)) -> Vec<egui::Shape> {
        let mut flat = Vec::new();
        for shape in self.frame(events, body) {
            flatten(shape, &mut flat);
        }
        flat
    }

    fn frame(&mut self, events: Vec<egui::Event>, body: impl FnMut(&mut egui::Ui)) -> Vec<egui::Shape> {
        self.frame_output(events, body).0
    }

    fn frame_output(&mut self, events: Vec<egui::Event>, mut body: impl FnMut(&mut egui::Ui)) -> (Vec<egui::Shape>, egui::PlatformOutput) {
        self.frame += 1;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| body(ui),
        );
        output.textures_delta.clear();
        (
            output.shapes.into_iter().map(|clipped| clipped.shape).collect(),
            output.platform_output,
        )
    }

    /// Leave the pointer still on `pos` long enough for a tooltip, drawing
    /// `body` each frame; the texts of the last frame are returned.
    pub fn hover(&mut self, pos: egui::Pos2, mut body: impl FnMut(&mut egui::Ui)) -> Vec<Text> {
        let mut texts = self.run(vec![egui::Event::PointerMoved(pos)], &mut body);
        for _ in 0..60 {
            texts = self.run(Vec::new(), &mut body);
        }
        texts
    }
}

/// A text of a frame as it was painted.
pub(crate) struct Text {
    /// The whole text asked for.
    pub source: String,
    /// What is on screen (with the ellipsis when cut).
    pub shown: String,
    pub cut: bool,
    pub rect: egui::Rect,
}

fn flatten(shape: egui::Shape, out: &mut Vec<egui::Shape>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.into_iter().for_each(|s| flatten(s, out)),
        other => out.push(other),
    }
}

fn collect_texts(shape: &egui::Shape, out: &mut Vec<Text>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_texts(s, out)),
        egui::Shape::Text(t) => out.push(Text {
            source: t.galley.text().to_owned(),
            shown: t.galley.rows.iter().flat_map(|r| r.row.glyphs.iter().map(|g| g.chr)).collect(),
            cut: t.galley.elided,
            rect: egui::Rect::from_min_size(t.pos, t.galley.size()),
        }),
        _ => {}
    }
}

/// The text painted for `source`, as it stands in a list (not its tooltip).
pub(crate) fn painted<'a>(texts: &'a [Text], source: &str) -> &'a Text {
    texts.iter().find(|t| t.source == source).expect("the text is painted")
}

/// `source` is also shown whole somewhere: the tooltip of a cut text.
pub(crate) fn tooltip_shown(texts: &[Text], source: &str) -> bool {
    texts.iter().any(|t| t.source == source && !t.cut)
}

#[test]
fn painted_text_never_passes_its_width() {
    let mut frames = Frames::new();
    let origin = egui::pos2(50.0, 60.0);
    for text in [LONG, WIDE] {
        for width in (20..=420).step_by(7).map(|w| w as f32) {
            let mut cut = false;
            let texts = frames.run(Vec::new(), |ui| {
                cut = paint_elided(ui, text, origin, width, egui::FontId::proportional(12.0), Color32::WHITE);
            });
            let t = painted(&texts, text);
            assert!(cut && t.cut, "{text:?} does not fit in {width} px");
            assert!(t.shown.ends_with('…'), "{:?} at {width} px", t.shown);
            assert!(text.starts_with(t.shown.trim_end_matches('…')), "{:?} at {width} px", t.shown);
            assert!(
                t.rect.right() <= origin.x + width + 0.01,
                "{} px wide for {width} px",
                t.rect.width()
            );
            assert!(t.rect.left() >= origin.x - 0.01);
            // centered on the row
            assert!((t.rect.center().y - origin.y).abs() < 0.01);
        }
    }
}

#[test]
fn text_that_fits_is_left_whole() {
    let mut frames = Frames::new();
    let mut cut = true;
    let texts = frames.run(Vec::new(), |ui| {
        cut = paint_elided(
            ui,
            SHORT,
            egui::pos2(50.0, 60.0),
            200.0,
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    });
    let t = painted(&texts, SHORT);
    assert!(!cut && !t.cut);
    assert_eq!(t.shown, SHORT);
}

#[test]
fn no_room_paints_nothing_and_reports_the_cut() {
    let mut frames = Frames::new();
    let mut cut = (false, true);
    let texts = frames.run(Vec::new(), |ui| {
        let font = egui::FontId::proportional(12.0);
        cut.0 = paint_elided(ui, LONG, egui::pos2(50.0, 60.0), -30.0, font.clone(), Color32::WHITE);
        cut.1 = paint_elided(ui, "", egui::pos2(50.0, 60.0), 100.0, font, Color32::WHITE);
    });
    assert_eq!(cut, (true, false));
    assert!(texts.is_empty());
}

/// A name then a value at the right of a row, as the nearby list has.
fn row(ui: &mut egui::Ui, width: f32, name: &str) -> (egui::Rect, egui::Rect, egui::Rect) {
    let mut rects = (egui::Rect::NOTHING, egui::Rect::NOTHING);
    let row = ui
        .allocate_ui_with_layout(Vec2::new(width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let room = ui.available_width() - 70.0;
            rects.0 = ui.add(elided_label(ui, name, room)).rect;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                rects.1 = ui.label("123,45").rect;
            });
        })
        .response
        .rect;
    (row, rects.0, rects.1)
}

#[test]
fn label_leaves_the_rest_of_its_row_alone() {
    let mut frames = Frames::new();
    for name in [LONG, WIDE, SHORT] {
        for width in (120..=900).step_by(13).map(|w| w as f32) {
            let mut rects = None;
            frames.run(Vec::new(), |ui| rects = Some(row(ui, width, name)));
            let (row, label, value) = rects.expect("the row is drawn");
            assert!(label.right() <= value.left(), "{name:?} runs under the value at {width} px");
            assert!(
                row.width() <= width + 0.01,
                "{name:?} widens its row to {} at {width} px",
                row.width()
            );
        }
    }
}

#[test]
fn cut_label_shows_the_whole_text_on_hover() {
    for (name, cut) in [(LONG, true), (WIDE, true), (SHORT, false)] {
        let mut frames = Frames::new();
        let mut label = egui::Rect::NOTHING;
        let count = |texts: &[Text]| texts.iter().filter(|t| t.source == name).count();
        let texts = frames.run(Vec::new(), |ui| label = row(ui, 300.0, name).1);
        assert_eq!(painted(&texts, name).cut, cut);
        assert_eq!(count(&texts), 1, "no tooltip before the pointer comes");
        let texts = frames.hover(label.center(), |ui| {
            row(ui, 300.0, name);
        });
        // a name shown whole has nothing more to say on hover
        assert_eq!(count(&texts), if cut { 2 } else { 1 }, "{name:?}");
        assert!(!cut || tooltip_shown(&texts, name), "{name:?}");
    }
}

/// A two-word Arabic display name, as the message headers show it.
const ARABIC: &str = "ذئب بنفسجي";

#[test]
fn a_right_to_left_name_reads_from_the_right() {
    let mut frames = Frames::new();
    let texts = frames.run(Vec::new(), |ui| {
        ui.add(elided_label(ui, ARABIC, 300.0));
    });
    // egui is given the words in display order: the first one on the right
    assert!(
        texts.iter().any(|t| t.source == "بنفسجي ذئب" && !t.cut),
        "{:?}",
        texts.iter().map(|t| &t.source).collect::<Vec<_>>()
    );
}

#[test]
fn a_cut_right_to_left_name_shows_whole_on_hover() {
    let mut frames = Frames::new();
    let mut label = egui::Rect::NOTHING;
    let name = format!("{ARABIC} {ARABIC} {ARABIC} {ARABIC}");
    let texts = frames.run(Vec::new(), |ui| label = row(ui, 120.0, &name).1);
    assert!(texts.iter().any(|t| t.cut && t.shown.contains('…')), "the name is cut");
    let texts = frames.hover(label.center(), |ui| {
        row(ui, 120.0, &name);
    });
    // the tooltip: every word, uncut
    assert!(
        texts.iter().any(|t| !t.cut && t.source.matches("ذئب").count() == 4),
        "{:?}",
        texts.iter().map(|t| (&t.source, t.cut)).collect::<Vec<_>>()
    );
}
