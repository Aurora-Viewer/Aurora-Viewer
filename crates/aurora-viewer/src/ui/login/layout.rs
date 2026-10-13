//! Responsive account sizes, keeping the right information column clear.
use egui::Rect;

pub struct Layout {
    pub sidebar: Option<f32>,
    pub main_width: f32,
    pub per_page: usize,
    pub paginated: bool,
    pub card_width: f32,
    pub margin: i8,
    pub gap: f32,
    pub arrow_space: f32,
    pub peek: f32,
    pub scale: f32,
    pub text_scale: f32,
}

impl Layout {
    pub fn new(screen: Rect, count: usize) -> Self {
        let sidebar = (screen.width() >= 1440.0).then(|| (320.0 + (screen.width() - 1440.0) * 0.035).min(370.0));
        let main_width = sidebar.map_or(screen.width() - 48.0, |width| screen.width() - 2.0 * (32.0 + width + 24.0));
        let growth = (screen.width() / 1600.0).min(screen.height() / 900.0).max(1.0);
        let scale = (1.0 + (growth - 1.0) * 0.4).min(1.22);
        let gap = (16.0 * scale).min(24.0);
        let arrow_space = 52.0 + 2.0 * gap;
        // Up to three profiles form a fixed row. A carousel has an odd
        // number of full cards so its selected card can be exactly centred.
        let paginated = count > 3;
        let per_page = if paginated {
            if main_width - arrow_space >= 3.0 * 216.0 + 2.0 * gap + 80.0 {
                3
            } else {
                1
            }
        } else {
            count.max(1)
        };
        let shown = count.min(per_page).max(1);
        let peek = if paginated {
            ((main_width - arrow_space - 216.0 * shown as f32 - gap * shown.saturating_sub(1) as f32) / 2.0).clamp(0.0, 88.0 * scale)
        } else {
            0.0
        };
        let card_width = (264.0 * scale)
            .min(
                (main_width - if paginated { arrow_space + 2.0 * peek } else { 0.0 } - gap * shown.saturating_sub(1) as f32) / shown as f32,
            )
            .max(1.0);
        Self {
            sidebar,
            main_width,
            per_page,
            paginated,
            card_width,
            margin: ((if screen.height() < 800.0 { 12.0 } else { 16.0 }) * scale).round() as i8,
            gap,
            arrow_space,
            peek,
            scale,
            text_scale: 1.0 + (scale - 1.0) * 0.4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cards_grow_and_pages_fit_without_touching_the_sidebar() {
        let screen = |w, h| Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
        let normal = Layout::new(screen(1600.0, 900.0), 2);
        let large = Layout::new(screen(2560.0, 1440.0), 2);
        assert!(large.card_width > normal.card_width * 1.15 && large.card_width <= normal.card_width * 1.25);
        for size in [
            (800.0, 560.0),
            (1280.0, 720.0),
            (1440.0, 900.0),
            (1600.0, 900.0),
            (2560.0, 1440.0),
            (3440.0, 1440.0),
        ] {
            for count in [1, 2, 3, 4, 8] {
                let layout = Layout::new(screen(size.0, size.1), count);
                let shown = count.min(layout.per_page);
                let row = shown as f32 * layout.card_width
                    + shown.saturating_sub(1) as f32 * layout.gap
                    + if layout.paginated { layout.arrow_space } else { 0.0 };
                let row = row + 2.0 * layout.peek;
                assert!(
                    row <= layout.main_width + 0.1,
                    "{size:?}, {count} profiles: {row} > {}",
                    layout.main_width
                );
                assert_eq!(layout.paginated, count > layout.per_page);
                assert!(layout.per_page <= 3);
                assert_eq!(layout.peek > 0.0, layout.paginated);
                if let Some(width) = layout.sidebar {
                    assert!(size.0 / 2.0 + row / 2.0 + 24.0 <= size.0 - 32.0 - width + 0.1);
                }
            }
        }
    }
}
