// This file is a port and keeps the notices of what it ports (both under the
// BSD 2-Clause License, whose text NOTICE.md gives, "JPEG 2000 decoder").
//
// neo-j2k, `src/header.cpp`:
//   Copyright (c) 2026, Anne Skydancer (neo-j2k)
// OpenJPEG, through it:
//   Copyright (c) 2002-2014, Universite catholique de Louvain (UCL), Belgium
//   Copyright (c) 2002-2014, Professor Benoit Macq
//   Copyright (c) 2001-2003, David Janssens
//   Copyright (c) 2002-2003, Yannick Verschueren
//   Copyright (c) 2003-2007, Francois-Olivier Devaux
//   Copyright (c) 2003-2014, Antonin Descampe
//   Copyright (c) 2005, Herve Drolon, FreeImage Team
//   Copyright (c) 2008, Jerome Fimes, Communications & Systemes <jerome.fimes@c-s.fr>
//   Copyright (c) 2006-2007, Parvatha Elangovan
//   Copyright (c) 2010-2011, Kaori Hagihara
//   Copyright (c) 2011-2012, Centre National d'Etudes Spatiales (CNES), France
//   Copyright (c) 2012, CS Systemes d'Information, France
//   Copyright (c) 2017, IntoPIX SA <support@intopix.com>
//   All rights reserved.

//! The codestream header model: the main header and the tile-part headers.
//!
//! Port of neo-j2k's `src/header.cpp` and `src/header.h` (BSD-2-Clause,
//! Copyright (c) 2026 Anne Skydancer), whose checks derive from OpenJPEG
//! 2.5.4's `j2k.c` (`opj_j2k_read_siz`, `opj_j2k_read_cod` and their
//! siblings; BSD-2-Clause, see NOTICE.md).
//!
//! The reader accepts and refuses what OpenJPEG does on valid streams. Where
//! OpenJPEG accepts what the standard forbids, or reads a valid stream in a
//! way that depends on marker order, it follows the standard, as neo-j2k does
//! (its `doc/departures.md`, DP-5 to DP-8):
//! - marker precedence is applied when a tile-component's parameters are
//!   resolved, whatever the order the markers came in: tile-part COC >
//!   tile-part COD > main COC > main COD, the same for QCC / QCD and RGN;
//! - a quantization with fewer step sizes than its component's sub-bands is
//!   refused;
//! - a marker segment the decoder does not know is skipped by its length;
//! - JPEG 2000 Part 2 markers and HT code-blocks are refused.
//!
//! Every limit is checked before what it bounds is allocated.

use std::collections::BTreeMap;

use crate::{Error, Limits};

/// `OPJ_J2K_MAXRLVLS`.
pub(crate) const MAX_RESOLUTIONS: usize = 33;
/// `OPJ_J2K_MAXBANDS`.
pub(crate) const MAX_BANDS: usize = 3 * MAX_RESOLUTIONS - 2;
/// OpenJPEG's `J2K_MAX_POCS`: at most 31 progression changes per tile.
const MAX_POCS: usize = 32;
const MAX_COMPONENTS: u32 = 16384;
const MAX_TILES: u64 = 65535;

pub(crate) mod mk {
    pub const SOC: u16 = 0xFF4F;
    pub const CAP: u16 = 0xFF50;
    pub const SIZ: u16 = 0xFF51;
    pub const COD: u16 = 0xFF52;
    pub const COC: u16 = 0xFF53;
    pub const TLM: u16 = 0xFF55;
    pub const PLM: u16 = 0xFF57;
    pub const PLT: u16 = 0xFF58;
    pub const CPF: u16 = 0xFF59;
    pub const QCD: u16 = 0xFF5C;
    pub const QCC: u16 = 0xFF5D;
    pub const RGN: u16 = 0xFF5E;
    pub const POC: u16 = 0xFF5F;
    pub const PPM: u16 = 0xFF60;
    pub const PPT: u16 = 0xFF61;
    pub const CRG: u16 = 0xFF63;
    pub const COM: u16 = 0xFF64;
    pub const SOT: u16 = 0xFF90;
    pub const SOP: u16 = 0xFF91;
    pub const EPH: u16 = 0xFF92;
    pub const SOD: u16 = 0xFF93;
    pub const EOC: u16 = 0xFFD9;
}

/// Markers that carry no segment (ISO 15444-1 A.1): SOC, SOD, EOC, EPH and
/// 0xFF30-0xFF3F.
pub(crate) fn segmentless(m: u32) -> bool {
    m == u32::from(mk::SOC)
        || m == u32::from(mk::SOD)
        || m == u32::from(mk::EOC)
        || m == u32::from(mk::EPH)
        || (0xFF30..=0xFF3F).contains(&m)
}

/// JPEG 2000 Part 2 markers (DCO, VMS, DFS, ADS, MCT, MCC, NLT, MCO, CBD, ATK…).
pub(crate) fn part2_marker(m: u32) -> bool {
    (0xFF70..=0xFF7F).contains(&m)
}

pub(crate) const CSTY_PRECINCTS: u8 = 0x01;
pub(crate) const CSTY_SOP: u8 = 0x02;
pub(crate) const CSTY_EPH: u8 = 0x04;
pub(crate) const CBLK_HT: u8 = 0x40;
const CBLK_HT_MIXED: u8 = 0x80;
const QNT_NONE: u8 = 0;
const QNT_DERIVED: u8 = 1;
const QNT_EXPOUNDED: u8 = 2;

pub(crate) fn be16(p: &[u8]) -> u16 {
    u16::from_be_bytes([p[0], p[1]])
}

pub(crate) fn be32(p: &[u8]) -> u32 {
    u32::from_be_bytes([p[0], p[1], p[2], p[3]])
}

/// One component of the SIZ marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizComponent {
    /// Bit depth, 1..=31 (OpenJPEG's range; the standard allows up to 38).
    pub prec: u8,
    pub signed: bool,
    /// Horizontal and vertical sub-sampling, 1..=255.
    pub dx: u8,
    pub dy: u8,
}

/// What one COD or COC marker says about a tile-component (SPcod, SPcoc).
#[derive(Debug, Clone, Copy)]
pub(crate) struct CodingStyle {
    /// COD: `Scod & 1`; COC: `Scoc` as read (OpenJPEG's `tccp->csty`).
    pub csty: u8,
    /// 1..=33
    pub numresolutions: u8,
    /// Code-block exponents, 2..=10, their sum at most 12.
    pub cblkw: u8,
    pub cblkh: u8,
    pub cblksty: u8,
    /// 0: 9/7 irreversible, 1: 5/3 reversible.
    pub qmfbid: u8,
    /// `PPx | PPy << 4` per resolution; 0xFF when not given.
    pub precincts: [u8; MAX_RESOLUTIONS],
}

