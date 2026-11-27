// This file is a port and keeps the notices of what it ports (both under the
// BSD 2-Clause License, whose text NOTICE.md gives, "JPEG 2000 decoder").
//
// neo-j2k, `src/decode.cpp`:
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

//! The decode of a codestream after its main header: the walk over the
//! tile-parts, each tile's decode (`tile.rs`) and the copy of its samples to
//! the image.
//!
//! Port of neo-j2k's `src/decode.cpp` (BSD-2-Clause, Copyright (c) 2026
//! Anne Skydancer), itself after OpenJPEG 2.5.4's `j2k.c`
//! (`opj_j2k_decode_tiles`, `opj_j2k_read_tile_header`, `opj_j2k_read_sot`,
//! `opj_j2k_need_nb_tile_parts_correction`, `opj_j2k_read_sod`,
//! `opj_j2k_find_final_eoc`, `opj_j2k_decode_tile`,
//! `opj_j2k_update_image_data`; BSD-2-Clause, see NOTICE.md), as one state
//! machine with the same states, checks and outcomes for whole and cut
//! codestreams. The codestream is in memory, read as a stream that knows
//! its length and can seek, and decoded in OpenJPEG's non-strict mode (what
//! arrived of a cut codestream is decoded), which is how the viewer has
//! always read its textures.
//!
//! From neo-j2k (its departure DP-3): a tile whose tile-parts arrived
//! without data bytes, its packet headers in PPM or PPT, decodes as the
//! standard says (OpenJPEG refuses it); a tile that never arrived, or one
//! with neither data nor packed packet headers, is still not decoded.

use std::collections::BTreeMap;

use crate::header::{
    self, MainHeader, be16, be32, ceildivpow2, mk, part2_marker, read_tile_part_marker, resolve_tile, resolve_tile_component, segmentless,
};
use crate::mct::{self, Mct, Region};
use crate::par::Threads;
use crate::t2::{self, PackedCursor, PacketHeaders, PacketInput};
use crate::tile::{SampleWork, Tile};
use crate::{Component, Error, Limits, Samples, Scratch};

// OpenJPEG's J2K_STATE_* bits
const TPHSOT: u32 = 0x0008;
const TPH: u32 = 0x0010;
const NEOC: u32 = 0x0040;
const DATA: u32 = 0x0080;
const EOC: u32 = 0x0100;

/// `opj_stream_read_data`'s `(OPJ_SIZE_T)-1`: nothing is left.
const END: usize = usize::MAX;

/// A tile's walk state (OpenJPEG keeps it in its `opj_tcp_t`), made when
/// the tile first appears.
struct TileWalk {
    /// -1 before any.
    current_tile_part_number: i32,
    nb_tile_parts: u32,
    /// A tile-part reached its SOD since the tile was last decoded.
    seen: bool,
    /// OpenJPEG's `m_data != NULL`: a tile-part gave it bytes.
    has_data: bool,
    /// The bytes ended before a tile-part's declared length.
    cut: bool,
    ppt_merged: bool,
    /// Where its tile-parts' data lies in the codestream.
    ranges: Vec<(usize, usize)>,
}

impl Default for TileWalk {
    fn default() -> Self {
        TileWalk {
            current_tile_part_number: -1,
            nb_tile_parts: 0,
            seen: false,
            has_data: false,
            cut: false,
            ppt_merged: false,
            ranges: Vec::new(),
        }
    }
}

/// A component of the decoded image, as `opj_image_comp_t` carries it
/// through the decode.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutComp {
    pub x0: u32,
    pub y0: u32,
    pub w: u32,
    pub h: u32,
    /// A tile wrote it (`opj_j2k_are_all_used_components_decoded`).
    pub written: bool,
}

/// What a decode gives beyond its samples.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Outcome {
    /// The codestream ended before its end.
    pub truncated: bool,
    /// See `t2::PacketStats::cut_in_decoded`.
    pub cut_in_decoded: bool,
}

/// What the walk decodes into.
pub(crate) struct Output<'a> {
    pub comps: &'a mut [OutComp],
    pub samples: &'a mut Samples,
    /// Pixels across the packed image (`Samples::Packed`).
    pub width: usize,
}

struct Walk<'a, 'o> {
    data: &'a [u8],
    threads: Threads,
    mh: &'a mut MainHeader,
    limits: &'a Limits,
    /// The reduce factor in effect.
    factor: u32,
    /// Components whose samples are decoded: the first ones.
    decoded: usize,
    out: Output<'o>,
    tile: &'a mut Tile,
    scratch: &'a mut Scratch,

