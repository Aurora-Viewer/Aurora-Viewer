//! Prim water (#2008): phantom prims whose name or description holds one of
//! the chosen words act as water for our own avatar only. Their top replaces
//! the region water height in `Frame`, so swimming itself is unchanged.
//! Nothing here is sent to the simulator; other viewers see ordinary flight.
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

pub const DEFAULT_WORDS: &str = "AuroraWater";
/// Both horizontal sides at least this long: no rails, ropes or signs.
pub const MIN_WIDTH: f32 = 1.0;
/// Thinner volumes are water planes: their water reaches the floor beneath.
pub const THIN: f32 = 0.5;
/// About 8°: a tilted box is a ramp or a wave, not a pool.
const UPRIGHT: f32 = 0.99;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Words separated by commas or spaces, matched as whole words.
    pub words: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            words: DEFAULT_WORDS.to_owned(),
        }
    }
}

/// The words of the list, lower case, without repeats.
pub fn words(list: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in list.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let word = word.trim().to_lowercase();
        if !word.is_empty() && !out.contains(&word) {
            out.push(word);
        }
    }
    out
}

/// One of `words` in `name` or `description`, not inside a longer word
/// ("pool" matches "Pool water" and "pool_01", never "Liverpool").
pub fn matches(words: &[String], name: &str, description: &str) -> bool {
    let name = name.to_lowercase();
    let description = description.to_lowercase();
    words.iter().any(|word| whole_word(&name, word) || whole_word(&description, word))
}

fn whole_word(text: &str, word: &str) -> bool {
    let boundary = |c: Option<char>| !c.is_some_and(char::is_alphanumeric);
    text.match_indices(word)
        .any(|(at, _)| boundary(text[..at].chars().next_back()) && boundary(text[at + word.len()..].chars().next()))
}

/// An upright box: its top is flat, its footprint the two other axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub center: Vec3,
    pub rotation: Quat,
    pub half: Vec3,
    /// The local axis that points up or down.
    vertical: usize,
}
impl Volume {
    /// None for a box tilted away from every axis, or too narrow to swim in.
    pub fn new(center: Vec3, rotation: Quat, size: Vec3) -> Option<Self> {
        if !center.is_finite() || !rotation.is_finite() || !size.is_finite() {
            return None;
        }
        let half = size.abs() * 0.5;
        let vertical = (0..3).find(|&axis| (rotation * Vec3::AXES[axis]).z.abs() >= UPRIGHT)?;
        let wide = (0..3).filter(|&axis| axis != vertical).all(|axis| half[axis] * 2.0 >= MIN_WIDTH);
        wide.then_some(Self {
            center,
            rotation,
            half,
            vertical,
        })
    }
    pub fn top(&self) -> f32 {
        self.center.z + self.half[self.vertical]
    }
    pub fn bottom(&self) -> f32 {
        self.center.z - self.half[self.vertical]
    }
    pub fn thin(&self) -> bool {
        self.half[self.vertical] * 2.0 < THIN
    }
    /// `point` is above or below the box, within its sides.
    pub fn over(&self, point: Vec3) -> bool {
        let local = self.rotation.inverse() * (point - self.center);
        (0..3)
            .filter(|&axis| axis != self.vertical)
            .all(|axis| local[axis].abs() <= self.half[axis])
    }
}

/// The water a volume gives an avatar: its surface, and the floor `deep`
/// and diving measure against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Water {
    pub surface: f32,
    pub floor: f32,
}

