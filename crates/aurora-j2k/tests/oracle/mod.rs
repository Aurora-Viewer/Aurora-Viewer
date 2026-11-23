//! OpenJPEG as the reference of the tests and of the comparison tool: its
//! decoder, called the way the viewer has always called it (a reduce factor
//! set before the header, non-strict mode so that a cut codestream decodes
//! as far as it goes), and its encoder, which writes the codestreams the
//! tests decode.
#![allow(dead_code)]

use std::ffi::c_void;
use std::ptr;

use openjpeg_sys as opj;

/// One component as OpenJPEG decodes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
    pub w: u32,
    pub h: u32,
    pub prec: u32,
    pub signed: bool,
    pub data: Vec<i32>,
}

struct MemStream<'a> {
    data: &'a [u8],
    offset: usize,
}

unsafe extern "C" fn mem_read(buffer: *mut c_void, nb_bytes: usize, user: *mut c_void) -> usize {
    // SAFETY: `user` is the `MemStream` of `decode`, which outlives the stream.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    if s.offset >= s.data.len() {
        return usize::MAX;
    }
    let n = nb_bytes.min(s.data.len() - s.offset);
    // SAFETY: OpenJPEG gives `nb_bytes` writable bytes at `buffer`; `n` is no more, and inside `s.data`.
    unsafe { ptr::copy_nonoverlapping(s.data.as_ptr().add(s.offset), buffer as *mut u8, n) };
    s.offset += n;
    n
}

unsafe extern "C" fn mem_skip(nb_bytes: i64, user: *mut c_void) -> i64 {
    // SAFETY: see `mem_read`.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    let to = (s.offset as i64).saturating_add(nb_bytes);
    if to < 0 || to > s.data.len() as i64 {
        s.offset = to.clamp(0, s.data.len() as i64) as usize;
        return -1;
    }
    s.offset = to as usize;
    nb_bytes
}

unsafe extern "C" fn mem_seek(offset: i64, user: *mut c_void) -> opj::OPJ_BOOL {
    // SAFETY: see `mem_read`.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    if offset < 0 || offset > s.data.len() as i64 {
        return 0;
    }
    s.offset = offset as usize;
    1
}