    pos: usize,
    state: u32,
    can_decode: bool,
    last_tile_part: bool,
    correction_checked: bool,
    correction: u32,
    sot_length: u32,
    current_tile: u32,
    truncated: bool,
    cut_in_decoded: bool,
    tiles: BTreeMap<u32, TileWalk>,
    ppm: PackedCursor,
    /// `image_working_memory`, once a tile needs it.
    image_need: Option<u64>,
    /// The working memory the limit counted for this decode
    /// (`decode_working_memory`); 0 without a memory limit.
    need: u64,
}

impl Walk<'_, '_> {
    // ---- the stream, over the bytes in memory ----------------------------------------------

    /// `opj_stream_get_number_byte_left`
    fn bytes_left(&self) -> usize {
        self.data.len() - self.pos
    }

    /// `opj_stream_read_data` without its copy: `n`, fewer at the end, or
    /// `END` when nothing is left.
    fn read(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let avail = self.bytes_left();
        if avail == 0 {
            return END;
        }
        let take = n.min(avail);
        self.pos += take;
        take
    }

    /// Two bytes as a marker or a length; `None` when fewer are left (what
    /// is left is read, as `opj_stream_read_data` does).
    fn read_u16(&mut self) -> Option<u32> {
        let at = self.pos;
        if self.read(2) != 2 {
            return None;
        }
        Some(u32::from(be16(&self.data[at..])))
    }

    fn seek(&mut self, to: usize) -> bool {
        if to > self.data.len() {
            return false;
        }
        self.pos = to;
        true
    }

    /// Whether a tile is decoded: OpenJPEG decodes a tile whose tile-parts
    /// gave it data bytes. DP-3: a tile whose tile-parts arrived whole
    /// without data bytes, its packet headers packed in PPM or PPT, decodes
    /// too; without packed headers its packets are missing, and it is
    /// refused as in OpenJPEG.
    fn decodable(&self, tile: u32) -> bool {
        let Some(tw) = self.tiles.get(&tile) else {
            return false;
        };
        if !tw.seen {
            return false;
        }
        if tw.has_data || self.mh.has_ppm {
            return true;
        }
        self.mh.overrides(tile).is_some_and(|ov| !ov.ppt.is_empty())
    }

    /// `opj_j2k_read_sot`
    fn read_sot(&mut self, p: &[u8]) -> Result<(), Error> {
        if p.len() != 8 {
            return Err(Error::Header("Error reading SOT marker"));
        }
        self.current_tile = u32::from(be16(p));
        let tot_len = be32(&p[2..]);
        let current_part = u32::from(p[6]);
        let mut num_parts = u32::from(p[7]);
        if self.current_tile >= self.mh.tile_count() {
            return Err(Error::Header("Invalid tile number"));
        }
        let correction = self.correction;
        let tw = self.tiles.entry(self.current_tile).or_default();
        // ISO 15444-1 A.4.2: tile-parts in increasing order
        if tw.current_tile_part_number + 1 != current_part as i32 {
            return Err(Error::Header("Invalid tile part index"));
        }
        tw.current_tile_part_number = current_part as i32;
        // (Psot = 12: an empty SOT, which OpenJPEG takes with a warning)
        if tot_len != 0 && tot_len < 14 && tot_len != 12 {
            return Err(Error::Header("Psot value is not correct regards to the JPEG2000 norm"));
        }
        if tot_len == 0 {
            self.last_tile_part = true;
        }
        if tw.nb_tile_parts != 0 && current_part >= tw.nb_tile_parts {
            self.last_tile_part = true;
            return Err(Error::Header("TPSot is not valid regards to the previous number of tile-parts"));
        }
        if num_parts != 0 {
            num_parts += correction;
            if current_part >= num_parts {
                self.last_tile_part = true;
                return Err(Error::Header("TPSot is not valid regards to the current number of tile-parts"));
            }
            tw.nb_tile_parts = num_parts;
        }
        if tw.nb_tile_parts != 0 && tw.nb_tile_parts == current_part + 1 {
            self.can_decode = true;
        }
        self.sot_length = if !self.last_tile_part { tot_len.wrapping_sub(12) } else { 0 };
        self.state = TPH;
        Ok(())
    }

