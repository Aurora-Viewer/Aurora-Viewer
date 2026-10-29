//! Photos of the world (Firestorm's snapshot floater and "Take Snapshot to
//! Disk", LLFloaterSnapshot / LLSnapshotLivePreview / LLViewerWindow::
//! rawSnapshot and saveImageLocal, indra/newview, originally LGPL 2.1): the
//! sizes and proportions a photo can have, how it is split when the card
//! cannot draw it in one pass, its file name, and the settings kept between
//! sessions. Saving runs on a worker (`save`), the recent photos are listed
//! by `gallery`, a photo for the inventory is prepared by `inventory`, one
//! for the profile feed posted by `profile`; what the photo shows is chosen
//! by `isolate`; the window is `ui::snapshot`, the frames `app::snapshot`.

pub mod gallery;
pub mod inventory;
pub mod isolate;
pub mod knobs;
pub mod light;
pub mod looks;
pub mod luts;
pub mod profile;
pub mod quality;
pub mod save;
pub mod viewfinder;

use glam::{Mat4, Vec4};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// MAX_SNAPSHOT_IMAGE_SIZE of llviewerwindow.h: the largest side.
pub const MAX_SIDE: u32 = 7680;
/// The smallest side (the floater's spinners, min_val="32").
pub const MIN_SIDE: u32 = 32;

/// Proportions of the photo, one click each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    /// Those of the window (Firestorm's default: "Current Window").
    Window,
    R16x9,
    R3x2,
    R4x3,
    R1x1,
    R4x5,
    R9x16,
}

impl Aspect {
    pub const ALL: [Aspect; 7] = [
        Aspect::Window,
        Aspect::R16x9,
        Aspect::R3x2,
        Aspect::R4x3,
        Aspect::R1x1,
        Aspect::R4x5,
        Aspect::R9x16,
    ];

    /// Width over height; the window's for `Window`.
    pub fn ratio(self, window: (u32, u32)) -> f32 {
        let (w, h) = match self {
            Aspect::Window => (window.0.max(1), window.1.max(1)),
            Aspect::R16x9 => (16, 9),
            Aspect::R3x2 => (3, 2),
            Aspect::R4x3 => (4, 3),
            Aspect::R1x1 => (1, 1),
            Aspect::R4x5 => (4, 5),
            Aspect::R9x16 => (9, 16),
        };
        w as f32 / h as f32
    }

    /// "16:9", or None for the window (its label is translated).
    pub fn short(self) -> Option<&'static str> {
        match self {
            Aspect::Window => None,
            Aspect::R16x9 => Some("16:9"),
            Aspect::R3x2 => Some("3:2"),
            Aspect::R4x3 => Some("4:3"),
            Aspect::R1x1 => Some("1:1"),
            Aspect::R4x5 => Some("4:5"),
            Aspect::R9x16 => Some("9:16"),
        }
    }
}

/// Size of the photo: its long side, or the window's, or typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// The window's size (with its proportions; otherwise its long side).
    Window,
    Hd,
    Qhd,
    Uhd,
    K5,
    K8,
    Custom,
}

impl Resolution {
    pub const ALL: [Resolution; 7] = [
        Resolution::Window,
        Resolution::Hd,
        Resolution::Qhd,
        Resolution::Uhd,
        Resolution::K5,
        Resolution::K8,
        Resolution::Custom,
    ];

    /// The long side of a preset.
    pub fn long_side(self) -> Option<u32> {
        match self {
            Resolution::Hd => Some(1920),
            Resolution::Qhd => Some(2560),
            Resolution::Uhd => Some(3840),
            Resolution::K5 => Some(5120),
            Resolution::K8 => Some(MAX_SIDE),
            Resolution::Window | Resolution::Custom => None,
        }
    }

    pub fn short(self) -> Option<&'static str> {
        match self {
            Resolution::Hd => Some("HD"),
            Resolution::Qhd => Some("QHD"),
            Resolution::Uhd => Some("4K"),
            Resolution::K5 => Some("5K"),
            Resolution::K8 => Some("8K"),
            Resolution::Window | Resolution::Custom => None,
        }
    }
}