/// OpenJPEG's decode of a codestream at a reduce factor: its components, or
/// `None` when it refuses the codestream.
pub fn decode(cs: &[u8], reduce: u32) -> Option<Vec<Plane>> {
    let mut mem = MemStream { data: cs, offset: 0 };
    // SAFETY: plain FFI use of OpenJPEG's decoder; every object is checked
    // for null and destroyed once, and `mem` outlives the stream that reads it.
    unsafe {
        let stream = opj::opj_stream_create(cs.len().clamp(4096, 1 << 20), 1);
        if stream.is_null() {
            return None;
        }
        opj::opj_stream_set_user_data(stream, &mut mem as *mut MemStream<'_> as *mut c_void, None);
        opj::opj_stream_set_user_data_length(stream, cs.len() as u64);
        opj::opj_stream_set_read_function(stream, Some(mem_read));
        opj::opj_stream_set_skip_function(stream, Some(mem_skip));
        opj::opj_stream_set_seek_function(stream, Some(mem_seek));
        let codec = opj::opj_create_decompress(opj::CODEC_FORMAT::OPJ_CODEC_J2K);
        let mut params: opj::opj_dparameters_t = std::mem::zeroed();
        opj::opj_set_default_decoder_parameters(&mut params);
        params.cp_reduce = reduce;
        let mut image: *mut opj::opj_image_t = ptr::null_mut();
        let mut out = None;
        if !codec.is_null() && opj::opj_setup_decoder(codec, &mut params) != 0 {
            opj::opj_decoder_set_strict_mode(codec, 0);
            if opj::opj_read_header(stream, codec, &mut image) != 0 && !image.is_null() {
                let decoded = opj::opj_decode(codec, stream, image);
                opj::opj_end_decompress(codec, stream);
                if decoded != 0 {
                    let img = &*image;
                    let comps = std::slice::from_raw_parts(img.comps, img.numcomps as usize);
                    let mut planes = Vec::new();
                    for c in comps {
                        if c.data.is_null() {
                            planes.clear();
                            break;
                        }
                        let data = std::slice::from_raw_parts(c.data, c.w as usize * c.h as usize).to_vec();
                        planes.push(Plane {
                            w: c.w,
                            h: c.h,
                            prec: c.prec,
                            signed: c.sgnd != 0,
                            data,
                        });
                    }
                    if !planes.is_empty() {
                        out = Some(planes);
                    }
                }
            }
        }
        if !image.is_null() {
            opj::opj_image_destroy(image);
        }
        if !codec.is_null() {
            opj::opj_destroy_codec(codec);
        }
        opj::opj_stream_destroy(stream);
        out
    }
}

struct VecWriter {
    buf: Vec<u8>,
    pos: usize,
}

unsafe extern "C" fn w_write(buffer: *mut c_void, n: usize, user: *mut c_void) -> usize {
    // SAFETY: `user` is the `VecWriter` of `encode`; OpenJPEG gives `n` readable bytes.
    let (w, src) = unsafe { (&mut *(user as *mut VecWriter), std::slice::from_raw_parts(buffer as *const u8, n)) };
    let end = w.pos + n;
    if w.buf.len() < end {
        w.buf.resize(end, 0);
    }
    w.buf[w.pos..end].copy_from_slice(src);
    w.pos = end;
    n
}

unsafe extern "C" fn w_skip(n: i64, user: *mut c_void) -> i64 {
    // SAFETY: see `w_write`.
    let w = unsafe { &mut *(user as *mut VecWriter) };
    w.pos = (w.pos as i64 + n).max(0) as usize;
    n
}

unsafe extern "C" fn w_seek(pos: i64, user: *mut c_void) -> opj::OPJ_BOOL {
    // SAFETY: see `w_write`.
    let w = unsafe { &mut *(user as *mut VecWriter) };
    w.pos = pos.max(0) as usize;
    1
}

/// What the encoder is asked for. The defaults are a plain lossless
/// codestream.
#[derive(Debug, Clone)]
pub struct Encode {
    pub w: u32,
    pub h: u32,
    pub comps: u32,
    pub prec: u32,
    pub signed: bool,
    /// The 9/7 wavelet (lossy), else the 5/3.
    pub irreversible: bool,
    pub mct: bool,
    pub numresolution: i32,
    /// Code-block size, powers of two from 4 to 1024, their product at most 4096.
    pub cblk: (i32, i32),
    /// Code-block style: 1 BYPASS, 2 RESET, 4 TERMALL, 8 VSC, 16 PTERM, 32 SEGSYM.
    pub mode: i32,
    /// 0 LRCP, 1 RLCP, 2 RPCL, 3 PCRL, 4 CPRL.
    pub prog: i32,
    /// A compression ratio a layer, the last one 0 for everything.
    pub rates: Vec<f32>,
    /// Tile size, or none for one tile.
    pub tile: Option<(i32, i32)>,
    /// The image's origin on the reference grid, and the tile grid's.
    pub origin: (i32, i32),
    pub tile_origin: (i32, i32),
    /// Precinct sizes from the highest resolution down, or none.
    pub precincts: Vec<(i32, i32)>,
    /// 2: SOP markers, 4: EPH markers.
    pub csty: i32,
    /// Sub-sampling of every component.
    pub sub: (i32, i32),
    /// A region of interest: the component and its shift.
    pub roi: Option<(i32, i32)>,
    /// Progression order changes: `(resno0, compno0, layno1, resno1, compno1, prog)`.
    pub pocs: Vec<(u32, u32, u32, u32, u32, i32)>,
}

impl Default for Encode {
    fn default() -> Self {
        Encode {
            w: 64,
            h: 64,
            comps: 3,
            prec: 8,
            signed: false,
            irreversible: false,
            mct: true,
            numresolution: 6,
            cblk: (64, 64),
            mode: 0,
            prog: 0,
            rates: vec![0.0],
            tile: None,
            origin: (0, 0),
            tile_origin: (0, 0),
            precincts: Vec::new(),
            csty: 0,
            sub: (1, 1),
            roi: None,
            pocs: Vec::new(),
        }
    }
}

fn prog_order(p: i32) -> opj::PROG_ORDER {
    match p {
        0 => opj::PROG_ORDER::OPJ_LRCP,
        1 => opj::PROG_ORDER::OPJ_RLCP,
        2 => opj::PROG_ORDER::OPJ_RPCL,
        3 => opj::PROG_ORDER::OPJ_PCRL,
        _ => opj::PROG_ORDER::OPJ_CPRL,
    }
}

/// A codestream written by OpenJPEG; `sample(x, y, component)` gives the
/// image. `None` when OpenJPEG refuses the settings.
pub fn encode(e: &Encode, sample: impl Fn(u32, u32, u32) -> i32) -> Option<Vec<u8>> {
    // SAFETY: plain FFI use of OpenJPEG's encoder on objects it allocates;
    // every pointer is checked and each object destroyed once. The planes
    // written are the `w * h` samples OpenJPEG allocated for each component.
    unsafe {
        let mut params: opj::opj_cparameters_t = std::mem::zeroed();
        opj::opj_set_default_encoder_parameters(&mut params);
        params.tcp_numlayers = e.rates.len() as i32;
        for (i, r) in e.rates.iter().enumerate() {
            params.tcp_rates[i] = *r;
        }
        params.cp_disto_alloc = 1;
        params.irreversible = i32::from(e.irreversible);
        params.tcp_mct = (e.mct && e.comps >= 3) as _;
        params.numresolution = e.numresolution;
        params.cblockw_init = e.cblk.0;
        params.cblockh_init = e.cblk.1;
        params.mode = e.mode;
        params.prog_order = prog_order(e.prog);
        params.csty = e.csty;
        params.subsampling_dx = e.sub.0;
        params.subsampling_dy = e.sub.1;
        params.image_offset_x0 = e.origin.0;
        params.image_offset_y0 = e.origin.1;
        if let Some((tw, th)) = e.tile {
            params.tile_size_on = 1;
            params.cp_tdx = tw;
            params.cp_tdy = th;
            params.cp_tx0 = e.tile_origin.0;
            params.cp_ty0 = e.tile_origin.1;
        }
        if !e.precincts.is_empty() {
            params.csty |= 1;
            params.res_spec = e.precincts.len() as i32;
            for (i, (pw, ph)) in e.precincts.iter().enumerate() {
                params.prcw_init[i] = *pw;
                params.prch_init[i] = *ph;
            }
        }
        if let Some((compno, shift)) = e.roi {
            params.roi_compno = compno;
            params.roi_shift = shift;
        }
        params.numpocs = e.pocs.len() as u32;
        for (i, p) in e.pocs.iter().enumerate() {
            let poc = &mut params.POC[i];
            poc.tile = 1;
            poc.resno0 = p.0;
            poc.compno0 = p.1;
            poc.layno1 = p.2;
            poc.resno1 = p.3;
            poc.compno1 = p.4;
            poc.prg1 = prog_order(p.5);
        }
        let (dx, dy) = (e.sub.0 as u32, e.sub.1 as u32);
        let (x0, y0) = (e.origin.0 as u32, e.origin.1 as u32);
        // the image area on the reference grid, and each component's samples in it
        let (x1, y1) = (x0 + (e.w - 1) * dx + 1, y0 + (e.h - 1) * dy + 1);
        let mut cparms: Vec<opj::opj_image_cmptparm_t> = (0..e.comps)
            .map(|_| {
                let mut p: opj::opj_image_cmptparm_t = std::mem::zeroed();
                p.dx = dx;
                p.dy = dy;
                p.w = e.w;
                p.h = e.h;
                p.x0 = x0;
                p.y0 = y0;
                p.prec = e.prec;
                p.sgnd = u32::from(e.signed);
                p
            })
            .collect();
        let space = if e.comps >= 3 {
            opj::COLOR_SPACE::OPJ_CLRSPC_SRGB
        } else {
            opj::COLOR_SPACE::OPJ_CLRSPC_GRAY
        };
        let image = opj::opj_image_create(e.comps, cparms.as_mut_ptr(), space);
        if image.is_null() {
            return None;
        }
        (*image).x0 = x0;
        (*image).y0 = y0;
        (*image).x1 = x1;
        (*image).y1 = y1;
        for c in 0..e.comps {
            let comp = &*(*image).comps.add(c as usize);
            let plane = std::slice::from_raw_parts_mut(comp.data, (e.w * e.h) as usize);
            for y in 0..e.h {
                for x in 0..e.w {
                    plane[(y * e.w + x) as usize] = sample(x, y, c);
                }
            }
        }
        let codec = opj::opj_create_compress(opj::CODEC_FORMAT::OPJ_CODEC_J2K);
        let mut writer = VecWriter { buf: Vec::new(), pos: 0 };
        let stream = opj::opj_stream_create(1 << 16, 0);
        let mut ok = !codec.is_null() && !stream.is_null();
        if ok {
            opj::opj_stream_set_user_data(stream, &mut writer as *mut VecWriter as *mut c_void, None);
            opj::opj_stream_set_write_function(stream, Some(w_write));
            opj::opj_stream_set_skip_function(stream, Some(w_skip));
            opj::opj_stream_set_seek_function(stream, Some(w_seek));
            ok = opj::opj_setup_encoder(codec, &mut params, image) != 0
                && opj::opj_start_compress(codec, image, stream) != 0
                && opj::opj_encode(codec, stream) != 0
                && opj::opj_end_compress(codec, stream) != 0;
        }
        if !stream.is_null() {
            opj::opj_stream_destroy(stream);
        }
        if !codec.is_null() {
            opj::opj_destroy_codec(codec);
        }
        opj::opj_image_destroy(image);
        ok.then_some(writer.buf)
    }
}

/// A decode of this crate as planes, whatever form its samples took.
pub fn planes_of(image: &aurora_j2k::Image) -> Vec<Plane> {
    match &image.samples {
        aurora_j2k::Samples::Packed(data) => {
            let k = usize::from(image.components);
            (0..k)
                .map(|c| Plane {
                    w: image.width,
                    h: image.height,
                    prec: 8,
                    signed: false,
                    data: data.iter().skip(c).step_by(k).map(|&v| i32::from(v)).collect(),
                })
                .collect()
        }
        aurora_j2k::Samples::Planes(planes) => planes
            .iter()
            .map(|p| Plane {
                w: p.width,
                h: p.height,
                prec: u32::from(p.precision),
                signed: p.signed,
                data: p.data.clone(),
            })
            .collect(),
    }
}

/// How two decodes of one request compare.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The same samples, or both refused.
    Same,
    /// One decoder refused what the other decoded.
    Outcome { oracle: bool, ours: bool },
    /// Different sizes, or different samples: the first component and
    /// sample that differ, and how many do.
    Differ { comp: usize, at: usize, count: usize },
}