    /// OpenJPEG refuses a reduce factor at or above the resolutions of a COD
    /// or COC it reads, tile-part headers included; here each
    /// tile-component is checked after marker precedence (DP-5), when a
    /// tile-part header that changes the coding style is complete.
    fn check_tile_reduce(&self) -> Result<(), Error> {
        if self.factor == 0 {
            return Ok(());
        }
        let Some(ov) = self.mh.overrides(self.current_tile) else {
            return Ok(());
        };
        if ov.cod.is_none() && ov.coc.is_empty() {
            return Ok(());
        }
        for c in 0..self.mh.comps.len() {
            let p = resolve_tile_component(self.mh, Some(ov), c)?;
            if self.factor >= u32::from(p.style.numresolutions) {
                return Err(Error::Limit("the reduce factor is not below the resolutions of a component"));
            }
        }
        Ok(())
    }

    /// `opj_j2k_read_sod` (the tile-part's data: where it lies)
    fn read_sod(&mut self) -> Result<(), Error> {
        // the tile-part's data length comes from what arrived, not from its
        // header (a cut Psot = 0 tile-part): no data then means the bytes
        // ended at its SOD, not an empty tile-part (DP-3)
        let mut length_from_cut = false;
        if self.last_tile_part {
            let left = self.bytes_left();
            // opj_j2k_find_final_eoc: whether the codestream's last two bytes are EOC
            let ends_with_eoc = left >= 2 && u32::from(be16(&self.data[self.data.len() - 2..])) == u32::from(mk::EOC);
            if !ends_with_eoc {
                // Psot = 0 runs the tile-part to the EOC marker, but a cut codestream has none
                self.sot_length = left.min(u32::MAX as usize) as u32;
                self.truncated = true;
                length_from_cut = true;
            } else {
                self.sot_length = (left - 2).min(u32::MAX as usize) as u32;
            }
        } else if self.sot_length >= 2 {
            self.sot_length -= 2;
        }
        let mut pb_detected = false;
        if self.sot_length != 0 {
            // (a tile-part longer than the stream: a warning, the cut below)
            if self.sot_length > u32::MAX - 2 {
                return Err(Error::Data("tile-part length above system limits"));
            }
            // OpenJPEG allocates sot_length bytes here, before reading them;
            // this decoder records where the bytes are
        } else {
            pb_detected = true;
        }
        let at = self.pos;
        let mut read_size = 0;
        if !pb_detected {
            read_size = self.read(self.sot_length as usize);
        }
        let sot_length = self.sot_length as usize;
        let Some(tw) = self.tiles.get_mut(&self.current_tile) else {
            return Err(Error::Data("tile-part data without its SOT"));
        };
        if !pb_detected || !length_from_cut {
            // the tile-part arrived (DP-3: one that declares no data bytes
            // counts, one whose bytes ended at its SOD does not)
            tw.seen = true;
        }
        if read_size != sot_length {
            if read_size == END {
                return Err(Error::NeedData("a tile-part's data"));
            }
            self.state = NEOC;
            self.truncated = true;
            tw.cut = true;
        } else {
            self.state = TPHSOT;
        }
        if length_from_cut {
            tw.cut = true;
        }
        if !pb_detected {
            tw.has_data = true;
            if read_size != 0 {
                tw.ranges.push((at, read_size));
            }
        }
        Ok(())
    }

    /// `opj_j2k_need_nb_tile_parts_correction` (OpenJPEG issue 254:
    /// TPsot == TNsot)
    fn need_correction(&mut self, tile_no: u32) -> Result<bool, Error> {
        let backup = self.pos;
        let (current_part, num_parts);
        loop {
            let at = self.pos;
            if self.read(2) != 2 || u32::from(be16(&self.data[at..])) != u32::from(mk::SOT) {
                self.seek(backup);
                return Ok(false);
            }
            let Some(size) = self.read_u16() else {
                return Err(Error::NeedData("a SOT marker segment"));
            };
            if size != 10 {
                return Err(Error::Header("Inconsistent marker size"));
            }
            let at = self.pos;
            if self.read(8) != 8 {
                return Err(Error::NeedData("a SOT marker segment"));
            }
            let h = &self.data[at..at + 8];
            let l_tile_no = u32::from(be16(h));
            let l_tot_len = be32(&h[2..]);
            if l_tile_no == tile_no {
                current_part = u32::from(h[6]);
                num_parts = u32::from(h[7]);
                break;
            }
            if l_tot_len < 14 {
                self.seek(backup);
                return Ok(false);
            }
            // opj_stream_read_skip: the whole tile-part, or the search ends
            let skip = (l_tot_len - 12) as usize;
            if skip > self.bytes_left() {
                self.seek(backup);
                return Ok(false);
            }
            self.pos += skip;
        }
        self.seek(backup);
        Ok(current_part == num_parts)
    }