/// The volume around an avatar of `height` at `position`, `floor` being the
/// highest solid ground beneath it. From under the water to the height
/// where region water still holds a leaping swimmer; the highest top wins.
pub fn water_at(volumes: &[Volume], position: Vec3, height: f32, floor: Option<f32>) -> Option<Water> {
    volumes
        .iter()
        .filter(|v| v.over(position))
        .filter_map(|v| {
            let top = v.top();
            // A plane's water goes down to whatever it lies over.
            let bottom = if v.thin() { floor.unwrap_or(v.bottom()) } else { v.bottom() };
            let floor = floor.filter(|f| *f <= top).unwrap_or(bottom).max(bottom.min(top));
            let inside = (bottom - height * 0.5..=top + height + 1.5).contains(&position.z);
            inside.then_some(Water { surface: top, floor })
        })
        .max_by(|a, b| a.surface.total_cmp(&b.surface))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(center: Vec3, size: Vec3) -> Volume {
        Volume::new(center, Quat::IDENTITY, size).expect("upright pool")
    }

    #[test]
    fn word_list_splits_lowercases_and_dedupes() {
        assert_eq!(words(" AuroraWater, pool;POOL  pond,,"), ["aurorawater", "pool", "pond"]);
        assert!(words(" , ; ").is_empty());
        assert_eq!(Settings::default().words, DEFAULT_WORDS);
        assert!(!Settings::default().enabled);
    }

    #[test]
    fn words_match_whole_words_in_name_or_description() {
        let w = words("AuroraWater, pool");
        assert!(matches(&w, "AuroraWater", ""));
        assert!(matches(&w, "Pool water", ""));
        assert!(matches(&w, "pool_01", ""));
        assert!(matches(&w, "Box", "heated (AuroraWater) v2"));
        assert!(!matches(&w, "Liverpool sign", "poolside bar"));
        assert!(!matches(&w, "AuroraWaterfall", ""));
        assert!(!matches(&[], "AuroraWater", ""));
        // non-ASCII text around a word stays a boundary
        assert!(matches(&words("eau"), "Piscine — eau", ""));
        assert!(!matches(&words("eau"), "Château", ""));
    }

    #[test]
    fn volumes_must_be_upright_and_wide() {
        assert!(Volume::new(Vec3::ZERO, Quat::IDENTITY, Vec3::new(4.0, 4.0, 2.0)).is_some());
        // lying on its side: another axis is vertical, the footprint follows
        let side = Volume::new(
            Vec3::ZERO,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            Vec3::new(4.0, 2.0, 6.0),
        )
        .expect("rotated pool");
        assert!((side.top() - 1.0).abs() < 1e-4);
        assert!(side.over(Vec3::new(1.9, 2.9, 0.0)) && !side.over(Vec3::new(1.9, 3.1, 0.0)));
        // turned about Z: still upright, footprint turns with it
        let turned = Volume::new(Vec3::ZERO, Quat::from_rotation_z(0.6), Vec3::new(4.0, 1.2, 2.0)).expect("turned pool");
        assert!(turned.over(Quat::from_rotation_z(0.6) * Vec3::new(1.9, 0.0, 0.0)));
        assert!(!turned.over(Vec3::new(0.0, 1.9, 0.0)));
        assert!(
            Volume::new(Vec3::ZERO, Quat::from_rotation_x(0.4), Vec3::splat(4.0)).is_none(),
            "tilted"
        );
        assert!(
            Volume::new(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.5, 8.0, 2.0)).is_none(),
            "a rail"
        );
        assert!(Volume::new(Vec3::NAN, Quat::IDENTITY, Vec3::splat(4.0)).is_none());
    }

    #[test]
    fn a_raised_pool_gives_its_top_and_floor() {
        // skybox pool: top at 62.4, floor at 60, terrain hundreds of metres below
        let v = [pool(Vec3::new(10.0, 10.0, 61.2), Vec3::new(8.0, 8.0, 2.4))];
        let at = |z: f32, floor| water_at(&v, Vec3::new(10.0, 10.0, z), 1.7, floor);
        let water = at(61.8, Some(60.0)).expect("in the pool");
        assert_eq!(water.surface, 62.4);
        assert_eq!(water.floor, 60.0);
        // a floor reported below the box (terrain) stops at the box bottom
        assert_eq!(at(61.8, Some(25.0)).map(|w| w.floor), Some(60.0));
        assert_eq!(at(61.8, None).map(|w| w.floor), Some(60.0));
        // leaping out keeps the water until region water would drop it too
        assert!(at(62.4 + 1.7 + 1.4, Some(60.0)).is_some());
        assert!(at(62.4 + 1.7 + 1.6, Some(60.0)).is_none());
        // under the pool, or beside it, is dry
        assert!(at(58.0, Some(25.0)).is_none());
        assert!(water_at(&v, Vec3::new(14.5, 10.0, 61.8), 1.7, Some(60.0)).is_none());
    }

    #[test]
    fn a_thin_plane_reaches_the_floor_beneath() {
        let v = [pool(Vec3::new(0.0, 0.0, 62.375), Vec3::new(8.0, 8.0, 0.05))];
        assert!(v[0].thin());
        let water = water_at(&v, Vec3::new(0.0, 0.0, 60.8), 1.7, Some(60.0)).expect("under the plane");
        assert_eq!((water.surface, water.floor), (62.4, 60.0));
        assert!(
            water_at(&v, Vec3::new(0.0, 0.0, 58.9), 1.7, Some(60.0)).is_none(),
            "below the floor"
        );
    }

    #[test]
    fn the_highest_surface_wins_where_volumes_overlap() {
        let v = [
            pool(Vec3::new(0.0, 0.0, 61.0), Vec3::new(8.0, 8.0, 2.0)),
            pool(Vec3::new(0.0, 0.0, 61.5), Vec3::new(2.0, 2.0, 2.0)),
        ];
        assert_eq!(
            water_at(&v, Vec3::new(0.0, 0.0, 61.0), 1.7, Some(60.0)).map(|w| w.surface),
            Some(62.5)
        );
        assert_eq!(
            water_at(&v, Vec3::new(3.0, 0.0, 61.0), 1.7, Some(60.0)).map(|w| w.surface),
            Some(62.0)
        );
    }
}