/// Whether the codestream's main header chooses the irreversible 9/7
/// wavelet (the transformation byte of its COD marker).
pub fn lossy(cs: &[u8]) -> bool {
    let mut i = 2;
    while i + 4 <= cs.len() && cs[i] == 0xFF {
        let marker = cs[i + 1];
        // (SOT, SOD: the main header is over)
        if marker == 0x90 || marker == 0x93 {
            break;
        }
        if marker == 0x52 {
            return cs.get(i + 13) == Some(&0);
        }
        i += 2 + usize::from(u16::from_be_bytes([cs[i + 2], cs[i + 3]]));
    }
    false
}

/// How far a sample of this decoder may be from OpenJPEG's and still count
/// as the same. Nothing on x86-64, where OpenJPEG's 9/7 wavelet and colour
/// transform run its SSE code, which this decoder follows operation for
/// operation. Elsewhere OpenJPEG runs its plain C code, in which a compiler
/// may fuse a product and a sum into one operation (clang does on AArch64):
/// a rounding then falls the other way now and then, and OpenJPEG's own
/// lossy samples are one unit off its x86-64 ones in a few places. This
/// decoder's are the same on every platform (the test
/// `a_lossy_codestream_decodes_alike_on_every_platform`), so there the
/// comparison allows that unit, on lossy codestreams only.
fn tolerance(cs: &[u8]) -> u64 {
    if cfg!(target_arch = "x86_64") || !lossy(cs) { 0 } else { 1 }
}

