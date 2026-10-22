use super::*;

/// One row of the tested control at a known place.
struct Bench {
    ctx: egui::Context,
    on: bool,
    enabled: bool,
    switch: bool,
    rect: egui::Rect,
    changed: bool,
    labels: usize,
}

impl Bench {
    fn new(switch: bool) -> Self {
        let mut bench = Self {
            ctx: egui::Context::default(),
            on: false,
            enabled: true,
            switch,
            rect: egui::Rect::NOTHING,
            changed: false,
            labels: 0,
        };
        bench.frame(vec![]);
        bench
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let p = crate::theme::Theme::default().palette();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 200.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.add_enabled_ui(self.enabled, |ui| {
                    let resp = if self.switch {
                        switch_labeled(ui, &p, &mut self.on, "Setting")
                    } else {
                        checkbox(ui, &p, &mut self.on, "Setting")
                    };
                    self.rect = resp.rect;
                    self.changed = resp.changed();
                });
            },
        );
        fn count(shape: &egui::Shape, label: &str) -> usize {
            match shape {
                egui::Shape::Text(text) => usize::from(text.galley.text() == label),
                egui::Shape::Vec(shapes) => shapes.iter().map(|shape| count(shape, label)).sum(),
                _ => 0,
            }
        }
        self.labels = output.shapes.iter().map(|shape| count(&shape.shape, "Setting")).sum();
        output.textures_delta.clear();
    }

    /// Click at `pos`; whether the control reported a change.
    fn click(&mut self, pos: egui::Pos2) -> bool {
        let mut changed = false;
        for pressed in [true, false] {
            let button = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(pos), button]);
            changed |= self.changed;
        }
        changed
    }
}

#[test]
fn checkbox_keeps_the_contract_of_the_stock_one() {
    let mut bench = Bench::new(false);
    assert_eq!(bench.labels, 1, "the label is written once, as its own text");
    // the square, then the label: the whole row flips the value
    let square = bench.rect.left_center() + Vec2::new(6.0, 0.0);
    assert!(bench.click(square) && bench.on);
    let label = bench.rect.right_center() - Vec2::new(6.0, 0.0);
    assert!(bench.click(label) && !bench.on);
    // frames without a click report nothing
    bench.frame(vec![]);
    assert!(!bench.changed);
    // disabled: drawn, but a click does nothing
    bench.enabled = false;
    bench.frame(vec![]);
    assert!(!bench.click(square) && !bench.on);
    assert_eq!(bench.labels, 1);
}

#[test]
fn labeled_switch_flips_from_its_switch_and_from_its_label() {
    let mut bench = Bench::new(true);
    assert_eq!(bench.labels, 1);
    let switch = bench.rect.left_center() + Vec2::new(10.0, 0.0);
    assert!(bench.click(switch) && bench.on);
    let label = bench.rect.right_center() - Vec2::new(6.0, 0.0);
    assert!(bench.click(label) && !bench.on);
    bench.frame(vec![]);
    assert!(!bench.changed);
}

/// Three radio buttons for one value, each at a known place.
fn radio_frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    choice: &mut u8,
    enabled: bool,
) -> ([egui::Rect; 3], [bool; 3], Vec<egui::WidgetInfo>) {
    let p = crate::theme::Theme::default().palette();
    let mut rects = [egui::Rect::NOTHING; 3];
    let mut changed = [false; 3];
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 200.0))),
            events,
            ..Default::default()
        },
        |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                for (index, label) in ["One", "Two", "Three"].into_iter().enumerate() {
                    let resp = radio_value(ui, &p, choice, index as u8, label);
                    rects[index] = resp.rect;
                    changed[index] = resp.changed();
                }
            });
        },
    );
    output.textures_delta.clear();
    let infos = output
        .platform_output
        .events
        .iter()
        .map(|event| match event {
            egui::output::OutputEvent::Clicked(info)
            | egui::output::OutputEvent::DoubleClicked(info)
            | egui::output::OutputEvent::TripleClicked(info)
            | egui::output::OutputEvent::FocusGained(info)
            | egui::output::OutputEvent::TextSelectionChanged(info)
            | egui::output::OutputEvent::ValueChanged(info) => info.clone(),
        })
        .collect();
    (rects, changed, infos)
}

#[test]
fn radio_keeps_the_contract_of_the_stock_one() {
    let ctx = egui::Context::default();
    let mut choice = 0u8;
    let (rects, ..) = radio_frame(&ctx, vec![], &mut choice, true);
    // the row of the check box: same height, the mark then the label
    let mut bench = Bench::new(false);
    bench.frame(vec![]);
    assert_eq!(rects[0].height(), bench.rect.height());
    assert!(rects[1].top() > rects[0].bottom() - 0.5, "one row per choice");
    let click = |ctx: &egui::Context, pos: egui::Pos2, choice: &mut u8, enabled: bool| {
        let mut changed = [false; 3];
        let mut infos = Vec::new();
        for pressed in [true, false] {
            let button = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let (_, now, said) = radio_frame(ctx, vec![egui::Event::PointerMoved(pos), button], choice, enabled);
            for (all, one) in changed.iter_mut().zip(now) {
                *all |= one;
            }
            infos.extend(said);
        }
        (changed, infos)
    };
    // the label of the third choice selects it, and only it reports a change
    let (changed, infos) = click(&ctx, rects[2].right_center() - Vec2::new(4.0, 0.0), &mut choice, true);
    assert_eq!((choice, changed), (2, [false, false, true]));
    // announced as a radio button with its label, for the click sound and
    // the screen readers
    let info = infos.first().expect("a click is announced");
    assert_eq!(info.role, egui::Role::RadioButton);
    assert_eq!(info.label.as_deref(), Some("Three"));
    // the mark of the first one; clicking the selected choice changes nothing
    let mark = rects[0].left_center() + Vec2::new(6.0, 0.0);
    let (changed, _) = click(&ctx, mark, &mut choice, true);
    assert_eq!((choice, changed), (0, [true, false, false]));
    let (changed, _) = click(&ctx, mark, &mut choice, true);
    assert_eq!((choice, changed), (0, [false; 3]));
    // disabled: drawn, but a click does nothing
    let (changed, _) = click(&ctx, rects[1].left_center() + Vec2::new(6.0, 0.0), &mut choice, false);
    assert_eq!((choice, changed), (0, [false; 3]));
}
