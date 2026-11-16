//! The creators' LUT packs built into the viewer: 3D look-up tables that
//! their makers agreed to share with every Aurora user, shown in the photo
//! mode's LUT tab under their name. Each pack is made from the creator's
//! .cube files by `scripts/generate-photo-luts.py` (format in the script);
//! its origin and credit are in `assets/photo-luts/README.md` and NOTICE.md.
//!
//! A look of a pack is saved as `pack:<pack id>/<table name>`. A table is
//! stored compressed (8-bit colours, the precision of the texture the look
//! samples) and turned into a `Lut` the first time it is shown (a few tens
//! of thousands of colours, well under a millisecond) and kept.

use aurora_render::looks::Lut;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// One creator's pack.
pub struct Pack {
    /// Its id in the saved looks.
    pub id: &'static str,
    /// The creator, as they asked to be credited (never translated).
    pub creator: &'static str,
    /// Its tables, in the order of their names.
    pub tables: Vec<Table>,
}

/// A table of a pack: its name and where its compressed colours are.
pub struct Table {
    pub name: String,
    size: u32,
    colours: &'static [u8],
}

/// The packs' files, by id and creator.
const FILES: &[(&str, &str, &[u8])] = &[
    (
        "czar-fenris",
        "Czar Fenris",
        include_bytes!("../../../../assets/photo-luts/czar-fenris.bin"),
    ),
    (
        "nanner-pudding",
        "Nanner Pudding",
        include_bytes!("../../../../assets/photo-luts/nanner-pudding.bin"),
    ),
];

const MAGIC: &[u8] = b"AURLUTS2";

/// The packs, read once.
pub fn packs() -> &'static [Pack] {
    static PACKS: OnceLock<Vec<Pack>> = OnceLock::new();
    PACKS.get_or_init(|| {
        FILES
            .iter()
            .filter_map(|&(id, creator, bytes)| match read(bytes) {
                Some(tables) => Some(Pack { id, creator, tables }),
                None => {
                    log::warn!("photo LUT pack {id}: not read");
                    None
                }
            })
            .collect()
    })
}

/// A pack's tables from its file; None when the file is cut or wrong.
fn read(bytes: &'static [u8]) -> Option<Vec<Table>> {
    let mut rest = bytes.strip_prefix(MAGIC)?;
    let mut take = |n: usize| -> Option<&'static [u8]> {
        let (head, tail) = rest.split_at_checked(n)?;
        rest = tail;
        Some(head)
    };
    let count = u16::from_le_bytes(take(2)?.try_into().ok()?);
    let mut tables = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len = take(1)?[0] as usize;
        let name = std::str::from_utf8(take(len)?).ok()?.to_owned();
        let size = take(1)?[0] as u32;
        if size < 2 {
            return None;
        }
        let packed = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let colours = take(packed as usize)?;
        tables.push(Table { name, size, colours });
    }
    rest.is_empty().then_some(tables)
}

/// The id of a look of a pack.
pub fn look_id(pack: &Pack, table: &Table) -> String {
    format!("{}{}/{}", super::looks::PACK, pack.id, table.name)
}

/// The pack and the table of a look's id (without its `pack:`).
pub fn find(id: &str) -> Option<(&'static Pack, &'static Table)> {
    let (pack, name) = id.split_once('/')?;
    let pack = packs().iter().find(|p| p.id == pack)?;
    let table = pack.tables.iter().find(|t| t.name == name)?;
    Some((pack, table))
}

/// A table of a pack as a `Lut` (by its id without `pack:`), made once.
pub fn table(id: &str) -> Option<Arc<Lut>> {
    static MADE: OnceLock<Mutex<HashMap<String, Arc<Lut>>>> = OnceLock::new();
    let made = MADE.get_or_init(Default::default);
    if let Some(lut) = made.lock().ok()?.get(id) {
        return Some(lut.clone());
    }
    let (_, table) = find(id)?;
    let Some(data) = colours(table) else {
        log::warn!("photo LUT {id}: colours not read");
        return None;
    };
    let lut = Arc::new(Lut { size: table.size, data });
    made.lock().ok()?.insert(id.to_owned(), lut.clone());
    Some(lut)
}