/// Compares what both decoders made of `cs` (see [`tolerance`]).
pub fn compare(cs: &[u8], oracle: &Option<Vec<Plane>>, ours: &Result<aurora_j2k::Image, aurora_j2k::Error>) -> Verdict {
    match (oracle, ours) {
        (None, Err(_)) => Verdict::Same,
        (Some(a), Ok(img)) => {
            let b = planes_of(img);
            let mut first = None;
            let mut count = 0;
            let mut furthest = 0u64;
            for (c, (pa, pb)) in a.iter().zip(&b).enumerate() {
                if (pa.w, pa.h) != (pb.w, pb.h) {
                    return Verdict::Differ {
                        comp: c,
                        at: 0,
                        count: pa.data.len().max(1),
                    };
                }
                for (i, (x, y)) in pa.data.iter().zip(&pb.data).enumerate() {
                    if x != y {
                        count += 1;
                        first.get_or_insert((c, i));
                        furthest = furthest.max((i64::from(*x) - i64::from(*y)).unsigned_abs());
                    }
                }
            }
            match first {
                Some((comp, at)) if furthest > tolerance(cs) => Verdict::Differ { comp, at, count },
                _ => Verdict::Same,
            }
        }
        (o, r) => Verdict::Outcome {
            oracle: o.is_some(),
            ours: r.is_ok(),
        },
    }
}