/// File format on disk (SnapshotFormat: PNG, JPEG, BMP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileFormat {
    Png,
    Jpeg,
    Bmp,
}

impl FileFormat {
    pub const ALL: [FileFormat; 3] = [FileFormat::Png, FileFormat::Jpeg, FileFormat::Bmp];

    pub fn extension(self) -> &'static str {
        match self {
            FileFormat::Png => "png",
            FileFormat::Jpeg => "jpg",
            FileFormat::Bmp => "bmp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileFormat::Png => "PNG",
            FileFormat::Jpeg => "JPEG",
            FileFormat::Bmp => "BMP",
        }
    }

    /// The format of a file name chosen in "Save as", by its extension.
    pub fn of_path(path: &Path) -> Option<FileFormat> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some(FileFormat::Png),
            "jpg" | "jpeg" => Some(FileFormat::Jpeg),
            "bmp" => Some(FileFormat::Bmp),
            _ => None,
        }
    }
}

/// Composition guides over the preview and the full-frame view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Guide {
    None,
    Thirds,
    Golden,
    Diagonals,
    Center,
}

impl Guide {
    pub const ALL: [Guide; 4] = [Guide::Thirds, Guide::Golden, Guide::Diagonals, Guide::Center];

    /// Lines in unit coordinates of the frame (0..1 both ways, y down).
    pub fn lines(self) -> Vec<[[f32; 2]; 2]> {
        let v = |x: f32| [[x, 0.0], [x, 1.0]];
        let h = |y: f32| [[0.0, y], [1.0, y]];
        match self {
            Guide::None => Vec::new(),
            Guide::Thirds => vec![v(1.0 / 3.0), v(2.0 / 3.0), h(1.0 / 3.0), h(2.0 / 3.0)],
            Guide::Golden => {
                // phi grid: lines at 1/phi^2 and 1/phi of each side
                let a = 1.0 - 1.0 / std::f32::consts::GOLDEN_RATIO;
                vec![v(a), v(1.0 - a), h(a), h(1.0 - a)]
            }
            Guide::Diagonals => vec![[[0.0, 0.0], [1.0, 1.0]], [[1.0, 0.0], [0.0, 1.0]]],
            Guide::Center => vec![v(0.5), h(0.5)],
        }
    }
}

/// Settings of the photo window, kept between sessions (Firestorm keeps
/// them in its settings: LastSnapshotToDiskWidth / Height, KeepAspect-
/// ForSnapshot, SnapshotFormat, SnapshotQuality, RenderUIInSnapshot,
/// RenderHUDInSnapshot, AutoSnapshot, FSSnapshotLocalNamesWithTimestamps,
/// FSRememberSnapshotPathSessions, PlayModeUISndSnapshot, SnapshotBaseDir).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub aspect: Aspect,
    pub resolution: Resolution,
    /// Typed size (Custom).
    pub custom: [u32; 2],
    /// "Constrain proportions": the typed size keeps the chosen aspect.
    pub keep_ratio: bool,
    pub format: FileFormat,
    /// JPEG quality, 0..=100 (SnapshotQuality, 75).
    pub jpeg_quality: u8,
    /// The interface drawn on the photo (only with the window's proportions).
    pub interface: bool,
    pub huds: bool,
    /// The preview follows the camera.
    pub auto_refresh: bool,
    /// Date and time in the file name.
    pub timestamp_names: bool,
    /// Folder of the photos; None: the pictures folder.
    pub folder: Option<PathBuf>,
    /// Keep the folder chosen in "Save as" for the next sessions.
    pub remember_folder: bool,
    /// First part of the file name (SnapshotBaseName, "Snapshot").
    pub base_name: String,
    pub guide: Guide,
    /// The shutter sound and the snapshot animation, for everyone around;
    /// off: "Quiet snapshots" (PlayModeUISndSnapshot on). The photo window,
    /// Preferences › Privacy and › Sound, and Advanced › Quiet snapshots
    /// all switch it.
    pub sound: bool,
    /// Size of a photo for the inventory (LastSnapshotToInventoryResolution).
    pub inventory_size: inventory::TextureSize,
    /// Size of a photo for the profile feed (LastSnapshotToProfileResolution).
    pub profile_size: profile::ProfileSize,
    /// The place in the profile post (SnapshotToProfileIncludeLocation).
    pub profile_location: bool,
    /// Size of a postcard (LastSnapshotToEmailResolution: 800 × 600).
    pub postcard_size: profile::ProfileSize,
    /// The window's depth of field (the Preferences' own is left alone).
    pub dof: crate::dof::PhotoDof,
    /// "Photo quality": the graphics raised while the window is open
    /// (`quality`), the user's settings back once it closes.
    pub quality: bool,
    /// The viewfinder's display, AF area, peaking and beep.
    pub viewfinder: viewfinder::Prefs,
    /// The photo's looks, the first one drawn first (`looks`); none: the
    /// Legacy Viewer's "No Filter".
    pub looks: Vec<looks::Layer>,
    /// The settings of Aurora's looks by id (`knobs`), kept when a look is
    /// taken out.
    pub look_settings: std::collections::BTreeMap<String, Vec<f32>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            aspect: Aspect::Window,
            // 4K: a photo, not a screenshot of the window
            resolution: Resolution::Uhd,
            custom: [1920, 1080],
            keep_ratio: true,
            format: FileFormat::Png,
            jpeg_quality: 75,
            interface: false,
            huds: false,
            auto_refresh: false,
            timestamp_names: true,
            folder: None,
            remember_folder: true,
            base_name: "Snapshot".into(),
            guide: Guide::Thirds,
            sound: true,
            inventory_size: inventory::TextureSize::S512,
            profile_size: profile::ProfileSize::S640,
            profile_location: true,
            postcard_size: profile::ProfileSize::S800,
            dof: Default::default(),
            quality: false,
            viewfinder: Default::default(),
            looks: Vec::new(),
            look_settings: Default::default(),
        }
    }
}

