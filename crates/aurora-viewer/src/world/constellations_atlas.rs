//! Where the pictures and names sit in the renderer's atlas
//! (aurora-render constellations.rs): layers of `ATLAS_W` x `ATLAS_H`
//! texels in blocks of `BLOCK`, a rectangle of whole blocks per item, the
//! first free one found by a summed-area table. Layers are added as the
//! view needs them, up to `MAX_LAYERS`; then the least recently used items
//! that nothing draws make room. An item something draws is never taken
//! away: what does not fit waits, and a figure that cannot wait draws its
//! lines and name alone.

use aurora_render::constellations::{ATLAS_H, ATLAS_W, BLOCK, Item, MAX_LAYERS};

/// The layers the atlas may take unless told more (`Atlas::set_room`): the
/// budget of one culture's figures, as before there were several. Several
/// cultures' names in a wide field get up to `MAX_LAYERS`.
pub const FEW_LAYERS: u32 = 3;

const COLS: usize = (ATLAS_W / BLOCK) as usize;
const ROWS: usize = (ATLAS_H / BLOCK) as usize;
/// Texels kept free around an item (its bilinear and mip taps).
const GUTTER: u32 = 4;

/// An item's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub layer: u32,
    pub block: [u32; 2],
    /// Its size in blocks.
    pub blocks: [u32; 2],
}

#[derive(Debug)]
struct Kept {
    item: Item,
    place: Place,
    used: u64,
}

#[derive(Debug, Default)]
pub struct Atlas {
    /// Per layer, which blocks are taken.
    taken: Vec<Vec<bool>>,
    kept: Vec<Kept>,
    /// The frame counter of `touch`.
    clock: u64,
    /// The layers it may take (0: `FEW_LAYERS`).
    room: u32,
}

impl Atlas {
    /// Layers in use (the renderer's texture has as many).
    pub fn layers(&self) -> u32 {
        self.taken.len() as u32
    }

    pub fn place_of(&self, item: Item) -> Option<Place> {
        self.kept.iter().find(|k| k.item == item).map(|k| k.place)
    }

    /// The item is drawn this frame: the last to be made room for.
    pub fn touch(&mut self, item: Item) {
        let clock = self.clock;
        if let Some(k) = self.kept.iter_mut().find(|k| k.item == item) {
            k.used = clock;
        }
    }

    /// The layers it may take from now on, up to `MAX_LAYERS` (the ones it
    /// has stay until it is cleared).
    pub fn set_room(&mut self, layers: u32) {
        self.room = layers.min(MAX_LAYERS);
    }

    /// One frame more on the clock.
    pub fn tick(&mut self) {
        self.clock += 1;
    }

    /// Lets every place go (the renderer's atlas is gone).
    pub fn clear(&mut self) {
        self.taken.clear();
        self.kept.clear();
    }

    pub fn release(&mut self, item: Item) {
        if let Some(i) = self.kept.iter().position(|k| k.item == item) {
            let k = self.kept.swap_remove(i);
            self.mark(k.place, false);
        }
    }

    fn mark(&mut self, p: Place, taken: bool) {
        let layer = &mut self.taken[p.layer as usize];
        for y in p.block[1]..p.block[1] + p.blocks[1] {
            for x in p.block[0]..p.block[0] + p.blocks[0] {
                layer[y as usize * COLS + x as usize] = taken;
            }
        }
    }

    /// A place for `item` of `size` texels: the first free one, else in a
    /// new layer, else where the least recently used items that `in_use`
    /// does not hold were. `None`: no room, nothing changed.
    pub fn place(&mut self, item: Item, size: [u32; 2], in_use: impl Fn(Item) -> bool) -> Option<Place> {
        if let Some(p) = self.place_of(item) {
            return Some(p);
        }
        let blocks = [(size[0] + GUTTER).div_ceil(BLOCK), (size[1] + GUTTER).div_ceil(BLOCK)];
        if blocks[0] as usize > COLS || blocks[1] as usize > ROWS || size[0] == 0 || size[1] == 0 {
            return None;
        }
        loop {
            if let Some(p) = self.free(blocks) {
                self.mark(p, true);
                self.kept.push(Kept {
                    item,
                    place: p,
                    used: self.clock,
                });
                return Some(p);
            }
            if self.layers() < if self.room == 0 { FEW_LAYERS } else { self.room } {
                self.taken.push(vec![false; COLS * ROWS]);
                continue;
            }
            let oldest = self
                .kept
                .iter()
                .enumerate()
                .filter(|(_, k)| !in_use(k.item))
                .min_by_key(|(_, k)| k.used)
                .map(|(i, _)| i)?;
            let k = self.kept.swap_remove(oldest);
            self.mark(k.place, false);
        }
    }