/// The tile-wide part of a COD marker (Scod, SGcod).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TileCoding {
    /// Scod: precincts, SOP, EPH.
    pub csty: u8,
    /// Progression order, 0..=4.
    pub prg: u8,
    /// 1..=65535
    pub numlayers: u16,
    /// 0 or 1.
    pub mct: u8,
}

/// What one QCD or QCC marker says (SQcd, SQcc).
#[derive(Debug, Clone)]
pub(crate) struct Quantization {
    /// None, derived or expounded.
    pub qntsty: u8,
    pub numgbits: u8,
    /// Step sizes given, at most `MAX_BANDS` (more are ignored, as OpenJPEG does).
    pub nbands: u8,
    /// `exponent << 11 | mantissa`; "none" has mantissa 0.
    pub steps: [u16; MAX_BANDS],
}

impl Quantization {
    /// A band's exponent and mantissa, as OpenJPEG derives them (derived:
    /// from band 0).
    pub fn step(&self, band: u32) -> (u32, u32) {
        if self.qntsty == QNT_DERIVED {
            let e0 = i32::from(self.steps[0] >> 11);
            let e = if band == 0 { e0 } else { e0 - ((band - 1) / 3) as i32 };
            return (e.max(0) as u32, u32::from(self.steps[0] & 0x7FF));
        }
        if band < u32::from(self.nbands) {
            let s = self.steps[band as usize];
            (u32::from(s >> 11), u32::from(s & 0x7FF))
        } else {
            (0, 0)
        }
    }
}

/// One progression order change (POC), as read; bounds are applied where it
/// is used.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PocEntry {
    pub compno0: u16,
    pub compno1: u16,
    pub layno1: u16,
    pub resno0: u8,
    pub resno1: u8,
    pub prg: u8,
}

/// Packed packet headers of one PPM or PPT marker, as read (Zppm or Zppt,
/// and the bytes after it).
#[derive(Debug, Clone)]
pub(crate) struct PackedChunk {
    pub z: u8,
    pub data: Vec<u8>,
}

/// The markers of one tile's headers (over all its tile-parts). Only tiles
/// whose headers carry COD, COC, QCD, QCC, RGN, POC or PPT get one. The COC,
/// QCC and RGN markers are kept in the order they came in: when a component
/// has several, the last one counts.
#[derive(Debug, Default)]
pub(crate) struct TileOverrides {
    pub cod: Option<(TileCoding, CodingStyle)>,
    pub qcd: Option<Quantization>,
    pub coc: Vec<(u16, CodingStyle)>,
    pub qcc: Vec<(u16, Quantization)>,
    pub rgn: Vec<(u16, u8)>,
    pub poc: Vec<PocEntry>,
    pub ppt: Vec<PackedChunk>,
}

/// The main header of a codestream.
#[derive(Debug)]
pub struct MainHeader {
    /// The image area on the reference grid.
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    /// Tile size and tile grid origin.
    pub(crate) tdx: u32,
    pub(crate) tdy: u32,
    pub(crate) tx0: u32,
    pub(crate) ty0: u32,
    /// Tiles across and down.
    pub(crate) tw: u32,
    pub(crate) th: u32,
    pub comps: Vec<SizComponent>,
    // COD and QCD (both required) and the main header's per-component overrides
    pub(crate) cod: TileCoding,
    pub(crate) cod_style: CodingStyle,
    pub(crate) qcd: Quantization,
    pub(crate) coc: Vec<Option<Box<CodingStyle>>>,
    pub(crate) qcc: Vec<Option<Box<Quantization>>>,
    /// Per component: roishift, or -1 when there is no main RGN.
    pub(crate) rgn: Vec<i16>,
    pub(crate) poc: Vec<PocEntry>,
    /// PPM: the packed packet headers in Zppm order, Nppm fields removed.
    pub(crate) has_ppm: bool,
    pub(crate) ppm_data: Vec<u8>,
    /// The fewest resolutions of any component, from the main header.
    pub(crate) min_resolutions: u32,
    /// Tiles with markers of their own (filled by the decode's walk).
    pub(crate) tiles: BTreeMap<u16, TileOverrides>,
}

/// A tile-component's parameters with precedence applied.
#[derive(Clone, Copy)]
pub(crate) struct TileComponentParams<'a> {
    pub coding: &'a TileCoding,
    pub style: &'a CodingStyle,
    pub quant: &'a Quantization,
    pub roishift: u8,
}

impl MainHeader {
    pub(crate) fn numcomps(&self) -> u32 {
        self.comps.len() as u32
    }

    pub(crate) fn tile_count(&self) -> u32 {
        self.tw * self.th
    }

    pub(crate) fn overrides(&self, tile: u32) -> Option<&TileOverrides> {
        u16::try_from(tile).ok().and_then(|t| self.tiles.get(&t))
    }

    fn main_style(&self, comp: usize) -> &CodingStyle {
        self.coc[comp].as_deref().unwrap_or(&self.cod_style)
    }

    fn main_quant(&self, comp: usize) -> &Quantization {
        self.qcc[comp].as_deref().unwrap_or(&self.qcd)
    }

    fn main_roishift(&self, comp: usize) -> u8 {
        if self.rgn[comp] >= 0 { self.rgn[comp] as u8 } else { 0 }
    }

    /// Number of wavelet decomposition levels every component has at least:
    /// the largest reduce factor a decode accepts.
    pub fn max_reduce(&self) -> u8 {
        (self.min_resolutions.saturating_sub(1)).min(32) as u8
    }
}

// ---- marker segments -------------------------------------------------------------------------