    /// `opj_j2k_read_tile_header`: whether there is a tile to decode.
    fn read_tile_header(&mut self) -> Result<bool, Error> {
        let (sot, sod, eoc) = (u32::from(mk::SOT), u32::from(mk::SOD), u32::from(mk::EOC));
        let mut current_marker = sot;
        if self.state == EOC {
            current_marker = eoc;
        } else if self.state != TPHSOT {
            return Err(Error::Data("The codestream ended before its next tile-part"));
        }
        while !self.can_decode && current_marker != eoc {
            while current_marker != sod {
                if self.bytes_left() == 0 {
                    self.state = NEOC;
                    break;
                }
                if self.state & TPH != 0 && segmentless(current_marker) {
                    if !(0xFF30..=0xFF3F).contains(&current_marker) {
                        // SOC, EOC or EPH in a tile-part header (SOD ends it): not a marker of
                        // a tile-part header, and no segment to read (OpenJPEG fails on it too)
                        return Err(Error::Header("Marker is not compliant with its position"));
                    }
                    // a marker without a segment: skipped (DP-7), the length it leaves counted
                    if self.sot_length != 0 {
                        if self.sot_length < 2 {
                            return Err(Error::Header("Sot length is less than marker size + marker ID"));
                        }
                        self.sot_length -= 2;
                    }
                    current_marker = self.read_u16().ok_or(Error::NeedData("a tile-part header"))?;
                    continue;
                }
                let mut marker_size = self.read_u16().ok_or(Error::NeedData("a tile-part header"))?;
                if marker_size < 2 {
                    return Err(Error::Header("Inconsistent marker size"));
                }
                // OpenJPEG issue 226
                if current_marker == 0x8080 && self.bytes_left() == 0 {
                    self.state = NEOC;
                    break;
                }
                if self.state & TPH != 0 && self.sot_length != 0 {
                    if self.sot_length < marker_size + 2 {
                        return Err(Error::Header("Sot length is less than marker size + marker ID"));
                    }
                    self.sot_length -= marker_size + 2;
                }
                marker_size -= 2;
                // where the marker may be
                let is_sot = current_marker == sot;
                if current_marker < 0xFF00 {
                    return Err(Error::Header("A marker was expected in a tile-part header"));
                }
                if (self.state & TPHSOT != 0 && !is_sot) || (self.state & TPH != 0 && is_sot) {
                    return Err(Error::Header("Marker is not compliant with its position"));
                }
                if !is_sot && part2_marker(current_marker) {
                    return Err(Error::Unsupported("JPEG 2000 Part 2 marker"));
                }
                let size = marker_size as usize;
                if size > self.bytes_left() {
                    return Err(Error::NeedData("a tile-part header"));
                }
                let body = &self.data[self.pos..self.pos + size];
                self.pos += size;
                if is_sot {
                    self.read_sot(body)?;
                } else {
                    read_tile_part_marker(self.limits, self.mh, self.current_tile as u16, current_marker as u16, body)?;
                }
                current_marker = self.read_u16().ok_or(Error::NeedData("a tile-part header"))?;
            }
            if self.bytes_left() == 0 && self.state == NEOC {
                break;
            }
            self.check_tile_reduce()?;
            self.read_sod()?;
            if self.can_decode && !self.correction_checked {
                // OpenJPEG issue 254
                self.correction_checked = true;
                let single = self.tiles.get(&self.current_tile).is_some_and(|tw| tw.nb_tile_parts == 1);
                // a single tile-part declared: no check (as OpenJPEG since 2024)
                if !single && self.need_correction(self.current_tile)? {
                    // non conformant codestream: TPsot == TNsot
                    self.can_decode = false;
                    self.correction = 1;
                    for tw in self.tiles.values_mut() {
                        if tw.nb_tile_parts != 0 {
                            tw.nb_tile_parts += 1;
                        }
                    }
                }
            }
            if !self.can_decode && self.last_tile_part && self.bytes_left() == 0 {
                // the last tile-part (Psot = 0) of a cut codestream took every byte left
                self.state = NEOC;
                break;
            }
            if !self.can_decode {
                match self.read_u16() {
                    Some(m) => current_marker = m,
                    None => {
                        // likely non-conformant SPOT6 files: the last row of tiles with
                        // TPsot == 0 and TNsot == 0, no EOC, and no other tile-part found
                        if self.current_tile + 1 == self.mh.tile_count() {
                            let found = (0..self.mh.tile_count()).find(|t| {
                                let (cur, nb) = self
                                    .tiles
                                    .get(t)
                                    .map_or((-1, 0), |tw| (tw.current_tile_part_number, tw.nb_tile_parts));
                                cur == 0 && nb == 0
                            });
                            if let Some(t) = found {
                                self.current_tile = t;
                                current_marker = eoc;
                                self.state = EOC;
                                break;
                            }
                        }
                        return Err(Error::NeedData("the next tile-part"));
                    }
                }
            }
        }
        if current_marker == eoc && self.state != EOC {
            self.current_tile = 0;
            self.state = EOC;
        }
        // tiles with a single tile-part of TPsot == 0 and TNsot == 0
        if !self.can_decode {
            while self.current_tile < self.mh.tile_count() && !self.decodable(self.current_tile) {
                self.current_tile += 1;
            }
            if self.current_tile == self.mh.tile_count() {
                return Ok(false);
            }
        }
        self.state |= DATA;
        Ok(true)
    }