impl Settings {
    /// Bring loaded values back into their ranges.
    pub fn sanitize(&mut self) {
        self.custom = [clamp_side(self.custom[0]), clamp_side(self.custom[1])];
        self.jpeg_quality = self.jpeg_quality.min(100);
        if self.base_name.trim().is_empty() {
            self.base_name = "Snapshot".into();
        }
        // FSRememberSnapshotPathSessions off: the folder lasts one session
        self.dof.sanitize();
        if !self.remember_folder {
            self.folder = None;
        }
    }

    /// The size of the photo for a window of `window` pixels.
    pub fn size(&self, window: (u32, u32)) -> (u32, u32) {
        photo_size(self.aspect, self.resolution, self.custom, window)
    }

    /// Choose the proportions: a typed size whose proportions are kept
    /// takes the new ones, its long side kept (otherwise it stayed as
    /// typed and the choice changed nothing).
    pub fn set_aspect(&mut self, aspect: Aspect, window: (u32, u32)) {
        self.aspect = aspect;
        if self.resolution == Resolution::Custom && self.keep_ratio {
            let long = self.custom[0].max(self.custom[1]);
            let ratio = aspect.ratio((window.0.max(1), window.1.max(1)));
            self.custom = if ratio >= 1.0 {
                [long, clamp_side((long as f32 / ratio).round() as u32)]
            } else {
                [clamp_side((long as f32 * ratio).round() as u32), long]
            };
        }
    }

    /// The typed size after one side changed, the other following when the
    /// proportions are kept.
    pub fn set_custom(&mut self, width: Option<u32>, height: Option<u32>, window: (u32, u32)) {
        let ratio = self.aspect.ratio(window);
        if let Some(w) = width {
            self.custom[0] = clamp_side(w);
            if self.keep_ratio {
                self.custom[1] = clamp_side((self.custom[0] as f32 / ratio).round() as u32);
            }
        }
        if let Some(h) = height {
            self.custom[1] = clamp_side(h);
            if self.keep_ratio {
                self.custom[0] = clamp_side((self.custom[1] as f32 * ratio).round() as u32);
            }
        }
    }

    /// The folder photos go to.
    pub fn folder(&self) -> PathBuf {
        self.folder.clone().unwrap_or_else(default_folder)
    }
}