/// SPcod / SPcoc: the coding style of a tile-component. `cs.csty` is set by
/// the caller. Returns the bytes read; the caller checks the segment ends
/// there.
fn read_spcod(p: &[u8], cs: &mut CodingStyle) -> Result<usize, Error> {
    if p.len() < 5 {
        return Err(Error::Header("Error reading SPCod SPCoc element"));
    }
    let numres = usize::from(p[0]) + 1;
    if numres > MAX_RESOLUTIONS {
        return Err(Error::Header("Invalid value for numresolutions"));
    }
    let (cblkw, cblkh) = (u32::from(p[1]) + 2, u32::from(p[2]) + 2);
    if cblkw > 10 || cblkh > 10 || cblkw + cblkh > 12 {
        return Err(Error::Header("Invalid cblkw/cblkh combination"));
    }
    if p[3] & CBLK_HT_MIXED != 0 {
        return Err(Error::Unsupported("Mixed HT code-block style"));
    }
    if p[4] > 1 {
        return Err(Error::Header("Invalid transformation found"));
    }
    cs.numresolutions = numres as u8;
    cs.cblkw = cblkw as u8;
    cs.cblkh = cblkh as u8;
    cs.cblksty = p[3];
    cs.qmfbid = p[4];
    let mut used = 5;
    if cs.csty & CSTY_PRECINCTS != 0 {
        if p.len() - 5 < numres {
            return Err(Error::Header("Error reading SPCod SPCoc element"));
        }
        for r in 0..numres {
            let v = p[5 + r];
            // precinct exponent 0 is allowed at the lowest resolution only
            // (ISO 15444-1 Table A.21)
            if r != 0 && ((v & 0xF) == 0 || (v >> 4) == 0) {
                return Err(Error::Header("Invalid precinct size"));
            }
            cs.precincts[r] = v;
        }
        used += numres;
    } else {
        cs.precincts = [0xFF; MAX_RESOLUTIONS];
    }
    Ok(used)
}

fn empty_style(csty: u8) -> CodingStyle {
    CodingStyle {
        csty,
        numresolutions: 0,
        cblkw: 0,
        cblkh: 0,
        cblksty: 0,
        qmfbid: 0,
        precincts: [0xFF; MAX_RESOLUTIONS],
    }
}

/// COD: the tile-wide coding style and the default style of every component.
fn read_cod(limits: &Limits, p: &[u8]) -> Result<(TileCoding, CodingStyle), Error> {
    if p.len() < 5 {
        return Err(Error::Header("Error reading COD marker"));
    }
    if p[0] & !(CSTY_PRECINCTS | CSTY_SOP | CSTY_EPH) != 0 {
        return Err(Error::Header("Unknown Scod value in COD marker"));
    }
    if p[1] > 4 {
        return Err(Error::Header("Unknown progression order in COD marker"));
    }
    let layers = u32::from(be16(&p[2..]));
    if layers == 0 {
        return Err(Error::Header("Invalid number of layers in COD marker"));
    }
    if limits.max_layers != 0 && layers > limits.max_layers {
        return Err(Error::Limit("layers of a COD marker above the decoder's limit"));
    }
    if p[4] > 1 {
        return Err(Error::Header("Invalid multiple component transformation"));
    }
    let tc = TileCoding {
        csty: p[0],
        prg: p[1],
        numlayers: layers as u16,
        mct: p[4],
    };
    let mut cs = empty_style(tc.csty & CSTY_PRECINCTS);
    let used = read_spcod(&p[5..], &mut cs)?;
    if 5 + used != p.len() {
        return Err(Error::Header("Error reading COD marker"));
    }
    Ok((tc, cs))
}

fn component_room(numcomps: u32) -> usize {
    if numcomps <= 256 { 1 } else { 2 }
}

fn read_component(p: &[u8], room: usize) -> u32 {
    if room == 1 { u32::from(p[0]) } else { u32::from(be16(p)) }
}

/// COC: one component's coding style.
fn read_coc(numcomps: u32, p: &[u8]) -> Result<(u32, CodingStyle), Error> {
    let room = component_room(numcomps);
    if p.len() < room + 1 {
        return Err(Error::Header("Error reading COC marker"));
    }
    let comp = read_component(p, room);
    if comp >= numcomps {
        return Err(Error::Header("Error reading COC marker (bad number of components)"));
    }
    let mut cs = empty_style(p[room]);
    let used = read_spcod(&p[room + 1..], &mut cs)?;
    if room + 1 + used != p.len() {
        return Err(Error::Header("Error reading COC marker"));
    }
    Ok((comp, cs))
}

/// SQcd / SQcc. The whole remainder of the segment must be used.
fn read_sqcd(p: &[u8]) -> Result<Quantization, Error> {
    if p.is_empty() {
        return Err(Error::Header("Error reading SQcd or SQcc element"));
    }
    let mut out = Quantization {
        qntsty: p[0] & 0x1F,
        numgbits: p[0] >> 5,
        nbands: 0,
        steps: [0; MAX_BANDS],
    };
    let rest = p.len() - 1;
    if out.qntsty == QNT_DERIVED {
        if rest != 2 {
            return Err(Error::Header("Error reading QCD or QCC marker"));
        }
        out.nbands = 1;
        out.steps[0] = be16(&p[1..]);
    } else if out.qntsty == QNT_NONE || out.qntsty == QNT_EXPOUNDED {
        let width = if out.qntsty == QNT_NONE { 1 } else { 2 };
        if rest % width != 0 {
            return Err(Error::Header("Error reading QCD or QCC marker"));
        }
        // more sub-bands than OPJ_J2K_MAXBANDS: the rest is skipped, as OpenJPEG does
        let bands = (rest / width).min(MAX_BANDS);
        out.nbands = bands as u8;
        for b in 0..bands {
            out.steps[b] = if width == 1 {
                u16::from(p[1 + b] >> 3) << 11
            } else {
                be16(&p[1 + 2 * b..])
            };
        }
    } else {
        // reserved styles; OpenJPEG reads them as expounded (departure DP-8)
        return Err(Error::Header("Unknown quantization style"));
    }
    Ok(out)
}

fn read_qcc(numcomps: u32, p: &[u8]) -> Result<(u32, Quantization), Error> {
    let room = component_room(numcomps);
    if p.len() < room {
        return Err(Error::Header("Error reading QCC marker"));
    }
    let comp = read_component(p, room);
    if comp >= numcomps {
        return Err(Error::Header("Invalid component number in QCC marker"));
    }
    Ok((comp, read_sqcd(&p[room..])?))
}

fn read_rgn(numcomps: u32, p: &[u8]) -> Result<(u32, u8), Error> {
    let room = component_room(numcomps);
    if p.len() != 2 + room {
        return Err(Error::Header("Error reading RGN marker"));
    }
    let comp = read_component(p, room);
    if comp >= numcomps {
        return Err(Error::Header("bad component number in RGN"));
    }
    // a region of interest style other than 0 is read as implicit
    Ok((comp, p[room + 1]))
}