    /// `opj_j2k_read_tile_header`'s tile set-up and `opj_j2k_decode_tile`,
    /// then the image update.
    fn decode_tile(&mut self) -> Result<(), Error> {
        let tile_no = self.current_tile;
        let decodable = self.decodable(tile_no);
        self.decode_tile_samples(tile_no, decodable)?;
        self.can_decode = false;
        self.state &= !DATA;
        // the marker after the tile
        if !(self.bytes_left() == 0 && self.state == NEOC) && self.state != EOC {
            match self.read_u16() {
                None => self.truncated = true,
                Some(m) if m == u32::from(mk::EOC) => {
                    self.current_tile = 0;
                    self.state = EOC;
                }
                Some(m) if m != u32::from(mk::SOT) => {
                    if self.bytes_left() == 0 {
                        // the stream does not end with EOC
                        self.state = NEOC;
                        self.truncated = true;
                    } else {
                        return Err(Error::Data("Stream too short, expected SOT"));
                    }
                }
                Some(_) => {}
            }
        }
        // opj_j2k_tcp_data_destroy (a 1 x 1 grid keeps it, but is not decoded again)
        if let Some(tw) = self.tiles.get_mut(&tile_no) {
            tw.seen = false;
            tw.has_data = false;
            tw.cut = false;
            tw.ranges.clear();
        }
        Ok(())
    }

