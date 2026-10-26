//! What each staff mark (`world::staff::Mark`) looks like, for the plates
//! (`paint.rs`) and the windows (`ui/staff.rs`): the Aurora team's
//! northern light (`aurora.rs`) or Linden Lab's stars and shine
//! (`linden.rs`), their tag's words, icon and colors.

use super::{aurora, linden};
use crate::theme::Palette;
use crate::world::staff::Mark;
use egui::{Color32, Mesh, Rect};

/// A mark, with the colour it is drawn in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub mark: Mark,
    /// The Linden colour (Preferences › Colors), for Linden Lab's.
    linden: Color32,
}

impl Look {
    pub fn new(mark: Mark) -> Look {
        Look {
            mark,
            linden: if mark == Mark::Linden {
                linden::color()
            } else {
                Color32::TRANSPARENT
            },
        }
    }

    /// The tag's words.
    pub fn label(&self) -> &'static str {
        match self.mark {
            Mark::Aurora => aurora_i18n::tr!("nameplate-aurora-staff"),
            Mark::Linden => aurora_i18n::tr!("nameplate-linden-lab"),
        }
    }

    /// What the tag means, under the pointer.
    pub fn tooltip(&self) -> &'static str {
        match self.mark {
            Mark::Aurora => aurora_i18n::tr!("nameplate-aurora-staff-tooltip"),
            Mark::Linden => aurora_i18n::tr!("nameplate-linden-lab-tooltip"),
        }
    }

    /// The tag's icon: sparkles for the team, the verified rosette for
    /// Linden Lab.
    pub fn icon(&self) -> &'static str {
        match self.mark {
            Mark::Aurora => "sparkles-filled",
            Mark::Linden => "rosette-discount-check-filled",
        }
    }

    /// The color of the `i`th of `n` letters of the tag at `t`.
    pub fn letter(&self, p: &Palette, i: usize, n: usize, t: f32) -> Color32 {
        match self.mark {
            Mark::Aurora => aurora::letter_color(p, i, n, t),
            Mark::Linden => linden::letter_color(p, self.linden, i, n, t),
        }
    }

    /// The icon's color at `t`.
    pub fn icon_color(&self, p: &Palette, t: f32) -> Color32 {
        self.letter(p, 0, 2, t)
    }

    /// The border's color `s` of the way around (0..1) at `t`.
    pub fn edge(&self, p: &Palette, s: f32, t: f32) -> Color32 {
        match self.mark {
            Mark::Aurora => aurora::border_color(p, s, t),
            Mark::Linden => linden::edge(p, self.linden, s, t),
        }
    }

    /// The light over a plate's top edge: curtains or rising stars.
    pub fn above(&self, mesh: &mut Mesh, p: &Palette, rect: Rect, ppp: f32, t: f32, alpha: f32) {
        match self.mark {
            Mark::Aurora => aurora::curtains(mesh, p, rect, ppp, t, alpha),
            Mark::Linden => linden::stars(mesh, p, self.linden, rect, t, alpha),
        }
    }

    /// A plate's border, `width` points inside its edge, over its halo.
    #[allow(clippy::too_many_arguments)]
    pub fn border(&self, mesh: &mut Mesh, p: &Palette, rect: Rect, ppp: f32, t: f32, alpha: f32, width: f32, scale: f32) {
        aurora::border(mesh, rect, ppp, alpha, width, scale, |s| self.edge(p, s, t));
    }

    /// The tag's surface at `rect`: `fill` inside the border.
    #[allow(clippy::too_many_arguments)]
    pub fn tag(&self, mesh: &mut Mesh, p: &Palette, rect: Rect, radius: f32, ppp: f32, t: f32, fill: Color32, alpha: f32, width: f32) {
        aurora::tag(mesh, rect, radius, ppp, fill, alpha, width, |s| self.edge(p, s + 0.25, t));
    }

    /// The light over a profile's banner: the curtains rising from its
    /// bottom edge, or stars twinkling all over it.
    pub fn banner(&self, mesh: &mut Mesh, p: &Palette, banner: Rect, ppp: f32, t: f32) {
        match self.mark {
            Mark::Aurora => {
                let foot = Rect::from_min_size(egui::pos2(banner.left(), banner.bottom() - 1.0), banner.size());
                aurora::curtains(mesh, p, foot, ppp, t, 1.0);
            }
            Mark::Linden => linden::starfield(mesh, p, self.linden, banner, t, 26.0),
        }
    }
}
