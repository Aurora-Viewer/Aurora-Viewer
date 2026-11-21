//! The lattice a cloth is simulated on (Aurora's own; no Legacy Viewer
//! counterpart). A worn mesh has far more vertices than its motion needs: a
//! dress in many pieces had 60 000 moving nodes, two hairs 21 000, stepped
//! on one core up to eight times a frame. Its nodes are gathered into cells
//! of about `spacing`, one simulated point each, and every node follows the
//! points of its cell and of the cells beside it (as Unreal's cloth LODs and
//! the skinned proxies of HDT-SMP do): the shape the creator modelled stays,
//! only the motion is coarser.
//!
//! A cell holds the nodes of one voxel of the rest pose that are joined by
//! the mesh's edges inside it: two hair cards crossing the same voxel stay
//! two points and never move as one, nor does a node follow a point it is
//! not joined to. Pinned and moving nodes are never in one cell: what is
//! pinned stays exactly on the body.

use glam::Vec3;
use std::collections::{HashMap, HashSet};

/// The lattice of a cloth (see the module).
#[derive(Debug, Default)]
pub struct Lattice {
    /// Per node, its cell.
    pub cell_of: Vec<u32>,
    /// Per cell, the node it is simulated as: the nearest to the cell's
    /// middle (its skin, bind place and normal are the cell's).
    pub rep: Vec<u32>,
    /// Per cell, its nodes' mean freedom (pinned cells: 0).
    pub free: Vec<f32>,
    pub free_still: Vec<f32>,
    /// Between cells whose nodes an edge joins, at their rest distance.
    pub edges: Vec<(u32, u32, f32)>,
    /// Per node, where its links start in `links` (one more entry than nodes).
    pub start: Vec<u32>,
    /// The cells a moving node follows, with weights summing to 1 (a pinned
    /// node follows none: it stays on its skinning).
    pub links: Vec<(u32, f32)>,
}

impl Lattice {
    /// The links of node `f`.
    pub fn links_of(&self, f: usize) -> &[(u32, f32)] {
        match (self.start.get(f), self.start.get(f + 1)) {
            (Some(&a), Some(&b)) => self.links.get(a as usize..b as usize).unwrap_or(&[]),
            _ => &[],
        }
    }
}

fn find(parent: &mut [u32], mut u: u32) -> u32 {
    while parent[u as usize] != u {
        let up = parent[parent[u as usize] as usize];
        parent[u as usize] = up;
        u = up;
    }
    u
}

