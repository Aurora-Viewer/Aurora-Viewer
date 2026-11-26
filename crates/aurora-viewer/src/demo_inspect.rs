//! Violet Wolf's outfit for AURORA_DEMO_INSPECT: small worn prims, some
//! linked, named and described like store items and credited to the demo
//! residents, so that Inspect shows several body areas and creators.

use super::{prim, shape, te};
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
use glam::{Quat, Vec3};
use uuid::Uuid;

/// One worn object: local id, attachment point, name, description, creator
/// (0: Violet Wolf, 1: Nova, 2: the friend, 3: the demo agent), linked prims.
type Piece = (u32, u8, &'static str, &'static str, u8, &'static [&'static str]);

const OUTFIT: &[Piece] = &[
    (
        9110,
        2,
        "Aster Head 3.1",
        "Bento head. Skin and make-up change with its HUD.",
        1,
        &["Teeth", "Tongue"],
    ),
    (
        9111,
        2,
        "Sorrel Bob — Ink",
        "Rigged mesh hair. 30 colours in the HUD.",
        2,
        &["Bangs"],
    ),
    (9112, 13, "Halo Drop Earrings", "Gold and pearl.", 2, &["Drop R"]),
    (9113, 1, "Ribbed Knit Crop", "Ten knits in the HUD.", 3, &["Sleeves"]),
    (
        9114,
        9,
        "Cropped Moto Jacket",
        "Leather and suede, 12 colours.",
        2,
        &["Collar", "Zip"],
    ),
    (9115, 39, "Thin Chain Pendant", "", 2, &[]),
    (9116, 42, "Stack Rings", "", 2, &["Ring 2", "Ring 3"]),
    (9117, 3, "Mini Saddle Bag", "The strap length is set from its menu.", 3, &["Strap"]),
    (9118, 10, "Pleated Mini", "", 3, &[]),
    (9119, 7, "Riding Boots", "For high feet.", 1, &["Boot R", "Laces"]),
];

/// The wolf's own hat and orb (`wolf_attachments`), named for the scenario.
const WOLF_OWN: &[(u32, &str, &str)] = &[(9100, "Violet Top Hat", "A demo hat."), (9101, "Glow Orb", "Glows in the dark.")];

pub fn enabled() -> bool {
    std::env::var_os("AURORA_DEMO_INSPECT").is_some()
}

fn creator(index: u8) -> Uuid {
    match index {
        1 => super::DEMO_NOVA,
        2 => super::DEMO_FRIEND,
        3 => super::DEMO_AGENT,
        _ => super::DEMO_LOUP,
    }
}

/// Local id of the n-th linked prim of a piece.
fn child_id(root: u32, n: usize) -> u32 {
    root * 10 + 1 + n as u32
}

/// The outfit's prims, worn by Violet Wolf (local id 9001).
pub fn outfit() -> Vec<ObjectUpdate> {
    let cube = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut out = Vec::new();
    for (i, (id, point, _, _, _, children)) in OUTFIT.iter().enumerate() {
        let shade = 0.35 + 0.05 * i as f32;
        let mut root = prim(
            *id,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::splat(0.04),
            cube,
            te([shade, 0.3, 0.6, 1.0], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        root.parent_id = 9001;
        root.state = ((point & 0x0F) << 4) | ((point & 0xF0) >> 4);
        out.push(root);
        for n in 0..children.len() {
            let pos = Vec3::new(0.0, 0.0, 0.03 * (n + 1) as f32);
            let mut child = prim(
                child_id(*id, n),
                pos,
                Quat::IDENTITY,
                Vec3::splat(0.03),
                cube,
                te([shade, 0.3, 0.6, 1.0], 0, false, 0.0),
                ExtraParams::default(),
                "",
            );
            child.parent_id = *id;
            out.push(child);
        }
    }
    out
}

/// Name, description and creator of a prim of the outfit, by local id.
pub fn props(local_id: u32) -> Option<(&'static str, &'static str, Uuid)> {
    if !enabled() {
        return None;
    }
    if let Some((_, name, desc)) = WOLF_OWN.iter().find(|w| w.0 == local_id) {
        return Some((name, desc, super::DEMO_LOUP));
    }
    OUTFIT.iter().find_map(|(id, _, name, desc, who, children)| {
        if *id == local_id {
            return Some((*name, *desc, creator(*who)));
        }
        children
            .iter()
            .enumerate()
            .find(|(n, _)| child_id(*id, *n) == local_id)
            .map(|(_, child)| (*child, "", creator(*who)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_prims_keep_their_own_ids_and_names() {
        assert_eq!(child_id(9110, 0), 91101);
        // (the ids of the outfit's linked prims are free in the demo)
        let ids: Vec<u32> = OUTFIT.iter().flat_map(|p| (0..p.5.len()).map(move |n| child_id(p.0, n))).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }
}