/// POC entries, appended to `list`. `existing` is what OpenJPEG counts
/// against its limit of 31 changes.
fn read_poc(numcomps: u32, p: &[u8], existing: usize, list: &mut Vec<PocEntry>) -> Result<(), Error> {
    let room = component_room(numcomps);
    let chunk = 5 + 2 * room;
    let count = p.len() / chunk;
    if count == 0 || p.len() % chunk != 0 {
        return Err(Error::Header("Error reading POC marker"));
    }
    if existing + count >= MAX_POCS {
        return Err(Error::Header("Too many POCs"));
    }
    for e in p.chunks_exact(chunk) {
        list.push(PocEntry {
            resno0: e[0],
            compno0: read_component(&e[1..], room) as u16,
            layno1: be16(&e[1 + room..]),
            resno1: e[3 + room],
            compno1: read_component(&e[4 + room..], room) as u16,
            prg: e[4 + 2 * room],
        });
    }
    Ok(())
}

/// A Zppm / Zppt chunk. Each index may come once (OpenJPEG: "Zppm %u already
/// read").
fn add_packed(p: &[u8], list: &mut Vec<PackedChunk>) -> Result<(), Error> {
    // the index and at least one byte of packed headers
    if p.len() < 2 {
        return Err(Error::Header("Error reading PPM or PPT marker"));
    }
    if list.iter().any(|c| c.z == p[0]) {
        return Err(Error::Header("Zppm or Zppt already read"));
    }
    list.push(PackedChunk {
        z: p[0],
        data: p[1..].to_vec(),
    });
    Ok(())
}

/// State of the TLM markers read so far: only what the reader's checks need
/// (the decode walks the tile-parts in codestream order and never uses the
/// index).
#[derive(Default)]
struct Tlm {
    invalid: bool,
    entries: u32,
}

/// TLM, as OpenJPEG reads it: an invalid one is ignored, never an error.
fn read_tlm(tlm: &mut Tlm, tile_count: u32, p: &[u8]) -> Result<(), Error> {
    if p.len() < 2 {
        return Err(Error::Header("Error reading TLM marker"));
    }
    if tlm.invalid {
        return Ok(());
    }
    let st = usize::from((p[1] >> 4) & 0x3);
    let sp = usize::from((p[1] >> 6) & 0x1);
    if st == 3 {
        tlm.invalid = true;
        return Ok(());
    }
    let psize = (sp + 1) * 2;
    let entry = psize + st;
    let body = &p[2..];
    if body.len() % entry != 0 {
        tlm.invalid = true;
        return Ok(());
    }
    for q in body.chunks_exact(entry) {
        let tile = match st {
            0 => tlm.entries,
            1 => u32::from(q[0]),
            _ => u32::from(be16(q)),
        };
        if tile >= tile_count {
            tlm.invalid = true;
            return Ok(());
        }
        tlm.entries += 1;
    }
    Ok(())
}

/// What SIZ gives, before the rest of the main header.
struct Siz {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    tdx: u32,
    tdy: u32,
    tx0: u32,
    ty0: u32,
    tw: u32,
    th: u32,
    comps: Vec<SizComponent>,
}

/// SIZ: the image, the tile grid and the components.
fn read_siz(limits: &Limits, p: &[u8]) -> Result<Siz, Error> {
    let n = p.len();
    if n < 36 || (n - 36) % 3 != 0 {
        return Err(Error::Header("Error with SIZ marker size"));
    }
    let (x1, y1, x0, y0) = (be32(&p[2..]), be32(&p[6..]), be32(&p[10..]), be32(&p[14..]));
    let (tdx, tdy, tx0, ty0) = (be32(&p[18..]), be32(&p[22..]), be32(&p[26..]), be32(&p[30..]));
    let csiz = u32::from(be16(&p[34..]));
    if csiz == 0 || csiz > MAX_COMPONENTS {
        return Err(Error::Header("SIZ marker: illegal number of components"));
    }
    if csiz as usize != (n - 36) / 3 {
        return Err(Error::Header("SIZ marker: number of components not compatible with its size"));
    }
    if limits.max_components != 0 && csiz > limits.max_components {
        return Err(Error::Limit("components above the decoder's limit"));
    }
    if x0 >= x1 || y0 >= y1 {
        return Err(Error::Header("SIZ marker: negative or zero image size"));
    }
    if tdx == 0 || tdy == 0 {
        return Err(Error::Header("SIZ marker: invalid tile size"));
    }
    if tx0 > x0 || ty0 > y0 || u64::from(tx0) + u64::from(tdx) <= u64::from(x0) || u64::from(ty0) + u64::from(tdy) <= u64::from(y0) {
        return Err(Error::Header("SIZ marker: illegal tile offset"));
    }
    let mut comps = Vec::with_capacity(csiz as usize);
    for c in p[36..].chunks_exact(3) {
        let (prec, dx, dy) = ((c[0] & 0x7F) + 1, c[1], c[2]);
        if dx < 1 || dy < 1 {
            return Err(Error::Header("SIZ marker: invalid component sub-sampling"));
        }
        if prec > 31 {
            return Err(Error::Header("SIZ marker: component precision above 31 bits"));
        }
        comps.push(SizComponent {
            prec,
            signed: c[0] >> 7 != 0,
            dx,
            dy,
        });
    }
    let tw = (u64::from(x1) - u64::from(tx0)).div_ceil(u64::from(tdx));
    let th = (u64::from(y1) - u64::from(ty0)).div_ceil(u64::from(tdy));
    if tw == 0 || th == 0 || tw > MAX_TILES / th {
        return Err(Error::Header("Invalid number of tiles"));
    }
    if limits.max_tiles != 0 && tw * th > u64::from(limits.max_tiles) {
        return Err(Error::Limit("tiles above the decoder's limit"));
    }
    Ok(Siz {
        x0,
        y0,
        x1,
        y1,
        tdx,
        tdy,
        tx0,
        ty0,
        tw: tw as u32,
        th: th as u32,
        comps,
    })
}

