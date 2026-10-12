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
                        *joint = *index as u8;
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
