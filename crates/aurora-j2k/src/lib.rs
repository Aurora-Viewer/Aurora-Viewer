//! A JPEG 2000 Part 1 decoder in Rust, made for the textures of Second
//! Life: whole codestreams, codestreams cut at any byte (a texture whose
//! download stopped there) and reduced resolutions, with the same pixels as
//! OpenJPEG.
//!
//! It is a port of [neo-j2k](https://github.com/anne-skydancer/neo-j2k)
//! (BSD-2-Clause, Copyright (c) 2026 Anne Skydancer), a C++ decoder derived
//! from OpenJPEG 2.5.4 (BSD-2-Clause) that keeps OpenJPEG's arithmetic and
//! rebuilds everything around it; NOTICE.md has both notices. Each module
//! names the files it follows. What this crate adds to neo-j2k is in
//! `dwt.rs` (the rows of a tile-component stored where the inverse wavelet
//! leaves them), `t1.rs` (masked indices instead of raw pointers) and
//! `par.rs` (a large tile shared with the threads its caller lends, with
//! no `unsafe` either).
//!
//! The crate has no `unsafe` and no dependency. A decode allocates nothing
//! but the image once its [`Decoder`] has decoded an image of that size.
//!
//! # Threads
//!
//! A decode runs on the thread that calls it. A caller with a pool of
//! threads can lend its idle ones ([`Options::threads`]): a tile of 512 x
//! 512 samples or more is then decoded by the calling thread and by the
//! threads of the pool that are free at that moment, with the same samples
//! whatever comes. The decoder creates no thread and blocks none: on a pool
//! whose threads are all busy, a decode is one thread's work, as without.
//!
//! What it decodes: every Part 1 codestream OpenJPEG decodes, but HT
//! code-blocks (Part 15), which no Second Life texture has. What it leaves
//! to its caller: the JP2 file format (a caller hands it the codestream of
//! the `jp2c` box) and the conversion of samples that are not 8-bit.
//!
//! # Limits
//!
//! Every [`Limits`] field is checked from the headers before what it bounds
//! is allocated. Without limits the decoder decodes any size the format
//! allows, and a header can then ask for as much memory as it likes: a
//! caller that reads codestreams it did not write sets some.
//!
//! # Cut codestreams
//!
//! A codestream cut at any byte decodes: what arrived is used. Where the
//! bytes end inside a packet, this decoder follows neo-j2k and keeps what
//! earlier packets gave the code-blocks of that packet, where OpenJPEG
//! 2.5.3 drops those code-blocks whole (`t2.rs`). [`Image::cut_in_decoded`]
//! says when a decode did so: its pixels can then be finer than OpenJPEG's
//! in those code-blocks. Every other decode has OpenJPEG's pixels.
#![forbid(unsafe_code)]

mod decode;
mod dwt;
mod header;
mod mct;
mod par;
mod t1;
mod t1_luts;
mod t2;
mod tile;

pub use header::{MainHeader, SizComponent};
pub use par::{Spawn, Threads, spawn_thread};