/// Nppm: the packed headers of each tile-part in turn, over the PPM markers
/// in Zppm order (OpenJPEG's `opj_j2k_merge_ppm`, with its checks). The Nppm
/// fields are removed: the packet headers of the tile-parts follow each
/// other, and Tier-2 consumes them across the tiles.
fn merge_ppm(chunks: &mut [PackedChunk]) -> Result<Vec<u8>, Error> {
    chunks.sort_by_key(|c| c.z);
    let mut merged = Vec::new();
    let mut declared: u64 = 0;
    let mut remaining: u64 = 0;
    for c in chunks.iter() {
        let mut d = &c.data[..];
        let carry = remaining.min(d.len() as u64) as usize;
        merged.extend_from_slice(&d[..carry]);
        remaining -= carry as u64;
        d = &d[carry..];
        while !d.is_empty() {
            if d.len() < 4 {
                return Err(Error::Header("Not enough bytes to read Nppm"));
            }
            let nppm = u64::from(be32(d));
            d = &d[4..];
            if declared > 0xFFFF_FFFF - nppm {
                return Err(Error::Header("Too large value for Nppm"));
            }
            declared += nppm;
            let take = nppm.min(d.len() as u64) as usize;
            merged.extend_from_slice(&d[..take]);
            d = &d[take..];
            remaining = nppm - take as u64;
        }
    }
    if remaining != 0 {
        return Err(Error::Header("Corrupted PPM markers"));
    }
    // Packed packet headers that hold no byte at all: OpenJPEG refuses them
    // too (its opj_malloc(0) gives NULL)
    if declared == 0 {
        return Err(Error::Header("Corrupted PPM markers: the packet headers hold no bytes"));
    }
    Ok(merged)
}

/// PLT, as OpenJPEG checks it: the packet lengths must end with a complete
/// length.
fn check_plt(p: &[u8]) -> Result<(), Error> {
    if p.is_empty() {
        return Err(Error::Header("Error reading PLT marker"));
    }
    let mut length = 0u32;
    for &b in &p[1..] {
        length |= u32::from(b & 0x7F);
        if b & 0x80 != 0 {
            length <<= 7;
        } else {
            length = 0;
        }
    }
    if length != 0 {
        return Err(Error::Header("Error reading PLT marker"));
    }
    Ok(())
}

/// A component's parameters from the main header (main COC > main COD, main
/// QCC > main QCD), then the tile's own COD and QCD over them (tile-part COD
/// > main COC, tile-part QCD > main QCC).
fn component_defaults<'a>(mh: &'a MainHeader, tile: Option<&'a TileOverrides>, comp: usize) -> TileComponentParams<'a> {
    let mut p = TileComponentParams {
        coding: &mh.cod,
        style: mh.main_style(comp),
        quant: mh.main_quant(comp),
        roishift: mh.main_roishift(comp),
    };
    if let Some(t) = tile {
        if let Some((coding, style)) = &t.cod {
            p.coding = coding;
            p.style = style;
        }
        if let Some(q) = &t.qcd {
            p.quant = q;
        }
    }
    p
}

/// Every sub-band of every resolution needs its step size; derived
/// quantization has one (departure DP-6).
fn check_step_count(p: &TileComponentParams<'_>) -> Result<(), Error> {
    if p.quant.qntsty == QNT_DERIVED {
        return Ok(());
    }
    let bands = 3 * u32::from(p.style.numresolutions) - 2;
    if u32::from(p.quant.nbands) >= bands {
        return Ok(());
    }
    Err(Error::Header(
        "the quantization gives fewer step sizes than the component has sub-bands",
    ))
}

/// Reads the main header, from SOC to the first SOT marker (which is
/// consumed). Returns the header and the offset just after that SOT marker.
pub(crate) fn read_main_header(data: &[u8], limits: &Limits) -> Result<(MainHeader, usize), Error> {
    if data.is_empty() {
        return Err(Error::NeedData("the main header (no bytes)"));
    }
    if data[0] != 0xFF || (data.len() >= 2 && data[1] != 0x4F) {
        return Err(Error::NotJpeg2000);
    }
    if data.len() == 1 {
        return Err(Error::NeedData("the SOC marker"));
    }
    let mut pos = 2usize;
    let mut siz: Option<Siz> = None;
    let mut cod: Option<(TileCoding, CodingStyle)> = None;
    let mut qcd: Option<Quantization> = None;
    let mut coc: Vec<Option<Box<CodingStyle>>> = Vec::new();
    let mut qcc: Vec<Option<Box<Quantization>>> = Vec::new();
    let mut rgn: Vec<i16> = Vec::new();
    let mut poc: Vec<PocEntry> = Vec::new();
    let mut ppm_chunks: Vec<PackedChunk> = Vec::new();
    let mut has_ppm = false;
    let mut tlm = Tlm::default();
    loop {
        let Some(b) = data.get(pos..pos + 2) else {
            return Err(Error::NeedData("the main header"));
        };
        pos += 2;
        let m = be16(b);
        if m < 0xFF00 {
            return Err(Error::Header("A marker was expected in the main header"));
        }
        if siz.is_none() && m != mk::SIZ {
            // ISO 15444-1 A.5: SIZ follows SOC (OpenJPEG skips unknown markers to find it)
            return Err(Error::Header("Expected a SIZ marker after SOC"));
        }
        if m == mk::SOT {
            break;
        }
        if segmentless(u32::from(m)) {
            if (0xFF30..=0xFF3F).contains(&m) {
                continue;
            }
            return Err(Error::Header("Marker is not compliant with its position (main header)"));
        }
        let Some(b) = data.get(pos..pos + 2) else {
            return Err(Error::NeedData("the main header"));
        };
        pos += 2;
        let length = usize::from(be16(b));
        if length < 2 {
            return Err(Error::Header("Invalid marker size"));
        }
        if part2_marker(u32::from(m)) {
            return Err(Error::Unsupported("JPEG 2000 Part 2 marker"));
        }
        let n = length - 2;
        let Some(body) = data.get(pos..pos + n) else {
            return Err(Error::NeedData("the main header"));
        };
        pos += n;
        let numcomps = siz.as_ref().map_or(0, |s| s.comps.len() as u32);
        match m {
            mk::SIZ => {
                if siz.is_some() {
                    return Err(Error::Header("A second SIZ marker in the main header"));
                }
                let s = read_siz(limits, body)?;
                coc = vec![None; s.comps.len()];
                qcc = vec![None; s.comps.len()];
                rgn = vec![-1; s.comps.len()];
                siz = Some(s);
            }
            mk::COD => cod = Some(read_cod(limits, body)?),
            mk::COC => {
                let (comp, cs) = read_coc(numcomps, body)?;
                coc[comp as usize] = Some(Box::new(cs));
            }
            mk::QCD => qcd = Some(read_sqcd(body)?),
            mk::QCC => {
                let (comp, q) = read_qcc(numcomps, body)?;
                qcc[comp as usize] = Some(Box::new(q));
            }
            mk::RGN => {
                let (comp, shift) = read_rgn(numcomps, body)?;
                rgn[comp as usize] = i16::from(shift);
            }
            mk::POC => {
                let existing = poc.len();
                read_poc(numcomps, body, existing, &mut poc)?;
            }
            mk::PPM => {
                has_ppm = true;
                add_packed(body, &mut ppm_chunks)?;
            }
            mk::TLM => {
                let tiles = siz.as_ref().map_or(0, |s| s.tw * s.th);
                read_tlm(&mut tlm, tiles, body)?;
            }
            mk::PLM => {
                if n < 1 {
                    return Err(Error::Header("Error reading PLM marker"));
                }
            }
            mk::CRG => {
                if n != numcomps as usize * 4 {
                    return Err(Error::Header("Error reading CRG marker"));
                }
            }
            mk::CAP | mk::COM | mk::CPF => {}
            mk::PLT | mk::PPT | mk::SOP => {
                return Err(Error::Header("Marker is not compliant with its position"));
            }
            // ISO 15444-1 A.1: a decoder skips marker segments it does not
            // know, by their length (OpenJPEG scans for the next marker it
            // knows instead)
            _ => {}
        }
    }
    let Some(siz) = siz else {
        return Err(Error::Header("Expected a SIZ marker after SOC"));
    };
    let Some((cod, cod_style)) = cod else {
        return Err(Error::Header("required COD marker not found in main header"));
    };
    let Some(qcd) = qcd else {
        return Err(Error::Header("required QCD marker not found in main header"));
    };
    let ppm_data = if has_ppm { merge_ppm(&mut ppm_chunks)? } else { Vec::new() };
    let mut mh = MainHeader {
        x0: siz.x0,
        y0: siz.y0,
        x1: siz.x1,
        y1: siz.y1,
        tdx: siz.tdx,
        tdy: siz.tdy,
        tx0: siz.tx0,
        ty0: siz.ty0,
        tw: siz.tw,
        th: siz.th,
        comps: siz.comps,
        cod,
        cod_style,
        qcd,
        coc,
        qcc,
        rgn,
        poc,
        has_ppm,
        ppm_data,
        min_resolutions: MAX_RESOLUTIONS as u32,
        tiles: BTreeMap::new(),
    };
    // the main header's defaults of every component: precedence and step counts
    let mut min_resolutions = MAX_RESOLUTIONS as u32;
    for c in 0..mh.comps.len() {
        let params = resolve_tile_component(&mh, None, c)?;
        min_resolutions = min_resolutions.min(u32::from(params.style.numresolutions));
    }
    mh.min_resolutions = min_resolutions;
    Ok((mh, pos))
}