    /// A tile's layout, packets and samples, and its copy into the image
    /// (which OpenJPEG makes after it has read the marker that follows the
    /// tile: a decode that fails on that marker gives no image either way).
    fn decode_tile_samples(&mut self, tile_no: u32, decodable: bool) -> Result<(), Error> {
        let (factor, decoded) = (self.factor, self.decoded);
        let mh: &MainHeader = self.mh;
        let scratch = &mut *self.scratch;
        let Some(tw) = self.tiles.get_mut(&tile_no) else {
            return Err(Error::Data("a tile without tile-parts"));
        };
        // the tile's parameters, with precedence applied and step counts checked
        let ov = mh.overrides(tile_no);
        let params = resolve_tile(mh, ov)?;
        // The decoder limits' working memory for a tile whose own coding (a
        // tile-part COD or COC) changes it: the decode's check when it
        // started took the main header's coding.
        let mut need = self.need;
        if self.limits.max_memory != 0 && ov.is_some_and(|ov| ov.cod.is_some() || !ov.coc.is_empty()) {
            let image = *self
                .image_need
                .get_or_insert_with(|| header::image_working_memory(mh, factor, decoded));
            let part = header::tile_working_memory(mh, tile_no, &params, factor, decoded);
            if image.saturating_add(part) > self.limits.max_memory {
                return Err(Error::Limit(
                    "a tile's own coding needs more working memory than the decoder's limit",
                ));
            }
            need = need.max(image.saturating_add(part));
        }
        // opj_j2k_merge_ppt: the tile's PPT markers in Zppt order, in one buffer
        scratch.ppt.clear();
        let has_ppt = ov.is_some_and(|ov| !ov.ppt.is_empty());
        if let Some(ov) = ov.filter(|_| has_ppt) {
            if tw.ppt_merged {
                return Err(Error::Header("Failed to merge PPT data"));
            }
            tw.ppt_merged = true;
            let mut order: Vec<usize> = (0..ov.ppt.len()).collect();
            order.sort_by_key(|&i| ov.ppt[i].z);
            for i in order {
                scratch.ppt.extend_from_slice(&ov.ppt[i].data);
            }
        }
        // The tile's bytes after SOD, in one piece: where they lie when they
        // came in one tile-part, gathered otherwise (OpenJPEG concatenates
        // every tile-part's data).
        let tile_data: &[u8] = match tw.ranges.as_slice() {
            [] => &[],
            [(at, len)] => &self.data[*at..*at + *len],
            ranges => {
                scratch.tile_bytes.clear();
                for (at, len) in ranges {
                    scratch.tile_bytes.extend_from_slice(&self.data[*at..*at + *len]);
                }
                &scratch.tile_bytes
            }
        };
        if tile_data.len() > u32::MAX as usize {
            return Err(Error::Limit("a tile's data above system limits"));
        }
        self.tile.init(mh, tile_no, &params, factor, decoded)?;
        // opj_j2k_decode_tile: the tile needs data (DP-3: or packed packet headers)
        if !decodable {
            return Err(Error::Data("a tile without data"));
        }
        let numcomps = mh.comps.len();
        scratch.resno.clear();
        scratch.resno.resize(numcomps, 0);
        // data that ends inside a packet makes no truncation of its own: when
        // the bytes were cut, the tile-part's length or the markers after it
        // say so; when they were not, the packets claim more than their
        // tile-part holds (damage, decoded as OpenJPEG does)
        let headers = if mh.has_ppm {
            PacketHeaders::Ppm(&mh.ppm_data, &mut self.ppm)
        } else if has_ppt {
            PacketHeaders::Ppt(&scratch.ppt, PackedCursor::default())
        } else {
            PacketHeaders::Inline
        };
        let input = PacketInput {
            mh,
            tile: tile_no,
            params: &params,
            data: tile_data,
            cut: tw.cut,
            headers,
        };
        let stats = t2::read_packets(self.tile, input, &mut scratch.resno, &mut scratch.included)?;
        self.cut_in_decoded |= stats.cut_in_decoded;

        // the samples: the decoder's buffers, a component each
        if scratch.planes.len() < numcomps {
            scratch.planes.resize_with(numcomps, Vec::new);
        }
        let mut total = 0u64;
        for c in 0..decoded {
            total = total.saturating_add(self.tile.samples(c) as u64 * 4);
        }
        if self.limits.max_memory != 0 && total > self.limits.max_memory {
            return Err(Error::Limit("the tile's samples need more memory than the decoder's limit"));
        }
        for (c, plane) in scratch.planes.iter_mut().enumerate().take(decoded) {
            let n = self.tile.samples(c);
            // (Tier-1 writes every sample of a component: nothing to clear)
            if plane.len() < n {
                *plane = Vec::new();
                *plane = crate::zeroed(n)?;
            }
        }
        let coding_mct = params[0].coding.mct != 0;
        // A shared decode holds, beyond what the limit counted, a copy of the
        // tile's bytes and the code-blocks the lent threads decoded (all of
        // them at most) until they are in their component: with a limit that
        // has no room for them, one thread.
        let shared = need.saturating_add(total).saturating_add(tile_data.len() as u64);
        let threads = if self.limits.max_memory != 0 && shared > self.limits.max_memory {
            Threads::default()
        } else {
            self.threads
        };
        let work = SampleWork {
            t1: &mut scratch.t1,
            dwt: &mut scratch.dwt,
            planes: &mut scratch.planes,
            blocks: &mut scratch.blocks,
            helpers: &mut scratch.helpers,
            threads,
        };
        self.tile.decode_samples(coding_mct, tile_data, &scratch.resno, work)?;
        write_tile(self.tile, mh, factor, decoded, &scratch.resno, &mut scratch.planes, &mut self.out)
    }

    /// `opj_j2k_decode_tiles`
    fn run(&mut self) -> Result<(), Error> {
        let mut nr_tiles = 0;
        loop {
            if !self.read_tile_header()? {
                break;
            }
            self.decode_tile()?;
            if self.bytes_left() == 0 && self.state == NEOC {
                break;
            }
            nr_tiles += 1;
            if nr_tiles == self.mh.tile_count() {
                break;
            }
        }
        // opj_j2k_are_all_used_components_decoded
        if self.out.comps[..self.decoded].iter().any(|c| !c.written) {
            // when the bytes ran out first, nothing was found damaged: more bytes bring the tiles
            return Err(if self.truncated || self.state & NEOC != 0 {
                Error::NeedData("the tiles of the image")
            } else {
                Error::Data("Failed to decode all used components")
            });
        }
        Ok(())
    }
}

