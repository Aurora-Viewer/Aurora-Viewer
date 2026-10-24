//! Blender parenting becomes an ordinary local linkset: distinct selectable
//! mesh objects with one root (LLSelectMgr::selectObjectAndFamily, llselectmgr.cpp).
use super::{Live, objects::Location, protocol::Prepared};
use crate::world::{World, objects::ObjKey};
use glam::{Mat3, Quat, Vec3};
use uuid::Uuid;

pub(super) struct Row {
    pub source: super::protocol::PreparedSource,
    pub number: usize,
    pub prims: usize,
}

impl Live {
    pub(super) fn linkset_rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else { return Vec::new() };
        let count = |id| snapshot.meshes.iter().filter(|mesh| mesh.source == id).count();
        let mut rows = Vec::new();
        for root in snapshot.sources.iter().filter(|source| source.link.is_none()) {
            let children: Vec<_> = snapshot
                .sources
                .iter()
                .filter(|source| source.link.is_some_and(|link| link.parent == root.id))
                .collect();
            let prims = count(root.id) + children.iter().map(|child| count(child.id)).sum::<usize>();
            rows.push(Row {
                source: root.clone(),
                number: if prims > 1 { 1 } else { 0 },
                prims,
            });
            let mut number = count(root.id) + 1;
            for child in children {
                rows.push(Row {
                    source: child.clone(),
                    number,
                    prims: count(child.id),
                });
                number += count(child.id);
            }
        }
        rows
    }

    pub fn preview_parent(
        &self,
        source: Uuid,
        part: usize,
        rigged: bool,
        avatar: ObjKey,
        roots: &std::collections::HashMap<Uuid, ObjKey>,
    ) -> Option<ObjKey> {
        if rigged {
            return Some(avatar);
        }
        let placement = self.placements.get(&source)?;
        if part != 0 || placement.parent.is_some() {
            // A simulator linkset is flat: every prim, including additional
            // material batches, is a direct child of its single root.
            roots.get(&placement.parent.unwrap_or(source)).copied()
        } else if matches!(placement.location, Location::Attachment(_)) {
            Some(avatar)
        } else {
            None
        }
    }

    pub fn preview_transform(&self, source: Uuid, part: usize) -> Option<(Vec3, Quat)> {
        let placement = self.placements.get(&source)?;
        Some(if part == 0 || placement.parent.is_some() {
            (placement.position, placement.rotation)
        } else {
            (Vec3::ZERO, Quat::IDENTITY)
        })
    }

    pub fn edit_target(&self, source: Uuid, linked: bool) -> Option<Uuid> {
        self.root(if linked { source } else { self.linked_root(source) })
    }

    pub fn linked_root(&self, source: Uuid) -> Uuid {
        self.snapshot
            .as_ref()
            .and_then(|s| s.sources.iter().find(|s| s.id == source))
            .and_then(|s| s.link)
            .map_or(source, |link| link.parent)
    }

    /// Update authored bounds once per source, before installing render batches.
    /// Local edits survive replacement meshes; linking uses Blender's spacing.
    pub fn prepare_placements(&mut self, snapshot: &Prepared, world: &World) {
        let previous = self.placements.clone();
        for source in snapshot.sources.iter().filter(|source| !source.rigged()) {
            let Some(mesh) = snapshot.meshes.iter().find(|mesh| mesh.source == source.id) else {
                continue;
            };
            if let Some(link) = source.link {
                let offset = Vec3::from_array(link.offset);
                let Some(parent) = self.placements.get(&link.parent).copied() else {
                    continue;
                };
                let mut placement = self.ensure_placement(source.id, mesh.size, world);
                if placement.parent == Some(link.parent) {
                    let previous_parent = previous.get(&link.parent).copied().unwrap_or(parent);
                    placement.position += (offset - placement.offset - parent.center + previous_parent.center) * parent.scale
                        + placement.rotation * ((mesh.center - placement.center) * placement.scale);
                } else {
                    placement.position = (offset + mesh.center - parent.center) * parent.scale;
                    placement.rotation = Quat::IDENTITY;
                    placement.scale = parent.scale;
                }
                placement.parent = Some(link.parent);
                placement.offset = offset;
                placement.center = mesh.center;
                placement.location = parent.location;
                self.placements.insert(source.id, placement);
            } else {
                if let Some(old) = previous.get(&source.id)
                    && let Some(parent) = old.parent.and_then(|id| previous.get(&id))
                {
                    // Unparenting keeps the current placement, including a worn
                    // attachment's point frame, rather than jumping to the avatar.
                    let mut placement = *old;
                    placement.position = parent.position + parent.rotation * placement.position;
                    placement.rotation = parent.rotation * placement.rotation;
                    placement.location = parent.location;
                    placement.parent = None;
                    placement.offset = Vec3::ZERO;
                    self.placements.insert(source.id, placement);
                }
                self.update_bounds(source.id, mesh.size, mesh.center, world);
            }
        }
    }

    /// Vertical support for the complete linked set, including rotated children.
    pub fn floor_offset(&self, source: Uuid) -> f32 {
        let source = self.linked_root(source);
        let Some(root) = self.placements.get(&source) else { return 0.0 };
        let mut bottom = -(Mat3::from_quat(root.rotation).abs() * (root.size * root.scale)).z * 0.5;
        for (_, child) in self.placements.iter().filter(|(id, p)| {
            p.parent == Some(source)
                && self.state(**id) == super::SourceState::Visible
                && self.snapshot.as_ref().is_some_and(|s| s.sources.iter().any(|s| s.id == **id))
        }) {
            let rotation = root.rotation * child.rotation;
            let radius = (Mat3::from_quat(rotation).abs() * (child.size * child.scale)).z * 0.5;
            bottom = bottom.min((root.rotation * child.position).z - radius);
        }
        -bottom
    }

    pub fn source_present(&self, source: Uuid) -> bool {
        self.placements
            .get(&self.linked_root(source))
            .is_some_and(|p| p.location != Location::Detached)
    }
}