/// One marker segment of a tile-part header (its marker code `m` and its
/// body), recorded in `mh.tiles` for the tile or checked: what may not appear
/// there is refused, a marker segment the decoder does not know is skipped
/// (departure DP-7). The decode's walk over the tile-parts calls it for each
/// marker it reads.
pub(crate) fn read_tile_part_marker(limits: &Limits, mh: &mut MainHeader, tile: u16, m: u16, body: &[u8]) -> Result<(), Error> {
    let numcomps = mh.numcomps();
    let main_pocs = mh.poc.len();
    let has_ppm = mh.has_ppm;
    match m {
        mk::COD => mh.tiles.entry(tile).or_default().cod = Some(read_cod(limits, body)?),
        mk::COC => {
            let (comp, cs) = read_coc(numcomps, body)?;
            mh.tiles.entry(tile).or_default().coc.push((comp as u16, cs));
        }
        mk::QCD => mh.tiles.entry(tile).or_default().qcd = Some(read_sqcd(body)?),
        mk::QCC => {
            let (comp, q) = read_qcc(numcomps, body)?;
            mh.tiles.entry(tile).or_default().qcc.push((comp as u16, q));
        }
        mk::RGN => {
            let (comp, shift) = read_rgn(numcomps, body)?;
            mh.tiles.entry(tile).or_default().rgn.push((comp as u16, shift));
        }
        mk::POC => {
            // OpenJPEG's limit counts the main header's changes too
            let t = mh.tiles.entry(tile).or_default();
            let existing = main_pocs + t.poc.len();
            read_poc(numcomps, body, existing, &mut t.poc)?;
        }
        mk::PPT => {
            if has_ppm {
                return Err(Error::Header("PPT marker: packet headers were found in the main header (PPM)"));
            }
            add_packed(body, &mut mh.tiles.entry(tile).or_default().ppt)?;
        }
        mk::PLT => check_plt(body)?,
        mk::COM => {}
        mk::SOT | mk::SIZ | mk::TLM | mk::PLM | mk::PPM | mk::CRG | mk::CAP | mk::CPF | mk::SOP => {
            return Err(Error::Header("Marker is not compliant with its position"));
        }
        // skipped by its length (OpenJPEG fails a tile-part header with an unknown marker)
        _ => {}
    }
    Ok(())
}

/// Every component's parameters of a tile, with marker precedence applied
/// and each component's step sizes checked against the bands its resolutions
/// need. `tile` is `None` for the main header's defaults.
pub(crate) fn resolve_tile<'a>(mh: &'a MainHeader, tile: Option<&'a TileOverrides>) -> Result<Vec<TileComponentParams<'a>>, Error> {
    let mut out: Vec<_> = (0..mh.comps.len()).map(|c| component_defaults(mh, tile, c)).collect();
    if let Some(t) = tile {
        // in the order read, so the last marker for a component is the one that stays
        for (comp, style) in &t.coc {
            out[usize::from(*comp)].style = style;
        }
        for (comp, quant) in &t.qcc {
            out[usize::from(*comp)].quant = quant;
        }
        for (comp, shift) in &t.rgn {
            out[usize::from(*comp)].roishift = *shift;
        }
    }
    for p in &out {
        check_step_count(p)?;
    }
    Ok(out)
}

