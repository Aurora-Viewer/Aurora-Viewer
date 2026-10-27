//! LLModelPreview::genBuffers / LLModel::getJointInfluences: LODs use the
//! high model's joint table and nearest source weights where none exist.
//! A balanced spatial tree avoids a quadratic search on large meshes.

use super::*;

struct Node<'a> {
    vertex: &'a Vertex,
    axis: usize,
    left: Option<usize>,
    right: Option<usize>,
}
/// LLGLTFLoader::populateModelFromMesh / FIRE-36838: supported joints keep
/// source order for small palettes. Large palettes put weighted joints first,
/// then keep unused joints in active skeletal groups (maximum 110).
pub fn prune_gltf(faces: &mut [Face], skin: &mut Skin, skeleton: &crate::Skeleton) -> Result<(), AssetError> {
    if skin.inverse_bind.len() != skin.names.len() || skin.alternate_bind.len() != skin.names.len() {
        return Err(AssetError::invalid("skin bind matrix count"));
    }
    let supported: Vec<_> = skin.names.iter().map(|name| skeleton.find(name)).collect();
    let mut used = vec![false; skin.names.len()];
    for vertex in faces.iter().flat_map(|face| &face.vertices) {
        for (joint, weight) in vertex.joints.iter().zip(vertex.weights) {
            if !weight.is_finite() || weight < 0.0 {
                return Err(AssetError::invalid("negative or nonfinite skin weight"));
            }
            if weight > 0.0 && supported.get(*joint as usize).is_some_and(Option::is_some) {
                used[*joint as usize] = true;
            }
        }
    }
    let mut selected = Vec::new();
    if supported.iter().filter(|joint| joint.is_some()).count() <= 110 {
        selected.extend(supported.iter().enumerate().filter_map(|(i, joint)| joint.map(|_| i)));
    } else {
        let mut active = std::collections::HashSet::from(["Torso"]);
        for (i, joint) in supported.iter().enumerate() {
            if used[i] {
                let bone = &skeleton.joints[joint.expect("supported weighted joint")];
                active.insert(bone.group.as_str());
                if let Some(parent) = bone.parent {
                    active.insert(skeleton.joints[parent].group.as_str());
                }
                selected.push(i);
            }
        }
        if selected.len() > 110 {
            return Err(AssetError::TooLarge("weighted mesh joints"));
        }
        for (i, joint) in supported.iter().enumerate() {
            if selected.len() == 110 {
                break;
            }
            if !used[i] && joint.is_some_and(|j| active.contains(skeleton.joints[j].group.as_str())) {
                selected.push(i);
            }
        }
    }
    let mut remap = vec![None; skin.names.len()];
    for (index, source) in selected.iter().enumerate() {
        remap[*source] = Some(index as u16);
    }
    for vertex in faces.iter_mut().flat_map(|face| &mut face.vertices) {
        for (joint, weight) in vertex.joints.iter_mut().zip(&mut vertex.weights) {
            if let Some(Some(mapped)) = remap.get(*joint as usize) {
                *joint = *mapped;
            } else {
                *joint = 0;
                *weight = 0.0;
            }
        }
        let sum: f32 = vertex.weights.iter().sum();
        if sum > 0.0 {
            for weight in &mut vertex.weights {
                *weight /= sum;
            }
        }
    }
    skin.names = selected.iter().map(|i| skin.names[*i].clone()).collect();
    skin.inverse_bind = selected.iter().map(|i| skin.inverse_bind[*i]).collect();
    skin.alternate_bind = selected.iter().map(|i| skin.alternate_bind[*i]).collect();
    Ok(())
}
fn tree<'a>(points: &mut [&'a Vertex], depth: usize, nodes: &mut Vec<Node<'a>>) -> Option<usize> {
    if points.is_empty() {
        return None;
    }
    let axis = depth % 3;
    let mid = points.len() / 2;
    points.select_nth_unstable_by(mid, |a, b| a.position[axis].total_cmp(&b.position[axis]));
    let (left, rest) = points.split_at_mut(mid);
    let (point, right) = rest.split_at_mut(1);
    let index = nodes.len();
    nodes.push(Node {
        vertex: point[0],
        axis,
        left: None,
        right: None,
    });
    nodes[index].left = tree(left, depth + 1, nodes);
    nodes[index].right = tree(right, depth + 1, nodes);
    Some(index)
}
fn nearest(nodes: &[Node<'_>], index: usize, point: Vec3, best: &mut (usize, f32)) {
    let node = &nodes[index];
    let position = Vec3::from_array(node.vertex.position);
    let distance = point.distance_squared(position);
    if distance < best.1 {
        *best = (index, distance);
    }
    let delta = point[node.axis] - position[node.axis];
    let (first, second) = if delta < 0.0 {
        (node.left, node.right)
    } else {
        (node.right, node.left)
    };
    if let Some(index) = first {
        nearest(nodes, index, point, best);
    }
    if delta * delta <= best.1
        && let Some(index) = second
    {
        nearest(nodes, index, point, best);
    }
}

/// LLModel::getJointInfluences (indra/llprimitive/llmodel.cpp, originally
/// LGPL 2.1): weights belong to a position, not to a vertex. Each vertex
/// takes the list of the first weighted position, in LLVector3 order,
/// within 1e-5 of it in normalized space. Layers that share their
/// positions (the onion skins of a mesh body) then skin identically,
/// in the preview and in the upload, instead of z-fighting by rounding.
/// A weighted position always finds itself; unweighted ones far from any
/// weighted position are left to `complete_weights`.
pub fn share_weights(faces: &mut [Face], center: Vec3, size: Vec3) {
    use rayon::prelude::*;
    use std::collections::HashMap;
    const EPSILON: f32 = 1e-5;
    const CELL: f32 = EPSILON * 2.0;
    let normalized = |vertex: &Vertex| (Vec3::from_array(vertex.position) - center) / size;
    // LLDAELoader / LLGLTFLoader fill a map keyed by position: the last
    // vertex written at an exact position wins.
    let mut exact = HashMap::new();
    for vertex in faces.iter().flat_map(|face| &face.vertices) {
        if vertex.weights.iter().sum::<f32>() > 0.0 {
            let point = normalized(vertex);
            exact.insert(
                (point + Vec3::ZERO).to_array().map(f32::to_bits),
                (point, vertex.joints, vertex.weights),
            );
        }
    }
    if exact.is_empty() {
        return;
    }
    let keys: Vec<_> = exact.into_values().collect();
    let cell = |value: f32| (value / CELL).floor() as i64;
    let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (index, (point, ..)) in keys.iter().enumerate() {
        grid.entry(point.to_array().map(cell)).or_default().push(index);
    }
    let before = |a: Vec3, b: Vec3| (a.x, a.y, a.z) < (b.x, b.y, b.z);
    // A mesh body is a few huge faces: spread the vertices, not the faces.
    for face in faces.iter_mut() {
        face.vertices.par_iter_mut().for_each(|vertex| {
            let point = normalized(vertex);
            let low = (point - EPSILON).to_array().map(cell);
            let high = (point + EPSILON).to_array().map(cell);
            let mut best: Option<usize> = None;
            for x in low[0]..=high[0] {
                for y in low[1]..=high[1] {
                    for z in low[2]..=high[2] {
                        for &index in grid.get(&[x, y, z]).into_iter().flatten() {
                            let key = keys[index].0;
                            if key.distance(point) < EPSILON && best.is_none_or(|best| before(key, keys[best].0)) {
                                best = Some(index);
                            }
                        }
                    }
                }
            }
            if let Some(best) = best {
                vertex.joints = keys[best].1;
                vertex.weights = keys[best].2;
            }
        });
    }
}

/// LLModel::getJointInfluences / LLModelPreview::genBuffers: even High
/// inherits the nearest available weights when an exporter helper joint
/// was stripped. Search the whole source in its normalized metric before
/// splitting models; leaving zero weights mixes normalized and posed space.
pub fn complete_weights(faces: &mut [Face], normalized_scale: Vec3) {
    if !faces
        .iter()
        .flat_map(|face| &face.vertices)
        .any(|v| v.weights.iter().sum::<f32>() <= 0.0)
    {
        return;
    }
    let sources: Vec<_> = faces
        .iter()
        .flat_map(|face| &face.vertices)
        .filter(|v| v.weights.iter().sum::<f32>() > 0.0)
        .map(|v| {
            let mut source = v.clone();
            source.position = (Vec3::from_array(source.position) / normalized_scale).to_array();
            source
        })
        .collect();
    let mut points: Vec<_> = sources.iter().collect();
    let mut nodes = Vec::with_capacity(points.len());
    let Some(root) = tree(&mut points, 0, &mut nodes) else { return };
    for vertex in faces
        .iter_mut()
        .flat_map(|face| &mut face.vertices)
        .filter(|v| v.weights.iter().sum::<f32>() <= 0.0)
    {
        let mut best = (root, f32::MAX);
        nearest(&nodes, root, Vec3::from_array(vertex.position) / normalized_scale, &mut best);
        let source = nodes[best.0].vertex;
        vertex.joints = source.joints;
        vertex.weights = source.weights;
    }
}

pub fn transfer_weights(model: &mut Model, high: &Model) -> Result<(), AssetError> {
    let Some(skin) = &high.skin else { return Ok(()) };
    let remap = model.skin.as_ref().map(|s| {
        s.names
            .iter()
            .map(|name| skin.names.iter().position(|n| n == name))
            .collect::<Vec<_>>()
    });
    for vertex in model.faces.iter_mut().flat_map(|f| &mut f.vertices) {
        if let Some(remap) = &remap {
            for (joint, weight) in vertex.joints.iter_mut().zip(&mut vertex.weights) {
                if *weight > 0.0 {
                    if let Some(Some(index)) = remap.get(*joint as usize) {
                        *joint = *index as u16;
                    } else {
                        *weight = 0.0;
                    }
                }
            }
        } else {
            vertex.weights = [0.0; 4];
        }
        let sum = vertex.weights.iter().sum::<f32>();
        if sum > 0.0 {
            for weight in &mut vertex.weights {
                *weight /= sum;
            }
        }
    }
    if model
        .faces
        .iter()
        .flat_map(|f| &f.vertices)
        .any(|v| v.weights.iter().sum::<f32>() <= 0.0)
    {
        let mut points: Vec<_> = high
            .faces
            .iter()
            .flat_map(|f| &f.vertices)
            .filter(|v| v.weights.iter().sum::<f32>() > 0.0)
            .collect();
        let mut nodes = Vec::with_capacity(points.len());
        let root = tree(&mut points, 0, &mut nodes).ok_or_else(|| AssetError::invalid("high LOD has no skin weights"))?;
        for vertex in model
            .faces
            .iter_mut()
            .flat_map(|f| &mut f.vertices)
            .filter(|v| v.weights.iter().sum::<f32>() <= 0.0)
        {
            let mut best = (root, f32::MAX);
            nearest(&nodes, root, Vec3::from_array(vertex.position), &mut best);
            let source = nodes[best.0].vertex;
            vertex.joints = source.joints;
            vertex.weights = source.weights;
        }
    }
    model.skin = Some(skin.clone());
    Ok(())
}