/// A photo of `size` has the window's proportions (to 1 %): the interface
/// can be drawn on it (it is laid out for the window and scaled, not moved).
pub fn same_proportions(size: (u32, u32), window: (u32, u32)) -> bool {
    let photo = size.0 as f32 / size.1.max(1) as f32;
    let win = window.0 as f32 / window.1.max(1) as f32;
    (photo - win).abs() <= 0.01 * win
}

fn clamp_side(v: u32) -> u32 {
    v.clamp(MIN_SIDE, MAX_SIDE)
}

/// `<Pictures>/Aurora Viewer`, or the data folder when there is no
/// pictures folder (Firestorm asks the first time; a ready folder lets the
/// first photo be one click).
pub fn default_folder() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.picture_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| crate::settings::config_dir().join("photos"))
        .join("Aurora Viewer")
}

/// Size of a photo: the preset's long side with the proportions, the
/// window's size, or the typed one; even sides between MIN_SIDE and
/// MAX_SIDE (the encoders and the viewers of most sites prefer them).
pub fn photo_size(aspect: Aspect, resolution: Resolution, custom: [u32; 2], window: (u32, u32)) -> (u32, u32) {
    let window = (window.0.max(1), window.1.max(1));
    let ratio = aspect.ratio(window);
    let (w, h) = match resolution {
        Resolution::Custom => (custom[0], custom[1]),
        Resolution::Window if aspect == Aspect::Window => window,
        _ => {
            let long = resolution.long_side().unwrap_or(window.0.max(window.1));
            if ratio >= 1.0 {
                (long, (long as f32 / ratio).round() as u32)
            } else {
                ((long as f32 * ratio).round() as u32, long)
            }
        }
    };
    let even = |v: u32| (clamp_side(v) / 2 * 2).max(MIN_SIDE);
    (even(w), even(h))
}

/// A part of a photo, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One pass of a photo: the tile it keeps, and the larger part of the photo
/// it draws for it (the tile and its guard band, see [`TILE_GUARD`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilePass {
    pub keep: Tile,
    pub draw: Tile,
}

/// Photo pixels drawn around a tile on each side it shares with another
/// tile, then cropped, at least. The screen-space passes read the depth
/// around each pixel: GTAO's horizon search, its normal and its edges, the
/// contact shadows' rays (60 pixels). A tile drawn alone had no scene past
/// its border, and GTAO took the border row it read again beyond it for a
/// wall: a dark line along the seams. GTAO's radius (0.73 m) spans fewer
/// pixels than the band beyond 9 m in a photo 1,800 pixels high, and its
/// farthest samples weigh the least; the band costs 128 pixels of a side of
/// 4,096 or more in the photos that need tiles. The bloom and the depth of
/// field reach farther and widen it while they are on
/// (`aurora_render::bloom_tile_reach`, `dof_tile_reach`).
pub const TILE_GUARD: u32 = 128;

/// The pixel a drawn tile starts on, and the multiple its size is rounded
/// to (but at the photo's edges): the bloom halves the image six times, so
/// its levels' texels (and the depth of field's tiles of 32 pixels) fall
/// on those of one pass. Off them, each tile blurred its own grid and the
/// halos stepped at the seams.
const TILE_ALIGN: u32 = 64;