/// A table's colours, inflated and undone from their differences to the
/// previous colour's; None when they are cut or not as many as its size.
fn colours(table: &Table) -> Option<Vec<[f32; 3]>> {
    use std::io::Read;
    let count = (table.size * table.size * table.size) as usize;
    let mut bytes = Vec::with_capacity(count * 3);
    // one byte more than needed: a longer table is refused, not cut
    flate2::read::ZlibDecoder::new(table.colours)
        .take(count as u64 * 3 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() != count * 3 {
        return None;
    }
    let mut prev = [0u8; 3];
    Some(
        bytes
            .as_chunks::<3>()
            .0
            .iter()
            .map(|d| {
                prev = [0, 1, 2].map(|k| prev[k].wrapping_add(d[k]));
                prev.map(|v| v as f32 / 255.0)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_of_every_pack_reads() {
        let packs = packs();
        assert_eq!(packs.len(), FILES.len(), "a pack file did not read");
        let czar = packs.iter().find(|p| p.id == "czar-fenris").expect("Czar Fenris' pack");
        assert_eq!(czar.creator, "Czar Fenris");
        assert_eq!(czar.tables.len(), 20);
        let nanner = packs.iter().find(|p| p.id == "nanner-pudding").expect("Nanner Pudding's pack");
        assert_eq!(nanner.creator, "Nanner Pudding");
        assert_eq!(nanner.tables.len(), 45);
        for pack in packs {
            assert!(!pack.tables.is_empty());
            for t in &pack.tables {
                let id = look_id(pack, t);
                let rest = id.strip_prefix(super::super::looks::PACK).expect("prefix");
                let lut = table(rest).expect("reads");
                assert_eq!(lut.data.len(), (t.size * t.size * t.size) as usize);
                assert!(lut.data.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
            }
        }
        // a table of the pack named in plain words
        assert!(find("czar-fenris/Film look").is_some());
        assert!(find("czar-fenris/Nothing").is_none());
        assert!(find("nanner-pudding/Flip Phone").is_some());
        assert_eq!(find("nanner-pudding/Tis Autumn").map(|(_, t)| t.size), Some(65));
        assert!(find("nobody/Film look").is_none());
    }

    /// A pack of one table "A" of side 2 whose colours are `raw` (already
    /// as differences), compressed as the script does.
    fn pack_of(raw: &[u8]) -> &'static [u8] {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(raw).expect("in memory");
        let colours = z.finish().expect("in memory");
        let mut out = b"AURLUTS2\x01\x00\x01A\x02".to_vec();
        out.extend((colours.len() as u32).to_le_bytes());
        out.extend(colours);
        out.leak()
    }

    #[test]
    fn a_table_is_undone_from_its_differences() {
        // black, then each channel up by 255 / 85 / 1, then wrapping back
        let mut raw = vec![0, 0, 0, 255, 85, 1, 1, 171, 254];
        raw.resize(24, 0);
        let tables = read(pack_of(&raw)).expect("reads");
        let data = colours(&tables[0]).expect("colours");
        assert_eq!(data.len(), 8);
        assert_eq!(data[0], [0.0; 3]);
        assert_eq!(data[1], [1.0, 85.0 / 255.0, 1.0 / 255.0]);
        assert_eq!(data[2], [0.0, 0.0, 1.0]);
        assert_eq!(data[7], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_cut_or_foreign_file_is_refused() {
        let good = pack_of(&[0; 24]);
        assert_eq!(read(good).map(|t| t.len()), Some(1));
        assert!(read(&good[..good.len() - 1]).is_none());
        assert!(read(b"AURLUTS1\x00\x00").is_none());
        static LONGER: &[u8] = b"AURLUTS2\x00\x00\x00";
        assert!(read(LONGER).is_none());
        // colours fewer or more than the table's side asks for
        for raw in [&[0u8; 23][..], &[0u8; 25][..]] {
            let tables = read(pack_of(raw)).expect("the file itself reads");
            assert!(colours(&tables[0]).is_none());
        }
        // colours that are not compressed
        let mut plain = b"AURLUTS2\x01\x00\x01A\x02\x18\x00\x00\x00".to_vec();
        plain.extend([7u8; 24]);
        let tables = read(plain.leak()).expect("the file itself reads");
        assert!(colours(&tables[0]).is_none());
    }
}