/// The lattice of nodes at `rest` (the rest pose) joined by `edges`, with
/// their freedom (`free`, `free_still`), cells of about `spacing`. Each of
/// `pieces` (small solid pieces: a buckle, a gem) all moving is one cell
/// whatever its size, its nodes following that cell alone: it moves as one,
/// never bending.
pub fn build(rest: &[Vec3], free: &[f32], free_still: &[f32], edges: &[(u32, u32, f32)], pieces: &[Vec<u32>], spacing: f32) -> Lattice {
    let n = rest.len();
    let spacing = spacing.max(1e-4);
    let pinned = |u: usize| free.get(u).is_none_or(|&f| f <= 0.0);
    let voxel = |u: usize| {
        let c = (rest[u] / spacing).floor();
        (c.x as i32, c.y as i32, c.z as i32, pinned(u))
    };
    let keys: Vec<(i32, i32, i32, bool)> = (0..n).map(voxel).collect();
    // the nodes of the solid pieces all moving (a piece touching the body
    // follows its skin already)
    let mut solid = vec![false; n];
    for m in pieces {
        if m.iter().all(|&u| (u as usize) < n && !pinned(u as usize)) {
            for &u in m {
                solid[u as usize] = true;
            }
        }
    }
    // the nodes of a voxel joined inside it (not across into a solid piece:
    // a buckle stitched to its belt stays a cell of its own)
    let mut parent: Vec<u32> = (0..n as u32).collect();
    for &(a, b, _) in edges {
        let (a, b) = (a as usize, b as usize);
        if a < n && b < n && keys[a] == keys[b] && solid[a] == solid[b] {
            let (ra, rb) = (find(&mut parent, a as u32), find(&mut parent, b as u32));
            if ra != rb {
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
    }
    // each solid piece all moving: one cell of its own
    for m in pieces
        .iter()
        .filter(|m| m.iter().all(|&u| solid.get(u as usize).copied().unwrap_or(false)))
    {
        for w in m.windows(2) {
            let (ra, rb) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            if ra != rb {
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
    }
    let mut cell_of = vec![0u32; n];
    let mut id_of: HashMap<u32, u32> = HashMap::new();
    let mut members: Vec<Vec<u32>> = Vec::new();
    for u in 0..n as u32 {
        let r = find(&mut parent, u);
        let id = *id_of.entry(r).or_insert_with(|| {
            members.push(Vec::new());
            (members.len() - 1) as u32
        });
        cell_of[u as usize] = id;
        members[id as usize].push(u);
    }
    let cells = members.len();
    let mut rep = Vec::with_capacity(cells);
    let mut cell_free = Vec::with_capacity(cells);
    let mut cell_still = Vec::with_capacity(cells);
    for m in &members {
        let mid = m.iter().map(|&u| rest[u as usize]).sum::<Vec3>() / m.len() as f32;
        let r = m
            .iter()
            .copied()
            .min_by(|&a, &b| {
                (rest[a as usize] - mid)
                    .length_squared()
                    .total_cmp(&(rest[b as usize] - mid).length_squared())
            })
            .unwrap_or(0);
        rep.push(r);
        let mean = |v: &[f32]| m.iter().map(|&u| v.get(u as usize).copied().unwrap_or(0.0).max(0.0)).sum::<f32>() / m.len() as f32;
        cell_free.push(mean(free));
        cell_still.push(mean(free_still));
    }
    // the cells' edges, and each cell's neighbours
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    let mut cell_edges = Vec::new();
    let mut beside: Vec<Vec<u32>> = vec![Vec::new(); cells];
    for &(a, b, _) in edges {
        let (Some(&ca), Some(&cb)) = (cell_of.get(a as usize), cell_of.get(b as usize)) else {
            continue;
        };
        if ca == cb || !seen.insert((ca.min(cb), ca.max(cb))) {
            continue;
        }
        beside[ca as usize].push(cb);
        beside[cb as usize].push(ca);
        if cell_free[ca as usize] > 0.0 || cell_free[cb as usize] > 0.0 {
            let d = (rest[rep[ca as usize] as usize] - rest[rep[cb as usize] as usize]).length();
            cell_edges.push((ca.min(cb), ca.max(cb), d));
        }
    }
    // each moving node: its cell's point and those beside it, by a smooth
    // falloff of their distance (the surface bends evenly between points)
    let reach = 2.0 * spacing;
    let mut start = Vec::with_capacity(n + 1);
    let mut links = Vec::new();
    for u in 0..n {
        start.push(links.len() as u32);
        if pinned(u) {
            continue;
        }
        let own = cell_of[u];
        if solid[u] {
            links.push((own, 1.0));
            continue;
        }
        let from = links.len();
        let mut sum = 0.0;
        for c in std::iter::once(own).chain(beside[own as usize].iter().copied()) {
            let d = (rest[u] - rest[rep[c as usize] as usize]).length() / reach;
            let w = (1.0 - d * d).max(0.0).powi(2);
            if w > 0.0 {
                links.push((c, w));
                sum += w;
            }
        }
        if sum <= 1e-6 {
            links.truncate(from);
            links.push((own, 1.0));
        } else {
            for l in &mut links[from..] {
                l.1 /= sum;
            }
        }
    }
    start.push(links.len() as u32);
    Lattice {
        cell_of,
        rep,
        free: cell_free,
        free_still: cell_still,
        edges: cell_edges,
        start,
        links,
    }
}

#[cfg(test)]
#[path = "coarse_tests.rs"]
mod tests;