    fn free(&self, blocks: [u32; 2]) -> Option<Place> {
        let (bw, bh) = (blocks[0] as usize, blocks[1] as usize);
        for (layer, taken) in self.taken.iter().enumerate() {
            // sums[y][x]: blocks taken above and left of (x, y)
            let mut sums = vec![0u32; (COLS + 1) * (ROWS + 1)];
            for y in 0..ROWS {
                for x in 0..COLS {
                    sums[(y + 1) * (COLS + 1) + x + 1] =
                        u32::from(taken[y * COLS + x]) + sums[y * (COLS + 1) + x + 1] + sums[(y + 1) * (COLS + 1) + x]
                            - sums[y * (COLS + 1) + x];
                }
            }
            let s = |x: usize, y: usize| sums[y * (COLS + 1) + x];
            for y in 0..=ROWS - bh {
                for x in 0..=COLS - bw {
                    if s(x + bw, y + bh) + s(x, y) - s(x + bw, y) - s(x, y + bh) == 0 {
                        return Some(Place {
                            layer: layer as u32,
                            block: [x as u32, y as u32],
                            blocks,
                        });
                    }
                }
            }
        }
        None
    }
}

/// An `Rg8` image and its halved levels (`count` in all), each texel the
/// mean of the up to four it covers.
pub fn levels(rg: &[u8], w: u32, h: u32, count: u32) -> Vec<Vec<u8>> {
    let mut out = vec![rg.to_vec()];
    let (mut w, mut h) = (w as usize, h as usize);
    for _ in 1..count {
        let src = out.last().map_or(&[][..], Vec::as_slice);
        let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
        let mut dst = vec![0u8; nw * nh * 2];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..2 {
                    let mut sum = 0u32;
                    let mut n = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = (2 * x + dx, 2 * y + dy);
                        if sx < w && sy < h {
                            sum += u32::from(src[(sy * w + sx) * 2 + c]);
                            n += 1;
                        }
                    }
                    dst[(y * nw + x) * 2 + c] = ((sum + n / 2) / n.max(1)) as u8;
                }
            }
        }
        out.push(dst);
        (w, h) = (nw, nh);
    }
    out
}