/// One tile-component's parameters, as [`resolve_tile`] gives them.
pub(crate) fn resolve_tile_component<'a>(
    mh: &'a MainHeader,
    tile: Option<&'a TileOverrides>,
    comp: usize,
) -> Result<TileComponentParams<'a>, Error> {
    let mut p = component_defaults(mh, tile, comp);
    if let Some(t) = tile {
        if let Some((_, style)) = t.coc.iter().rev().find(|(c, _)| usize::from(*c) == comp) {
            p.style = style;
        }
        if let Some((_, quant)) = t.qcc.iter().rev().find(|(c, _)| usize::from(*c) == comp) {
            p.quant = quant;
        }
        if let Some((_, shift)) = t.rgn.iter().rev().find(|(c, _)| usize::from(*c) == comp) {
            p.roishift = *shift;
        }
    }
    check_step_count(&p)?;
    Ok(p)
}

// ---- geometry, as OpenJPEG computes it -------------------------------------------------------

pub(crate) fn ceildiv(a: u32, b: u32) -> u32 {
    ((u64::from(a) + u64::from(b) - 1) / u64::from(b)) as u32
}

pub(crate) fn ceildivpow2(a: u32, e: u32) -> u32 {
    if e >= 32 {
        u32::from(a != 0)
    } else {
        ((u64::from(a) + (1u64 << e) - 1) >> e) as u32
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// The tile's area on the reference grid, clipped to the image.
pub(crate) fn tile_rect(mh: &MainHeader, tile: u32) -> Rect {
    let (p, q) = (u64::from(tile % mh.tw), u64::from(tile / mh.tw));
    let x0 = u64::from(mh.tx0) + p * u64::from(mh.tdx);
    let y0 = u64::from(mh.ty0) + q * u64::from(mh.tdy);
    Rect {
        x0: x0.max(u64::from(mh.x0)) as u32,
        y0: y0.max(u64::from(mh.y0)) as u32,
        x1: (x0 + u64::from(mh.tdx)).min(u64::from(mh.x1)) as u32,
        y1: (y0 + u64::from(mh.tdy)).min(u64::from(mh.y1)) as u32,
    }
}

/// Component geometry of the image a header read gives when the reduce
/// factor is set before it (OpenJPEG's `opj_image_comp_header_update`):
/// `(x0, y0, w, h)`, `w` and `h` the component's extent divided by
/// `2^factor`.
pub(crate) fn header_component_geometry(mh: &MainHeader, comp: usize, factor: u32) -> (u32, u32, u32, u32) {
    // the tile grid's area clipped to the image
    let gx0 = mh.tx0.max(mh.x0);
    let gy0 = mh.ty0.max(mh.y0);
    let ex1 = u64::from(mh.tx0) + u64::from(mh.tw) * u64::from(mh.tdx);
    let ey1 = u64::from(mh.ty0) + u64::from(mh.th) * u64::from(mh.tdy);
    let gx1 = ex1.min(u64::from(mh.x1)) as u32;
    let gy1 = ey1.min(u64::from(mh.y1)) as u32;
    let c = &mh.comps[comp];
    let (cx0, cy0) = (ceildiv(gx0, u32::from(c.dx)), ceildiv(gy0, u32::from(c.dy)));
    let (cx1, cy1) = (ceildiv(gx1, u32::from(c.dx)), ceildiv(gy1, u32::from(c.dy)));
    (cx0, cy0, ceildivpow2(cx1 - cx0, factor), ceildivpow2(cy1 - cy0, factor))
}

// ---- working memory --------------------------------------------------------------------------

// Bytes per item in the working-memory estimate, as neo-j2k measured them
// against what its decodes take from the allocator. The structures here are
// of about the same size; the estimate is a bound for the limit check, not
// an account.
const CODE_BLOCK_DECODED: u64 = 128;
const CODE_BLOCK_HEADER: u64 = 32;
const PRECINCT: u64 = 64;
const SCRATCH: u64 = 256 * 1024;
const LINE_BYTES: u64 = 64;
const ARENA_RESERVE: u64 = 2;

/// Cells of a `2^e` grid anchored at 0 that `[a, b)` meets, plus one: an
/// upper bound that does not depend on where the tile sits.
fn cells(a: u64, b: u64, e: u32) -> u64 {
    if a >= b {
        return 0;
    }
    ((b - a + (1u64 << e) - 1) >> e) + 1
}

/// Code-block and precinct state of one tile-component, over every
/// resolution; `most_precincts` becomes the most precincts of any of its
/// resolutions, if more.
fn structure_bytes(s: &CodingStyle, factor: u32, tc: (u32, u32, u32, u32), most_precincts: &mut u64) -> u64 {
    let (tcx0, tcy0, tcx1, tcy1) = tc;
    let mut bytes = 0u64;
    let levels = u32::from(s.numresolutions) - 1;
    for r in 0..=levels {
        let down = levels - r;
        let (rx0, ry0) = (u64::from(ceildivpow2(tcx0, down)), u64::from(ceildivpow2(tcy0, down)));
        let (rx1, ry1) = (u64::from(ceildivpow2(tcx1, down)), u64::from(ceildivpow2(tcy1, down)));
        let (ppx, ppy) = (u32::from(s.precincts[r as usize] & 0xF), u32::from(s.precincts[r as usize] >> 4));
        let precincts = cells(rx0, rx1, ppx).saturating_mul(cells(ry0, ry1, ppy));
        *most_precincts = (*most_precincts).max(precincts);
        bytes = bytes.saturating_add(precincts.saturating_mul(PRECINCT));
        // code-blocks never cross a precinct: above resolution 0 the bands are half the resolution
        let gx = if r != 0 { ppx.saturating_sub(1) } else { ppx };
        let gy = if r != 0 { ppy.saturating_sub(1) } else { ppy };
        let (cbw, cbh) = (u32::from(s.cblkw).min(gx), u32::from(s.cblkh).min(gy));
        let decoded = r + factor <= levels;
        let each = if decoded { CODE_BLOCK_DECODED } else { CODE_BLOCK_HEADER };
        if r == 0 {
            bytes = bytes.saturating_add(cells(rx0, rx1, cbw).saturating_mul(cells(ry0, ry1, cbh)).saturating_mul(each));
            continue;
        }
        // bands HL, LH, HH: each about half the resolution in each direction
        let (bw, bh) = ((rx1 - rx0).div_ceil(2) + 1, (ry1 - ry0).div_ceil(2) + 1);
        let per_band = cells(0, bw, cbw).saturating_mul(cells(0, bh, cbh));
        bytes = bytes.saturating_add(per_band.saturating_mul(3).saturating_mul(each));
    }
    bytes
}

/// One tile's part of the working memory, with each component's coding style
/// from `style(c)` and the tile's layers: the code-block and precinct state
/// of every resolution, the packet iterator's table of packets met, and the
/// tile's coefficients at the decoded level.
fn tile_part<'a>(mh: &MainHeader, t: Rect, style: impl Fn(usize) -> &'a CodingStyle, layers: u32, factor: u32, decoded: usize) -> u64 {
    let mut tile = 0u64;
    let mut most_precincts = 0u64;
    let mut most_resolutions = 0u32;
    for (c, sc) in mh.comps.iter().enumerate() {
        let (dx, dy) = (u32::from(sc.dx), u32::from(sc.dy));
        let tc = (ceildiv(t.x0, dx), ceildiv(t.y0, dy), ceildiv(t.x1, dx), ceildiv(t.y1, dy));
        let s = style(c);
        most_resolutions = most_resolutions.max(u32::from(s.numresolutions));
        tile = tile.saturating_add(structure_bytes(s, factor, tc, &mut most_precincts).saturating_mul(ARENA_RESERVE));
        if c < decoded {
            // the tile's coefficients at the decoded level, before they go to the image
            let w = u64::from(ceildivpow2(tc.2, factor) - ceildivpow2(tc.0, factor));
            let h = u64::from(ceildivpow2(tc.3, factor) - ceildivpow2(tc.1, factor));
            tile = tile.saturating_add(w.saturating_mul(h).saturating_mul(4));
        }
    }
    let table_bits = (u64::from(layers) + 1)
        .saturating_mul(u64::from(most_resolutions))
        .saturating_mul(u64::from(mh.numcomps()))
        .saturating_mul(most_precincts);
    tile.saturating_add((table_bits / 8 + 16).saturating_mul(ARENA_RESERVE))
}

/// The part of [`decode_working_memory`] that does not depend on the tiles:
/// the decoded image of the first `decoded` components and the wavelet's
/// lines and fixed scratch.
pub(crate) fn image_working_memory(mh: &MainHeader, factor: u32, decoded: usize) -> u64 {
    let mut total = SCRATCH;
    let mut longest = 1u64;
    for c in 0..mh.comps.len().min(decoded) {
        let (_, _, w, h) = header_component_geometry(mh, c, factor);
        let (w, h) = (u64::from(w), u64::from(h));
        total = total.saturating_add(w.saturating_mul(h).saturating_mul(4));
        longest = longest.max(w).max(h);
    }
    total.saturating_add(longest.saturating_mul(LINE_BYTES))
}

/// One tile's part of the working memory with its own parameters
/// ([`resolve_tile`]).
pub(crate) fn tile_working_memory(mh: &MainHeader, tile: u32, params: &[TileComponentParams<'_>], factor: u32, decoded: usize) -> u64 {
    tile_part(
        mh,
        tile_rect(mh, tile),
        |c| params[c].style,
        u32::from(params[0].coding.numlayers),
        factor,
        decoded,
    )
}

/// The memory a decode at this factor needs at most at once, computed from
/// the header before anything is allocated: the decoded samples of the first
/// `decoded` components, the wavelet's lines and fixed scratch, and the
/// largest tile's part with the main header's coding. Saturates at
/// `u64::MAX`. Tile-part headers can change a tile's coding; decoding checks
/// such a tile again with its own parameters before it allocates the tile.
pub(crate) fn decode_working_memory(mh: &MainHeader, factor: u32, decoded: usize) -> u64 {
    // the largest tile with the main header's coding: the first, an inner
    // and the last column and row of the grid, each once
    let mut cols = vec![0];
    let mut rows = vec![0];
    if mh.tw > 2 {
        cols.push(1);
    }
    if mh.tw > 1 {
        cols.push(mh.tw - 1);
    }
    if mh.th > 2 {
        rows.push(1);
    }
    if mh.th > 1 {
        rows.push(mh.th - 1);
    }
    let mut largest = 0u64;
    for &q in &rows {
        for &p in &cols {
            let tile = tile_part(
                mh,
                tile_rect(mh, q * mh.tw + p),
                |c| mh.main_style(c),
                u32::from(mh.cod.numlayers),
                factor,
                decoded,
            );
            largest = largest.max(tile);
        }
    }
    image_working_memory(mh, factor, decoded).saturating_add(largest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_divisions_match_openjpeg() {
        assert_eq!(ceildiv(0, 3), 0);
        assert_eq!(ceildiv(7, 3), 3);
        assert_eq!(ceildiv(u32::MAX, 1), u32::MAX);
        assert_eq!(ceildivpow2(0, 5), 0);
        assert_eq!(ceildivpow2(33, 5), 2);
        assert_eq!(ceildivpow2(u32::MAX, 31), 2);
        assert_eq!(ceildivpow2(5, 32), 1);
        assert_eq!(ceildivpow2(0, 40), 0);
    }

    #[test]
    fn derived_quantization_steps() {
        let mut q = Quantization {
            qntsty: QNT_DERIVED,
            numgbits: 2,
            nbands: 1,
            steps: [0; MAX_BANDS],
        };
        q.steps[0] = (9 << 11) | 0x123;
        assert_eq!(q.step(0), (9, 0x123));
        assert_eq!(q.step(1), (9, 0x123));
        assert_eq!(q.step(4), (8, 0x123));
        assert_eq!(q.step(40), (0, 0x123));
    }

    #[test]
    fn plt_must_end_with_a_complete_length() {
        assert!(check_plt(&[0, 0x05]).is_ok());
        assert!(check_plt(&[0, 0x85, 0x01]).is_ok());
        assert!(check_plt(&[0, 0x85]).is_err());
        assert!(check_plt(&[]).is_err());
    }

    #[test]
    fn main_header_needs_soc_then_siz() {
        let limits = Limits::NONE;
        assert_eq!(
            read_main_header(&[], &limits).err(),
            Some(Error::NeedData("the main header (no bytes)"))
        );
        assert_eq!(read_main_header(&[0x00, 0x01], &limits).err(), Some(Error::NotJpeg2000));
        assert_eq!(read_main_header(&[0xFF], &limits).err(), Some(Error::NeedData("the SOC marker")));
        // COD before SIZ
        let cs = [0xFF, 0x4F, 0xFF, 0x52, 0x00, 0x02];
        assert!(matches!(read_main_header(&cs, &limits), Err(Error::Header(_))));
    }
}