/// `opj_j2k_update_image_data` for one component, up to the copy: the
/// tile's decoded region that goes into the image.
struct Prepared {
    /// The region's first sample in the tile-component's buffer, and the
    /// buffer's row length.
    src: usize,
    stride: usize,
    /// Its place in the image component.
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

fn prepare_component(tile: &Tile, c: usize, resno_decoded: u32, dst: &OutComp, factor: u32) -> Result<Option<Prepared>, Error> {
    let tc = &tile.comps[c];
    if tile.samples(c) == 0 {
        return Ok(None);
    }
    let Some(res) = tile.res_of(c).get(resno_decoded as usize) else {
        return Err(Error::Data("decoded resolution out of range"));
    };
    let (res_x0, res_y0, res_x1, res_y1) = (i64::from(res.x0), i64::from(res.y0), i64::from(res.x1), i64::from(res.y1));
    let (width_src, height_src) = (res_x1 - res_x0, res_y1 - res_y0);
    let x0_dest = i64::from(ceildivpow2(dst.x0, factor));
    let y0_dest = i64::from(ceildivpow2(dst.y0, factor));
    let (x1_dest, y1_dest) = (x0_dest + i64::from(dst.w), y0_dest + i64::from(dst.h));
    // one axis: where the region starts in the image, its length, and what
    // is left out of the source before and after it
    let axis = |a0_dest: i64, a1_dest: i64, r0: i64, r1: i64, len_src: i64, len_dst: i64| -> (i64, i64, i64, i64) {
        if a0_dest < r0 {
            if a1_dest >= r1 {
                (r0 - a0_dest, len_src, 0, 0)
            } else {
                (r0 - a0_dest, a1_dest - r0, 0, len_src - (a1_dest - r0))
            }
        } else {
            let off0 = a0_dest - r0;
            if a1_dest >= r1 {
                (0, len_src - off0, off0, 0)
            } else {
                (0, len_dst, off0, r1 - a1_dest)
            }
        }
    };
    let (start_x, width, off_x0, off_x1) = axis(x0_dest, x1_dest, res_x0, res_x1, width_src, i64::from(dst.w));
    let (start_y, height, off_y0, off_y1) = axis(y0_dest, y1_dest, res_y0, res_y1, height_src, i64::from(dst.h));
    if off_x0 < 0 || off_y0 < 0 || off_x1 < 0 || off_y1 < 0 || width < 0 || height < 0 {
        return Err(Error::Data("a tile outside its image"));
    }
    // (what the copy relies on: the region inside both buffers)
    if off_x0 + width > i64::from(tc.data_w)
        || off_y0 + height > i64::from(tc.data_h)
        || start_x + width > i64::from(dst.w)
        || start_y + height > i64::from(dst.h)
    {
        return Err(Error::Data("a tile outside its image"));
    }
    if width == 0 || height == 0 {
        return Ok(None);
    }
    Ok(Some(Prepared {
        src: off_x0 as usize + off_y0 as usize * tc.data_w as usize,
        stride: tc.data_w as usize,
        x: start_x as usize,
        y: start_y as usize,
        w: width as usize,
        h: height as usize,
    }))
}

/// `opj_j2k_update_image_data` for every decoded component of a tile:
/// finished on the way (the colour transform, the DC shift) into the image.
/// When the components do not pair their samples by place, the tile is
/// finished in place first, as OpenJPEG does, and copied.
fn write_tile(
    tile: &mut Tile,
    mh: &MainHeader,
    factor: u32,
    decoded: usize,
    resno: &[u32],
    planes: &mut [Vec<i32>],
    out: &mut Output<'_>,
) -> Result<(), Error> {
    let mut preps = Vec::with_capacity(decoded);
    for c in 0..decoded {
        preps.push(prepare_component(tile, c, resno[c], &out.comps[c], factor)?);
    }
    // one pass for every component: the same region of the image, read at
    // the same place of each buffer
    let uniform = decoded <= 4
        && preps.iter().all(|p| p.is_some())
        && preps.windows(2).all(|p| match (&p[0], &p[1]) {
            (Some(a), Some(b)) => (a.src, a.stride, a.x, a.y, a.w, a.h) == (b.src, b.stride, b.x, b.y, b.w, b.h),
            _ => false,
        });
    if uniform {
        let Some(Some(p)) = preps.first() else {
            return Ok(());
        };
        let regions: Vec<Region<'_>> = (0..decoded)
            .map(|c| Region {
                src: &planes[c][p.src..],
                stride: p.stride,
                f: tile.finish_of(mh, c),
            })
            .collect();
        match out.samples {
            Samples::Packed(data) => {
                let row = out.width * decoded;
                mct::write_packed_u8(&regions, tile.mct, p.w, p.h, &mut data[p.y * row + p.x * decoded..], row);
            }
            Samples::Planes(image) => {
                let mut dsts: Vec<(&mut [i32], usize)> = image
                    .iter_mut()
                    .take(decoded)
                    .map(|pl| {
                        let stride = pl.width as usize;
                        (&mut pl.data[p.y * stride + p.x..], stride)
                    })
                    .collect();
                mct::write_planes(&regions, tile.mct, p.w, p.h, &mut dsts);
            }
        }
        for c in &mut out.comps[..decoded] {
            c.written = true;
        }
        return Ok(());
    }

    // The tile's samples finished where they are, as OpenJPEG does it: the
    // colour transform over the three whole buffers, then each component's
    // DC shift over its decoded region.
    if tile.mct != Mct::None && decoded >= 3 {
        let n = tile.samples(0).min(tile.samples(1)).min(tile.samples(2));
        let (a, rest) = planes.split_at_mut(1);
        let (b, c) = rest.split_at_mut(1);
        mct::mct_in_place(&mut a[0][..n], &mut b[0][..n], &mut c[0][..n], tile.mct);
    }
    tile.mct = Mct::None;
    for c in 0..decoded {
        if tile.samples(c) == 0 {
            continue;
        }
        let tc = tile.comps[c];
        let Some(res) = tile.res_of(c).get(resno[c] as usize) else {
            continue;
        };
        let (w, h) = (
            ((res.x1 - res.x0) as usize).min(tc.data_w as usize),
            ((res.y1 - res.y0) as usize).min(tc.data_h as usize),
        );
        mct::shift_in_place(&mut planes[c], tc.data_w as usize, w, h, &tile.finish_of(mh, c));
    }
    for (c, p) in preps.iter().enumerate() {
        let Some(p) = p else {
            continue;
        };
        let src = &planes[c][p.src..];
        match out.samples {
            Samples::Packed(data) => {
                let row = out.width * decoded;
                for j in 0..p.h {
                    let line = &mut data[(p.y + j) * row + p.x * decoded..];
                    for (px, &v) in line.chunks_exact_mut(decoded).zip(&src[j * p.stride..j * p.stride + p.w]) {
                        px[c] = v as u8;
                    }
                }
            }
            Samples::Planes(image) => {
                let stride = out.comps[c].w as usize;
                let plane = &mut image[c].data;
                for j in 0..p.h {
                    plane[(p.y + j) * stride + p.x..(p.y + j) * stride + p.x + p.w].copy_from_slice(&src[j * p.stride..j * p.stride + p.w]);
                }
            }
        }
        out.comps[c].written = true;
    }
    Ok(())
}

/// What [`decode_codestream`] works with.
pub(crate) struct Job<'a> {
    pub data: &'a [u8],
    pub mh: &'a mut MainHeader,
    /// The offset just after the first SOT marker.
    pub start: usize,
    pub limits: &'a Limits,
    pub factor: u32,
    pub decoded: usize,
    pub threads: Threads,
    /// `decode_working_memory`, as the limit counted it; 0 without a limit.
    pub need: u64,
    pub tile: &'a mut Tile,
    pub scratch: &'a mut Scratch,
}

