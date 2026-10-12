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