/// Why a codestream was not decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The bytes do not start a JPEG 2000 codestream.
    NotJpeg2000,
    /// The bytes end before the decoder has what it needs (inside a header,
    /// or before any tile): more bytes of the same codestream may decode.
    NeedData(&'static str),
    /// A header breaks the standard.
    Header(&'static str),
    /// A feature this decoder does not have (Part 2 extensions, HT
    /// code-blocks).
    Unsupported(&'static str),
    /// A [`Limits`] field, or a size no decoder holds.
    Limit(&'static str),
    /// The tiles' data is damaged.
    Data(&'static str),
    /// The allocator refused the image or the working memory.
    Memory,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotJpeg2000 => write!(f, "not a JPEG 2000 codestream"),
            Error::NeedData(what) => write!(f, "the codestream ends inside {what}"),
            Error::Header(what) => write!(f, "invalid JPEG 2000 header: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported JPEG 2000 feature: {what}"),
            Error::Limit(what) => write!(f, "JPEG 2000 decoder limit: {what}"),
            Error::Data(what) => write!(f, "damaged JPEG 2000 data: {what}"),
            Error::Memory => write!(f, "not enough memory to decode the JPEG 2000 image"),
        }
    }
}

impl std::error::Error for Error {}

/// Resource limits of a decode; 0 means no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Tiles of the image (SIZ marker).
    pub max_tiles: u32,
    /// Components of the image (SIZ marker).
    pub max_components: u32,
    /// Quality layers (COD marker of the main header and of each tile).
    pub max_layers: u32,
    /// Width and height of every component at the decoded resolution, after
    /// the reduce factor.
    pub max_decoded_side: u32,
    /// Bytes of working memory, computed from the headers before anything
    /// is allocated: the decoded samples, the wavelet's lines and each
    /// tile's code-block state.
    pub max_memory: u64,
}

impl Limits {
    /// No limits.
    pub const NONE: Limits = Limits {
        max_tiles: 0,
        max_components: 0,
        max_layers: 0,
        max_decoded_side: 0,
        max_memory: 0,
    };
    /// neo-j2k's profile for Second Life textures: 1,024 tiles, 8
    /// components, 64 quality layers, 2,048 pixels a side at the decoded
    /// resolution and 256 MiB of working memory.
    pub const SL_TEXTURE: Limits = Limits {
        max_tiles: 1024,
        max_components: 8,
        max_layers: 64,
        max_decoded_side: 2048,
        max_memory: 256 << 20,
    };
}

/// What to decode of a codestream.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Resolutions to leave out: each halves the width and the height. More
    /// than the codestream has is an error ([`MainHeader::max_reduce`]).
    pub reduce: u8,
    /// Decode the samples of this many components, the first ones; 0 for
    /// all. The others are skipped where the format allows it (their
    /// packets are still read). The first three always decode together: a
    /// colour transform ties them.
    pub components: u8,
    pub limits: Limits,
    /// The threads the caller lends this decode; none by default.
    pub threads: Threads,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            reduce: 0,
            components: 0,
            limits: Limits::NONE,
            threads: Threads::default(),
        }
    }
}

/// One component's decoded samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub width: u32,
    pub height: u32,
    /// Bit depth, 1 to 31.
    pub precision: u8,
    pub signed: bool,
    /// `width * height` samples, rows top to bottom, each in the range the
    /// precision and sign give.
    pub data: Vec<i32>,
}

/// The samples of a decoded image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Samples {
    /// Every decoded component (at most four) is 8-bit unsigned and of the
    /// image's size: `components` bytes a pixel, rows top to bottom.
    Packed(Vec<u8>),
    /// Anything else: a plane a component, as OpenJPEG gives them.
    Planes(Vec<Component>),
}

/// A decoded image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The size of the largest decoded component.
    pub width: u32,
    pub height: u32,
    /// Components decoded.
    pub components: u16,
    pub samples: Samples,
    /// The codestream ended before its end: the image is what arrived.
    pub truncated: bool,
    /// The bytes ended inside a packet of a resolution that was decoded
    /// (see the crate's documentation): the one case whose pixels can
    /// differ from OpenJPEG's.
    pub cut_in_decoded: bool,
}

/// `n` zeros. Up to 1 GiB they come from the allocator as zeros (pages the
/// system hands over blank, which nothing has to fill, and which are first
/// touched by whoever writes them: the threads of a shared decode); above
/// it, a size only a codestream without limits asks for, the memory is
/// asked for first, so that a refusal is an error and not the end of the
/// process.
pub(crate) fn zeroed<T: Clone + Default>(n: usize) -> Result<Vec<T>, Error> {
    if n.saturating_mul(std::mem::size_of::<T>()) <= 1 << 30 {
        return Ok(vec![T::default(); n]);
    }
    let mut data = Vec::new();
    data.try_reserve_exact(n).map_err(|_| Error::Memory)?;
    data.resize(n, T::default());
    Ok(data)
}

/// Working memory of the decodes, kept between them.
#[derive(Default)]
pub(crate) struct Scratch {
    pub t1: t1::T1,
    pub dwt: dwt::DwtScratch,
    /// The decoded resolution's samples of each tile-component.
    pub planes: Vec<Vec<i32>>,
    pub blocks: Vec<tile::BlockRef>,
    /// Tier-2's table of packets met.
    pub included: Vec<u64>,
    /// A tile's data gathered from several tile-parts.
    pub tile_bytes: Vec<u8>,
    /// A tile's PPT markers merged.
    pub ppt: Vec<u8>,
    /// Each component's decoded resolution.
    pub resno: Vec<u32>,
    /// The threads' own scratch when a tile's Tier-1 is shared.
    pub helpers: Vec<tile::Helper>,
}