/// How a photo of `size` is drawn: one pass when the card holds it (a side
/// within `max_side`, the pixels within `max_pixels`, the memory its render
/// targets take), otherwise the fewest equal tiles that fit with their guard
/// bands of `guard` pixels (LLViewerWindow::rawSnapshot draws tiles of the
/// window's size; one pass keeps the screen-space effects exact, so tiles
/// are the fallback only).
pub fn plan(size: (u32, u32), max_side: u32, max_pixels: u64, guard: u32) -> Vec<TilePass> {
    let (w, h) = (size.0.max(1), size.1.max(1));
    let max_side = max_side.max(MIN_SIDE);
    let max_pixels = max_pixels.max(u64::from(MIN_SIDE) * u64::from(MIN_SIDE));
    // at most a quarter of the side a pass can draw, with its alignment, so
    // that the tiles keep about half of what they draw (and always fit, down
    // to the smallest limits)
    let room = (max_side / 4).min(max_pixels.isqrt() as u32 / 4);
    let guard = guard.min(room);
    let align = TILE_ALIGN.min(prev_power_of_two(guard.max(2)));
    // the widest a pass draws when `side` is split in `n`: a tile, a band
    // on each side it shares and the pixels that put its edges on the
    // alignment (at most 2 x guard + 2 x (align - 1) more than the tile:
    // n large enough always fits)
    let drawn = |side: u32, n: u32| {
        let t = side.div_ceil(n);
        (0..n)
            .map(|i| i * t)
            .take_while(|&x| x < side)
            .map(|x| band(side, x, t.min(side - x), guard, align).1)
            .max()
            .unwrap_or(side)
    };
    let mut nx = w.div_ceil(max_side);
    let mut ny = h.div_ceil(max_side);
    loop {
        let (dw, dh) = (drawn(w, nx), drawn(h, ny));
        if dw > max_side {
            nx += 1;
        } else if dh > max_side {
            ny += 1;
        } else if u64::from(dw) * u64::from(dh) > max_pixels {
            // split the longer tile side
            if w.div_ceil(nx) >= h.div_ceil(ny) {
                nx += 1;
            } else {
                ny += 1;
            }
        } else {
            break;
        }
    }
    let (tw, th) = (w.div_ceil(nx), h.div_ceil(ny));
    let mut tiles = Vec::with_capacity((nx * ny) as usize);
    for j in 0..ny {
        for i in 0..nx {
            let (x, y) = (i * tw, j * th);
            if x < w && y < h {
                let keep = Tile {
                    x,
                    y,
                    width: tw.min(w - x),
                    height: th.min(h - y),
                };
                tiles.push(TilePass {
                    keep,
                    draw: guarded((w, h), keep, guard, align),
                });
            }
        }
    }
    tiles
}

/// The largest power of two within `v` (at least 1).
fn prev_power_of_two(v: u32) -> u32 {
    1 << (31 - v.max(1).leading_zeros())
}

/// `tile` of a photo of `size` and `guard` pixels around it, within the
/// photo, its edges on multiples of `align` (an even number): its own edges
/// stay those of one pass. The origin is at least on an even pixel, the
/// pixels GTAO at half resolution reads in one pass.
fn guarded(size: (u32, u32), tile: Tile, guard: u32, align: u32) -> Tile {
    let (x, width) = band(size.0, tile.x, tile.width, guard, align);
    let (y, height) = band(size.1, tile.y, tile.height, guard, align);
    Tile { x, y, width, height }
}

/// The start and the length drawn along one side of `side` pixels for the
/// `len` pixels kept from `start` (see [`guarded`]).
fn band(side: u32, start: u32, len: u32, guard: u32, align: u32) -> (u32, u32) {
    let from = start.saturating_sub(guard) / align * align;
    let to = (start + len + guard).next_multiple_of(align).min(side);
    (from, to - from)
}

/// The projection that draws `tile` of a photo of `size` with the photo's
/// projection `proj` in front of it: the tile's part of clip space is
/// stretched to the whole target (an off-centre frustum, like the tiles
/// of LLViewerCamera::setPerspective with its subregion).
pub fn tile_matrix(size: (u32, u32), tile: Tile) -> Mat4 {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let sx = w / tile.width.max(1) as f32;
    let sy = h / tile.height.max(1) as f32;
    // the tile's centre in normalised device coordinates (y up)
    let cx = 2.0 * (tile.x as f32 + tile.width as f32 * 0.5) / w - 1.0;
    let cy = 1.0 - 2.0 * (tile.y as f32 + tile.height as f32 * 0.5) / h;
    Mat4::from_cols(
        Vec4::new(sx, 0.0, 0.0, 0.0),
        Vec4::new(0.0, sy, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(-sx * cx, -sy * cy, 0.0, 1.0),
    )
}

/// Copy the RGBA rows of the tile a pass keeps, out of the `pixels` it
/// drew, into the photo.
pub fn stitch(photo: &mut [u8], photo_width: u32, pass: TilePass, pixels: &[u8]) {
    let (keep, draw) = (pass.keep, pass.draw);
    let row = keep.width as usize * 4;
    let (dx, dy) = ((keep.x - draw.x) as usize, (keep.y - draw.y) as usize);
    for y in 0..keep.height as usize {
        let from = ((dy + y) * draw.width as usize + dx) * 4;
        let start = ((keep.y as usize + y) * photo_width as usize + keep.x as usize) * 4;
        photo[start..start + row].copy_from_slice(&pixels[from..from + row]);
    }
}

/// The instant a photo drawn in several frames shows, kept from its first
/// part to its last (issues #977, #1109). The Legacy Viewer draws every tile
/// in one call (LLViewerWindow::rawSnapshot, indra/newview/llviewerwindow.cpp,
/// originally LGPL 2.1: display() per tile, no idle() between them), so
/// nothing moves from one tile to the next; Aurora draws a tile per frame and
/// stops the world's clock instead, or an avatar that moves is cut at every
/// seam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    /// The world's time: animations, motions, flexible prims, rez effects.
    pub at: std::time::Instant,
    /// The shaders' clock (water, portals), seconds since the start.
    pub clock: f32,
    /// A later part: nothing moves since the first (no network event
    /// applied, no step of time); the first part is drawn from the live
    /// world, whose instant is kept.
    pub later: bool,
}