/// Decodes every tile into `samples`, whose buffers the caller made for the
/// components' geometry (`comps`).
pub(crate) fn decode_codestream(job: Job<'_>, comps: &mut [OutComp], samples: &mut Samples, width: usize) -> Result<Outcome, Error> {
    // the tile-part headers' markers are this decode's
    job.mh.tiles.clear();
    let mut w = Walk {
        data: job.data,
        threads: job.threads,
        mh: job.mh,
        limits: job.limits,
        factor: job.factor,
        decoded: job.decoded,
        out: Output { comps, samples, width },
        tile: job.tile,
        scratch: job.scratch,
        pos: job.start,
        state: TPHSOT,
        can_decode: false,
        last_tile_part: false,
        correction_checked: false,
        correction: 0,
        sot_length: 0,
        current_tile: 0,
        truncated: false,
        cut_in_decoded: false,
        tiles: BTreeMap::new(),
        ppm: PackedCursor::default(),
        image_need: None,
        need: job.need,
    };
    w.run()?;
    Ok(Outcome {
        truncated: w.truncated || w.state != EOC,
        cut_in_decoded: w.cut_in_decoded,
    })
}

/// The image's planes for components that are not packed (see
/// [`Samples`]).
pub(crate) fn new_planes(mh: &MainHeader, comps: &[OutComp]) -> Result<Vec<Component>, Error> {
    let mut planes = Vec::with_capacity(comps.len());
    for (c, sc) in comps.iter().zip(&mh.comps) {
        let n = c.w as usize * c.h as usize;
        let data = crate::zeroed(n)?;
        planes.push(Component {
            width: c.w,
            height: c.h,
            precision: sc.prec,
            signed: sc.signed,
            data,
        });
    }
    Ok(planes)
}