/// An engraving's WebP (ink in its grey, the reveal time in its alpha) as
/// `Rg8` levels; it must be `size`.
pub fn decode(webp: &[u8], size: [u32; 2]) -> Result<Vec<Vec<u8>>, String> {
    // the user's file: its header's size is checked against its JSON and
    // the atlas before any pixel is decoded, and the decoder's memory is
    // bounded
    if webp.get(..4) != Some(b"RIFF".as_slice()) || webp.get(8..12) != Some(b"WEBP".as_slice()) {
        return Err("not a WebP file".into());
    }
    let reader = || {
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(webp), image::ImageFormat::WebP);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(ATLAS_W);
        limits.max_image_height = Some(ATLAS_H);
        limits.max_alloc = Some(u64::from(ATLAS_W) * u64::from(ATLAS_H) * 4 * 2);
        reader.limits(limits);
        reader
    };
    let declared = reader().into_dimensions().map_err(|e| e.to_string())?;
    if declared != (size[0], size[1]) {
        return Err(format!("{declared:?} texels, not {size:?}"));
    }
    let img = reader().decode().map_err(|e| e.to_string())?;
    let la = img.to_luma_alpha8();
    if la.dimensions() != (size[0], size[1]) {
        return Err(format!("{:?} texels, not {size:?}", la.dimensions()));
    }
    Ok(levels(la.as_raw(), size[0], size[1], aurora_render::constellations::ATLAS_MIPS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_take_free_blocks_then_new_layers_then_the_oldest_unused() {
        let mut a = Atlas::default();
        let never = |_: Item| false;
        // Hydra's size fills most of a layer
        let big = [2689, 1410];
        let p0 = a.place(Item::Picture(0), big, never).expect("first");
        assert_eq!((p0.layer, p0.block), (0, [0, 0]));
        assert_eq!(p0.blocks, [43, 23]);
        let p1 = a.place(Item::Picture(1), [600, 600], never).expect("beside it");
        assert_eq!(p1.layer, 0);
        assert!(p1.block[0] >= 43 || p1.block[1] >= 23);
        assert_eq!(a.place(Item::Picture(1), [600, 600], never), Some(p1), "placed once");
        a.place(Item::Picture(2), big, never).expect("second layer");
        a.place(Item::Picture(3), big, never).expect("third layer");
        assert_eq!(a.layers(), FEW_LAYERS);
        // full: picture 0 is drawn, 2 is the oldest the others can spare
        a.tick();
        a.touch(Item::Picture(0));
        a.touch(Item::Picture(3));
        let in_use = |i: Item| i == Item::Picture(0);
        let p4 = a.place(Item::Picture(4), big, in_use).expect("room made");
        assert!(a.place_of(Item::Picture(0)).is_some(), "what is drawn stays");
        assert!(a.place_of(Item::Picture(2)).is_none() || a.place_of(Item::Picture(1)).is_none());
        assert!(p4.layer > 0);
        // nothing can be spared: no room, and nothing changed
        let all = |_: Item| true;
        let before = a.kept.len();
        assert_eq!(a.place(Item::Name(9), big, all), None);
        assert_eq!(a.kept.len(), before);
        // too large, or empty
        assert_eq!(a.place(Item::Name(8), [5000, 10], never), None);
        assert_eq!(a.place(Item::Name(8), [0, 10], never), None);
        a.release(Item::Picture(0));
        assert!(a.place_of(Item::Picture(0)).is_none());
        a.clear();
        assert_eq!(a.layers(), 0);
        // given more room, it grows past FEW_LAYERS, up to MAX_LAYERS
        a.set_room(99);
        for k in 0..MAX_LAYERS {
            a.place(Item::Picture(10 + k), big, never).expect("a layer each");
        }
        assert_eq!(a.layers(), MAX_LAYERS);
        assert_eq!(a.place(Item::Picture(30), big, all), None);
    }

    #[test]
    fn levels_average_what_they_cover() {
        // 3 x 1: [0, 200], [100, 0], [50, 50]
        let l = levels(&[0, 200, 100, 0, 50, 50], 3, 1, 3);
        assert_eq!(l.len(), 3);
        assert_eq!(l[1], vec![50, 100, 50, 50]);
        assert_eq!(l[2], vec![50, 75]);
        assert!(decode(b"not a webp", [1, 1]).is_err());
    }
    #[test]
    fn a_bad_picture_is_an_error_never_a_panic() {
        // a small lossless WebP made here: 3 x 2 texels, LA
        let mut png = Vec::new();
        let img = image::DynamicImage::ImageLumaA8(image::ImageBuffer::from_fn(3, 2, |x, y| {
            image::LumaA([(x * 40) as u8, (y * 90) as u8])
        }));
        img.to_rgba8()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::WebP)
            .expect("encode");
        let levels = decode(&png, [3, 2]).expect("a good picture");
        assert_eq!(levels.len(), aurora_render::constellations::ATLAS_MIPS as usize);
        assert_eq!(levels[0].len(), 3 * 2 * 2);
        // its JSON says another size
        assert!(decode(&png, [4, 2]).is_err());
        // every truncation, and bytes flipped
        // (the stream's last bytes may be padding: what decodes is whole)
        for n in 0..png.len() {
            if let Ok(levels) = decode(&png[..n], [3, 2]) {
                assert!(n + 4 >= png.len() && levels[0].len() == 12, "truncated at {n}");
            }
        }
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..500 {
            let mut b = png.clone();
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = 12 + (seed % (b.len() as u64 - 12)) as usize;
            b[at] ^= (seed >> 40) as u8 | 1;
            let _ = decode(&b, [3, 2]);
        }
        // a header that claims more than the atlas: refused before decoding
        let mut huge = png.clone();
        if let Some(at) = huge.windows(4).position(|w| w == b"VP8L") {
            // VP8L: 14-bit width-1 and height-1 after the signature byte
            let bits: u32 = (16383) | (16383 << 14);
            huge[at + 9..at + 13].copy_from_slice(&bits.to_le_bytes());
        }
        assert!(decode(&huge, [16384, 16384]).is_err());
        assert!(decode(b"RIFF\0\0\0\0WEBPVP8L", [1, 1]).is_err());
        assert!(decode(b"", [1, 1]).is_err());
    }
}
