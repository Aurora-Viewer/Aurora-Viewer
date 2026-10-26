//! LLGLTFLoader::buildOverrideMatrix, computeGltfToViewerSkeletonTransform
//! and checkForXYrotation (indra/newview/gltf/llgltfloader.cpp, LGPL 2.1).
//! Resolve hierarchy once, then translate authored rest/bind frames to the
//! viewer skeleton. Joint-position imports never replace SL bone rotations.

use super::{AssetError, Skin};
use crate::{Skeleton, import_gltf::Document, skeleton::maya_q_xyz};
use glam::{Mat4, Vec3};
use serde_json::Value;
use std::collections::HashMap;

#[cfg(test)]
pub(super) fn globals(nodes: &[Value]) -> Result<Vec<Mat4>, AssetError> {
    globals_for(nodes, &(0..nodes.len()).collect::<Vec<_>>())
}

pub(super) fn globals_for(nodes: &[Value], required: &[usize]) -> Result<Vec<Mat4>, AssetError> {
    let mut parents = vec![None; nodes.len()];
    for (parent, node) in nodes.iter().enumerate() {
        for child in node["children"].as_array().into_iter().flatten() {
            let child = super::gltf::integer(child, "child node index")?;
            let slot = parents.get_mut(child).ok_or_else(|| AssetError::invalid("child node index"))?;
            if slot.replace(parent).is_some() {
                return Err(AssetError::invalid("glTF node has multiple parents"));
            }
        }
    }
    let mut global = vec![Mat4::IDENTITY; nodes.len()];
    let mut state = vec![0u8; nodes.len()];
    for &index in required {
        if index >= nodes.len() {
            return Err(AssetError::invalid("glTF node index"));
        }
        let mut chain = Vec::new();
        let mut current = Some(index);
        while let Some(node) = current {
            match state[node] {
                2 => break,
                1 => return Err(AssetError::invalid("cyclic glTF hierarchy")),
                _ => {}
            }
            state[node] = 1;
            chain.push(node);
            current = parents[node];
        }
        for node in chain.into_iter().rev() {
            global[node] = parents[node].map_or(Mat4::IDENTITY, |p| global[p]) * super::gltf::transform(&nodes[node])?;
            if !global[node].is_finite() || global[node].determinant().abs() <= f32::MIN_POSITIVE {
                return Err(AssetError::invalid("singular glTF hierarchy transform"));
            }
            state[node] = 2;
        }
    }
    Ok(global)
}

pub(super) fn basis(doc: &Document, nodes: &[Value], globals: &[Mat4], skeleton: &Skeleton) -> Result<Mat4, AssetError> {
    let basis = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let skin = &doc.json["skins"][0];
    let Some(accessor) = skin["inverseBindMatrices"].as_u64() else {
        return Ok(basis);
    };
    let Some(joints) = skin["joints"].as_array() else {
        return Ok(basis);
    };
    let matrices = super::gltf::attribute::<16>(doc, accessor as usize)?;
    let mut shoulders = [false; 2];
    for (i, joint) in joints.iter().enumerate() {
        let node = super::gltf::integer(joint, "skin joint index")?;
        let value = nodes.get(node).ok_or_else(|| AssetError::invalid("skin joint index"))?;
        let Some(joint) = value["name"].as_str().and_then(|name| skeleton.find(name)) else {
            continue;
        };
        let slot = match skeleton.joints[joint].name.as_str() {
            "mShoulderRight" => 0,
            "mShoulderLeft" => 1,
            _ => continue,
        };
        let inverse = matrices.get(i).ok_or_else(|| AssetError::invalid("skin bind matrix count"))?;
        let test = globals[node].inverse() * Mat4::from_cols_array(inverse);
        if [test.x_axis.x, test.y_axis.y, test.z_axis.z].iter().any(|n| n.abs() >= 0.5) {
            return Ok(basis);
        }
        shoulders[slot] = true;
    }
    Ok(if shoulders == [true; 2] {
        Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2) * basis
    } else {
        basis
    })
}

