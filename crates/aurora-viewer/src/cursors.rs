//! Original Firestorm Windows cursors: res/toolsit.cur and res/toolbuy.cur
//! (LGPL 2.1), exported as RGBA PNG without changing their hotspot (20, 15).
//! Firestorm's modern Pay cursor uses the same artwork as Buy.
use crate::interaction::Action;

pub struct Cursors {
    sit: Option<egui::CustomCursorImage>,
    money: Option<egui::CustomCursorImage>,
}
impl Default for Cursors {
    fn default() -> Self {
        fn load(bytes: &[u8]) -> Option<egui::CustomCursorImage> {
            match image::load_from_memory(bytes) {
                Ok(i) if i.width() == 32 && i.height() == 32 => Some(egui::CustomCursorImage {
                    rgba: i.into_rgba8().into_raw().into(),
                    size: [32, 32],
                    hotspot: [20, 15],
                }),
                Ok(_) => {
                    log::warn!("invalid Firestorm cursor dimensions");
                    None
                }
                Err(e) => {
                    log::warn!("cannot load Firestorm cursor: {e}");
                    None
                }
            }
        }
        Self {
            sit: load(include_bytes!("../assets/cursors/sit.png")),
            money: load(include_bytes!("../assets/cursors/money.png")),
        }
    }
}
impl Cursors {
    pub fn image(&self, action: Action) -> Option<egui::CustomCursorImage> {
        match action {
            Action::Sit => self.sit.clone(),
            Action::Buy | Action::Pay => self.money.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_artwork_hotspot_and_shared_buy_pay_buffer() {
        let c = Cursors::default();
        let buy = c.image(Action::Buy).unwrap();
        assert_eq!(buy.hotspot, [20, 15]);
        assert_eq!(buy.size, [32, 32]);
        assert!(std::sync::Arc::ptr_eq(&buy.rgba, &c.image(Action::Pay).unwrap().rgba));
        assert!(c.image(Action::Sit).is_some());
    }
}