/// A decoder: the working memory of its decodes, which the next one uses
/// again. One per thread; decodes on different decoders share nothing.
pub struct Decoder {
    tile: tile::Tile,
    scratch: Scratch,
    retained: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder {
            tile: tile::Tile::default(),
            scratch: Scratch::default(),
            retained: 8 << 20,
        }
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes of decoded samples the decoder keeps between two decodes
    /// (8 MiB unless set: a 512 x 512 texture of four components decodes
    /// without a heap call but for its image). What a larger image took is
    /// given back when its decode ends.
    pub fn set_retained_bytes(&mut self, bytes: usize) {
        self.retained = bytes;
    }

    /// Decodes a codestream (the bytes from its SOC marker on).
    pub fn decode(&mut self, codestream: &[u8], options: &Options) -> Result<Image, Error> {
        let limits = &options.limits;
        let (mut mh, start) = header::read_main_header(codestream, limits)?;
        let factor = u32::from(options.reduce);
        if factor >= mh.min_resolutions {
            return Err(Error::Limit("the reduce factor is not below the resolutions of the image"));
        }
        let numcomps = mh.comps.len();
        // (the three components of a colour transform decode together)
        let decoded = if options.components == 0 {
            numcomps
        } else {
            numcomps.min(usize::from(options.components).max(3))
        };

        // the components of the image at the factor in effect
        let mut comps = Vec::with_capacity(numcomps);
        for c in 0..numcomps {
            let (x0, y0, w, h) = header::header_component_geometry(&mh, c, factor);
            if c < decoded && limits.max_decoded_side != 0 && (w > limits.max_decoded_side || h > limits.max_decoded_side) {
                return Err(Error::Limit(
                    "a component is larger than the decoder's limit at the decoded resolution",
                ));
            }
            comps.push(decode::OutComp {
                x0,
                y0,
                w,
                h,
                written: false,
            });
        }
        let need = if limits.max_memory != 0 {
            header::decode_working_memory(&mh, factor, decoded)
        } else {
            0
        };
        if need > limits.max_memory {
            return Err(Error::Limit("the decode needs more working memory than the decoder's limit"));
        }
        let used = &comps[..decoded];
        let (width, height) = (
            used.iter().map(|c| c.w).max().unwrap_or(0),
            used.iter().map(|c| c.h).max().unwrap_or(0),
        );
        if width == 0 || height == 0 {
            return Err(Error::Data("the image has no sample at this resolution"));
        }
        let packed = decoded <= 4
            && used
                .iter()
                .zip(&mh.comps)
                .all(|(c, sc)| sc.prec == 8 && !sc.signed && c.w == width && c.h == height);
        let mut samples = if packed {
            let n = (width as usize)
                .checked_mul(height as usize)
                .and_then(|n| n.checked_mul(decoded))
                .ok_or(Error::Memory)?;
            Samples::Packed(zeroed(n)?)
        } else {
            Samples::Planes(decode::new_planes(&mh, used)?)
        };

        let job = decode::Job {
            data: codestream,
            mh: &mut mh,
            start,
            limits,
            factor,
            decoded,
            threads: options.threads,
            need,
            tile: &mut self.tile,
            scratch: &mut self.scratch,
        };
        let outcome = decode::decode_codestream(job, &mut comps, &mut samples, width as usize);
        // the tiles' samples beyond what the decoder keeps
        let mut kept = 0usize;
        for plane in &mut self.scratch.planes {
            kept = kept.saturating_add(plane.capacity() * 4);
            if kept > self.retained {
                *plane = Vec::new();
            }
        }
        let outcome = outcome?;
        Ok(Image {
            width,
            height,
            components: decoded.min(usize::from(u16::MAX)) as u16,
            samples,
            truncated: outcome.truncated,
            cut_in_decoded: outcome.cut_in_decoded,
        })
    }
}

/// Decodes a codestream with a decoder of its own. A caller that decodes
/// many keeps a [`Decoder`], one per thread.
pub fn decode(codestream: &[u8], options: &Options) -> Result<Image, Error> {
    Decoder::new().decode(codestream, options)
}

/// Reads the main header of a codestream: its size, its components and how
/// many resolutions a decode can leave out.
pub fn read_header(codestream: &[u8], limits: &Limits) -> Result<MainHeader, Error> {
    header::read_main_header(codestream, limits).map(|(mh, _)| mh)
}