pub(super) fn read(
    doc: &Document,
    index: usize,
    nodes: &[Value],
    globals: &[Mat4],
    basis: Mat4,
    skeleton: &Skeleton,
    warnings: &mut Vec<String>,
) -> Result<Skin, AssetError> {
    let skin = doc.json["skins"].get(index).ok_or_else(|| AssetError::invalid("skin index"))?;
    let joints = skin["joints"].as_array().ok_or_else(|| AssetError::invalid("skin has no joints"))?;
    if joints.is_empty() || joints.len() > u16::MAX as usize {
        return Err(AssetError::TooLarge("skin joints"));
    }
    let inverse = skin["inverseBindMatrices"]
        .as_u64()
        .map(|i| super::gltf::attribute::<16>(doc, i as usize))
        .transpose()?;
    if inverse.as_ref().is_some_and(|m| m.len() != joints.len()) {
        return Err(AssetError::invalid("inverse bind matrix count"));
    }
    let indices = joints
        .iter()
        .map(|j| super::gltf::integer(j, "skin joint index"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = Vec::new();
    let mut named_nodes = HashMap::new();
    for &index in &indices {
        let node = nodes.get(index).ok_or_else(|| AssetError::invalid("skin joint index"))?;
        let name = node["name"]
            .as_str()
            .ok_or_else(|| AssetError::invalid("skin joint needs a name"))?;
        if let Some(joint) = skeleton.find(name) {
            named_nodes.insert(joint, index);
            names.push(skeleton.joints[joint].name.clone());
        } else {
            warnings.push(format!("Unknown joint: {name}"));
            names.push(name.to_owned());
        }
    }
    let mut rests = vec![Mat4::IDENTITY; skeleton.joints.len()];
    let mut support_rests = rests.clone();
    let mut overrides = HashMap::new();
    for (index, joint) in skeleton.joints.iter().enumerate() {
        let parent_rest = joint.parent.map_or(Mat4::IDENTITY, |p| rests[p]);
        let support_rest = joint.parent.map_or(Mat4::IDENTITY, |p| support_rests[p]);
        if let Some(&node) = named_nodes.get(&index) {
            let parent = if joint.is_collision_volume { support_rest } else { parent_rest };
            let translation = (parent.inverse() * basis * globals[node]).w_axis.truncate();
            let local = Mat4::from_translation(translation);
            rests[index] = parent_rest * local;
            let rest = if joint.is_collision_volume {
                support_rest * local * Mat4::from_scale(joint.scale)
            } else {
                rests[index]
            };
            overrides.insert(node, (rest, translation));
        } else {
            rests[index] = parent_rest * Mat4::from_scale_rotation_translation(joint.scale, maya_q_xyz(joint.rot_euler_deg), joint.pos);
        }
        support_rests[index] = if joint.support == "base" { rests[index] } else { support_rest };
    }
    // The legacy conversion intentionally uses the ordinary Y/Z basis here;
    // the detected XY correction is already present in the override rest.
    let ordinary_basis = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let mut inverse_bind = Vec::new();
    let mut alternate_bind = Vec::new();
    for (i, &node) in indices.iter().enumerate() {
        let (matrix, translation) = if let Some((rest, translation)) = overrides.get(&node) {
            let change = *rest * (ordinary_basis * globals[node]).inverse();
            let matrix = if let Some(inverse) = &inverse {
                let authored = Mat4::from_cols_array(&inverse[i]);
                if !authored.is_finite() || authored.determinant().abs() <= f32::MIN_POSITIVE {
                    return Err(AssetError::invalid("singular skin inverse bind"));
                }
                (change * ordinary_basis * authored.inverse()).inverse()
            } else {
                change.inverse()
            };
            (matrix, *translation)
        } else {
            (Mat4::IDENTITY, Vec3::ZERO)
        };
        if !matrix.is_finite() {
            return Err(AssetError::invalid("nonfinite converted skin bind"));
        }
        inverse_bind.push(matrix);
        let mut alternate = matrix;
        alternate.w_axis = translation.extend(1.0);
        alternate_bind.push(alternate);
    }
    Ok(Skin {
        names,
        inverse_bind,
        alternate_bind,
        bind_shape: Mat4::IDENTITY,
        pelvis_offset: 0.0,
    })
}
