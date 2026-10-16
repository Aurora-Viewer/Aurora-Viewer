//! Order of the blended faces of a frame.
//!
//! The Legacy Viewer draws the blended faces of rigged meshes (worn rigged
//! meshes, animesh) in a pass of their own before the other blended faces,
//! writing depth (LLDrawPoolAlpha::render: forwardRender(true), then
//! forwardRender(); write_depth = rigged). A layer in front, mesh hair, then
//! hides what is behind it, eyelashes, whichever of the two is drawn first:
//! drawn first it rejects the lashes, drawn second it covers them. Sorted
//! only by the distance of their objects, the lashes (a small object in
//! front of the face) came after the hair (centred on the head) and showed
//! through it. The rigged faces come first here, back to front among
//! themselves, then the others back to front; the renderer draws the first
//! group writing depth (`DrawLists::blend_depth`).

use aurora_render::DrawCmd;

/// One blended face of the frame.
#[derive(Clone, Copy, Debug)]
pub struct BlendEntry {
    /// Distance from the eye to the centre of its object.
    pub distance: f32,
    pub cmd: DrawCmd,
    /// The face has glow (drawn into the glow right after it).
    pub glow: bool,
    /// The face belongs to a rigged mesh.
    pub rigged: bool,
}

/// AURORA_RIGGED_BLEND: `sorted`, the rigged faces sorted with the others
/// and drawn without depth, as before; `alt`, that one second in three
/// (`xmode=old` on the profile lines); otherwise apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Apart,
    Sorted,
    Alternate,
}

pub fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_RIGGED_BLEND").as_deref().map(str::trim) {
        Ok("sorted") => Mode::Sorted,
        Ok("alt") => Mode::Alternate,
        _ => Mode::Apart,
    })
}

/// With `alt`: the current second is drawn the former way.
static SORTED_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_sorted_now(sorted: bool) {
    SORTED_NOW.store(sorted, std::sync::atomic::Ordering::Relaxed);
}

/// Whether this frame draws the rigged faces apart, writing depth.
pub fn rigged_apart() -> bool {
    match mode() {
        Mode::Apart => true,
        Mode::Sorted => false,
        Mode::Alternate => !SORTED_NOW.load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// Sort `entries` into drawing order and return how many of them, at the
/// start, are drawn writing depth: the rigged faces back to front, then the
/// others back to front (all of them back to front, none apart, when
/// `rigged_apart` is false). Faces at the same distance keep one order from
/// frame to frame (by record, then first index): the lists are gathered in
/// parallel, in no fixed order, and an unstable tie would flicker.
pub fn sort(entries: &mut [BlendEntry], rigged_apart: bool) -> usize {
    entries.sort_unstable_by(|a, b| {
        let group = |e: &BlendEntry| !(rigged_apart && e.rigged);
        group(a)
            .cmp(&group(b))
            .then(b.distance.total_cmp(&a.distance))
            .then(a.cmd.record.cmp(&b.cmd.record))
            .then(a.cmd.first_index.cmp(&b.cmd.first_index))
    });
    if rigged_apart {
        entries.iter().take_while(|e| e.rigged).count()
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(record: u32, distance: f32, rigged: bool) -> BlendEntry {
        BlendEntry {
            distance,
            cmd: DrawCmd {
                index_count: 3,
                first_index: 0,
                base_vertex: 0,
                record,
                bounds: [0.0; 4],
            },
            glow: false,
            rigged,
        }
    }

    fn records(e: &[BlendEntry]) -> Vec<u32> {
        e.iter().map(|e| e.cmd.record).collect()
    }

    /// Mesh hair centred on the head (farther) and eyelashes in front of the
    /// face (nearer), both rigged, with a window and a particle-like card of
    /// the scene: the rigged faces first, back to front, then the others.
    #[test]
    fn rigged_faces_come_first_back_to_front_then_the_others() {
        let mut e = [
            entry(1, 5.0, false), // window
            entry(2, 3.10, true), // eyelashes
            entry(3, 9.0, false), // glass far away
            entry(4, 3.20, true), // hair
        ];
        assert_eq!(sort(&mut e, true), 2);
        assert_eq!(records(&e), [4, 2, 3, 1]);
    }

    #[test]
    fn sorted_mode_keeps_one_back_to_front_list_without_depth() {
        let mut e = [
            entry(1, 5.0, false),
            entry(2, 3.10, true),
            entry(3, 9.0, false),
            entry(4, 3.20, true),
        ];
        assert_eq!(sort(&mut e, false), 0);
        assert_eq!(records(&e), [3, 1, 4, 2]);
    }

    /// Faces of one rigged mesh share its distance: their order must not
    /// depend on the order they were gathered in.
    #[test]
    fn faces_at_the_same_distance_keep_a_fixed_order() {
        let mut a = [entry(9, 3.0, true), entry(4, 3.0, true), entry(7, 3.0, true), entry(5, 6.0, false)];
        let mut b = [entry(7, 3.0, true), entry(5, 6.0, false), entry(9, 3.0, true), entry(4, 3.0, true)];
        assert_eq!(sort(&mut a, true), 3);
        assert_eq!(sort(&mut b, true), 3);
        assert_eq!(records(&a), [4, 7, 9, 5]);
        assert_eq!(records(&a), records(&b));
    }

    #[test]
    fn no_rigged_face_means_nothing_drawn_with_depth() {
        let mut e = [entry(1, 2.0, false), entry(2, 4.0, false)];
        assert_eq!(sort(&mut e, true), 0);
        assert_eq!(records(&e), [2, 1]);
    }
}
