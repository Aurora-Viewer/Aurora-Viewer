//! The same inventory identities and skin tints as Aurora's item and folder rows.
use super::*;

pub(in crate::ui::inventory) struct RowStyle<'a> {
    pub(in crate::ui::inventory) palette: &'a Palette,
    pub(in crate::ui::inventory) icons: &'a Icons,
    pub(in crate::ui::inventory) prefs: &'a InventoryPreferences,
    pub(in crate::ui::inventory) facts: &'a Facts,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::ui::inventory) struct Glyph {
    name: &'static str,
    tint: Color32,
    // The computed native link glyph already contains its shortcut mark.
    link: Option<&'static str>,
}

impl Glyph {
    pub(super) fn with_symbol(self, custom: Option<&'static str>) -> Self {
        Self {
            name: custom.unwrap_or(self.name),
            ..self
        }
    }

    pub(super) fn texture(self, icons: &Icons) -> Option<&egui::TextureHandle> {
        icons.get(self.name).or_else(|| match self.link {
            Some(link) => icons.get(link),
            None => icons.get("Inv_Invalid"),
        })
    }

    pub(super) fn paint(self, painter: &egui::Painter, handle: Option<&egui::TextureHandle>, ink: Color32, rect: Rect) {
        if handle.is_none() && self.link.is_some() {
            // A missing skin resource must still distinguish links from originals.
            let size = rect.width().min(rect.height());
            painter.arrow(
                rect.left_bottom() + vec2(size * 0.2, -size * 0.2),
                vec2(size * 0.6, -size * 0.6),
                egui::Stroke::new((size * 0.11).clamp(1.4, 3.0), ink),
            );
        } else if let Some(handle) = handle {
            painter.image(handle.id(), rect, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), self.tint);
        } else {
            // A partially loaded skin still leaves a visible, neutral marker.
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "?",
                FontId::proportional(rect.height().clamp(10.0, 28.0)),
                self.tint,
            );
        }
    }

    pub(super) fn tint(self) -> Color32 {
        self.tint
    }

    pub(super) fn navigation(name: &'static str, tint: Color32) -> Self {
        Self { name, tint, link: None }
    }
}

impl RowStyle<'_> {
    fn folder(&self, inv: &Inventory, id: Uuid, open: bool, selected: bool, symbol: Option<&'static str>) -> Glyph {
        let kind = inv.folders.get(&id).map_or(-1, |f| f.info.type_default);
        let native = super::super::tree::folder_icon(kind, open);
        let tint = super::super::folder_colors::row_tint(self.prefs, id, selected, self.palette);
        Glyph::navigation(symbol.unwrap_or(native), tint)
    }

    pub(super) fn resolve(&self, inv: &Inventory, row: Row, open: bool, selected: bool) -> Glyph {
        let p = self.palette;
        match row {
            Row::Item(id) => {
                let Some(item) = inv.items.get(&id) else {
                    return Glyph::navigation("Inv_Invalid", p.muted);
                };
                let (name, tint) = super::super::inventory_item_icon(inv, item, p);
                let link = match item.asset_type {
                    24 => Some("Inv_LinkItem"),
                    25 => Some("Inv_LinkFolder"),
                    _ => None,
                };
                Glyph { name, tint, link }
            }
            Row::Place(Location::Folder(id)) => self.folder(inv, id, open, selected, model::folder_symbol(inv, id)),
            Row::Place(Location::Library) => self.folder(inv, inv.lib_root, open, selected, Some("library-filled")),
            Row::Place(Location::Home) => self.folder(inv, inv.root, open, selected, Some("home-filled")),
            Row::Place(Location::Group(group)) => Glyph::navigation(group.icon(), if selected { p.violet_pale } else { p.muted }),
            Row::Place(Location::MyFolders | Location::FullTree | Location::Custom(_)) => {
                let native = super::super::tree::folder_icon(-1, open);
                Glyph::navigation(native, if selected { p.violet_pale } else { p.muted })
            }
        }
    }

    pub(in crate::ui::inventory) fn location(&self, inv: &Inventory, layout: &Preferences, place: Location, selected: bool) -> Glyph {
        let row = Row::Place(place);
        self.resolve(inv, row, selected, selected)
            .with_symbol(super::folder_icons::for_row(layout, row))
    }

    pub(in crate::ui::inventory) fn glyph(&self, painter: &egui::Painter, glyph: Glyph, rect: Rect) {
        glyph.paint(painter, glyph.texture(self.icons), self.palette.ink, rect);
    }

    pub(super) fn thumbnail_badge(&self, painter: &egui::Painter, glyph: Glyph, picture: Rect, tile_side: f32) {
        // A bare texture is its own preview. Other uploaded thumbnail images can
        // depict anything, so preserve their native object/folder/wearable identity.
        if glyph.name == "Inv_Texture" && glyph.link.is_none() {
            return;
        }
        let size = (tile_side * 0.33).clamp(12.0, 29.0);
        let badge = Rect::from_min_max(
            pos2(picture.left(), picture.bottom() - size),
            pos2(picture.left() + size, picture.bottom()),
        );
        painter.rect_filled(
            badge,
            painter.ctx().global_style().visuals.widgets.inactive.corner_radius,
            self.palette.field,
        );
        painter.rect_stroke(
            badge,
            painter.ctx().global_style().visuals.widgets.inactive.corner_radius,
            egui::Stroke::new(1.0, self.palette.muted),
            egui::StrokeKind::Inside,
        );
        // At 12 px the generic shortcut arrow reads better than a type glyph with
        // a tiny embedded corner. Larger badges preserve the original + shortcut.
        let badge_glyph = if size < 17.0 {
            glyph.link.map_or(glyph, |name| Glyph {
                name,
                tint: self.palette.ink,
                link: glyph.link,
            })
        } else {
            glyph
        };
        self.glyph(painter, badge_glyph, badge.shrink(1.5));
    }
}