impl Hold {
    /// The hold of the frame that draws part `next` of a photo of `tiles`
    /// parts, `now` this frame's; `kept`, the job's, records the first
    /// part's. None for a photo in one pass.
    pub fn for_part(tiles: usize, next: usize, kept: &mut Option<Hold>, now: Hold) -> Option<Hold> {
        if tiles <= 1 {
            return None;
        }
        match *kept {
            Some(first) if next > 0 => Some(Hold { later: true, ..first }),
            _ => {
                let first = Hold { later: false, ..now };
                *kept = Some(first);
                Some(first)
            }
        }
    }
}

/// Vertical field of view of a focal length on a full-frame camera (the
/// 24 mm side of 36 x 24 mm), the photographer's way to set the camera's.
pub fn fov_of_focal(mm: f32) -> f32 {
    2.0 * (12.0 / mm.max(1.0)).atan()
}

pub fn focal_of_fov(fov_y: f32) -> f32 {
    12.0 / (fov_y * 0.5).tan().max(1e-4)
}

/// File name of a photo, like LLViewerWindow::saveImageLocal:
/// `<base>[_YYYY-MM-DD_HHMMSS]_NNN.<ext>`, NNN the first number not taken
/// in the folder.
pub fn file_name(base: &str, stamp: Option<&str>, number: u32, format: FileFormat) -> String {
    let base: String = base
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    match stamp {
        Some(s) => format!("{base}_{s}_{number:03}.{}", format.extension()),
        None => format!("{base}_{number:03}.{}", format.extension()),
    }
}

/// The first free file name in `folder`.
pub fn free_path(folder: &Path, base: &str, stamp: Option<&str>, format: FileFormat) -> PathBuf {
    let mut n = 1;
    loop {
        let path = folder.join(file_name(base, stamp, n, format));
        if !path.exists() || n >= 9999 {
            return path;
        }
        n += 1;
    }
}

/// Local date and time for file names, `YYYY-MM-DD_HHMMSS`
/// (LLDate::toLocalDateString("%Y-%m-%d_%H%M%S")).
pub fn local_stamp() -> String {
    let (y, mo, d, h, mi, s) = local_time();
    format!("{y:04}-{mo:02}-{d:02}_{h:02}{mi:02}{s:02}")
}

#[cfg(windows)]
fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime only fills the structure it is given.
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    (t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(not(windows))]
fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    // the TZ variable, else the system zone (/etc/localtime)
    civil(&chrono::Local::now())
}

/// A date and time's fields in its own time zone, as `local_time` gives them.
#[cfg_attr(windows, allow(dead_code))]
fn civil(t: &(impl chrono::Datelike + chrono::Timelike)) -> (u16, u16, u16, u16, u16, u16) {
    (
        u16::try_from(t.year()).unwrap_or(0),
        t.month() as u16,
        t.day() as u16,
        t.hour() as u16,
        t.minute() as u16,
        t.second() as u16,
    )
}

#[cfg(test)]
mod tests;
