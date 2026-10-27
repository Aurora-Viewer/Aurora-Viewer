//! Collada geometry, sources, transforms, skin controllers and material
//! bindings. Follows LLDAELoader (Legacy Viewer, originally LGPL 2.1).

use super::*;
use aurora_llsd::xml::{self, Element};
use glam::Quat;
use std::collections::{HashMap, HashSet};

fn tag(element: &Element) -> &str {
    element.name.rsplit(':').next().unwrap_or(&element.name)
}
fn child<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.elements().find(|e| tag(e) == name)
}
fn descendant<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    if tag(element) == name {
        return Some(element);
    }
    element.elements().find_map(|e| descendant(e, name))
}
fn numbers(text: &str) -> Result<Vec<f32>, AssetError> {
    let mut values = Vec::new();
    for token in text.split_whitespace() {
        values.push(
            token
                .parse::<f32>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| AssetError::invalid("Collada number"))?,
        );
    }
    Ok(values)
}
fn integers(text: &str) -> Result<Vec<usize>, AssetError> {
    text.split_whitespace()
        .map(|n| n.parse().map_err(|_| AssetError::invalid("Collada index")))
        .collect()
}
fn count(element: &Element, name: &str, default: usize) -> Result<usize, AssetError> {
    element
        .attr(name)
        .map(|n| n.parse().map_err(|_| AssetError::invalid("Collada count")))
        .unwrap_or(Ok(default))
}
fn reference<'a>(ids: &HashMap<String, &'a Element>, uri: &str) -> Result<&'a Element, AssetError> {
    let id = uri
        .strip_prefix('#')
        .ok_or_else(|| AssetError::Unsupported("external Collada element reference".into()))?;
    ids.get(id)
        .copied()
        .ok_or_else(|| AssetError::invalid("unresolved Collada element"))
}
fn index_ids<'a>(root: &'a Element, ids: &mut HashMap<String, &'a Element>) -> Result<(), AssetError> {
    if let Some(id) = root.attr("id")
        && ids.insert(id.into(), root).is_some()
    {
        return Err(AssetError::invalid("too many or duplicate Collada IDs"));
    }
    for element in root.elements() {
        index_ids(element, ids)?;
    }
    Ok(())
}
struct FloatSource {
    values: Vec<f32>,
    stride: usize,
    offset: usize,
    count: usize,
}
impl FloatSource {
    fn parse(element: &Element) -> Result<Self, AssetError> {
        let array = child(element, "float_array").ok_or_else(|| AssetError::invalid("Collada source has no floats"))?;
        let values = numbers(&array.text())?;
        if count(array, "count", values.len())? != values.len() {
            return Err(AssetError::invalid("Collada float count"));
        }
        let accessor = descendant(element, "accessor").ok_or_else(|| AssetError::invalid("Collada source accessor"))?;
        let stride = count(accessor, "stride", 1)?;
        let offset = count(accessor, "offset", 0)?;
        let count = count(accessor, "count", 0)?;
        if stride == 0
            || count
                .checked_mul(stride)
                .and_then(|n| n.checked_add(offset))
                .is_none_or(|n| n > values.len())
        {
            return Err(AssetError::invalid("Collada accessor bounds"));
        }
        Ok(Self {
            values,
            stride,
            offset,
            count,
        })
    }
    fn row<const N: usize>(&self, index: usize) -> Result<[f32; N], AssetError> {
        if index >= self.count || N > self.stride {
            return Err(AssetError::invalid("Collada source index or width"));
        }
        Ok(std::array::from_fn(|i| self.values[self.offset + index * self.stride + i]))
    }
}
fn source(id: &str, ids: &HashMap<String, &Element>, cache: &mut HashMap<String, FloatSource>) -> Result<(), AssetError> {
    if !cache.contains_key(id) {
        cache.insert(id.into(), FloatSource::parse(reference(ids, id)?)?);
    }
    Ok(())
}

pub fn parse(data: &[u8], skeleton: &crate::Skeleton) -> Result<Scene, AssetError> {
    parse_with_meters(data, skeleton, false)
}
/// FSLocalMeshScaleAlwaysMeters deliberately overrides the declared unit.
pub fn parse_with_meters(data: &[u8], skeleton: &crate::Skeleton, assume_meters: bool) -> Result<Scene, AssetError> {
    parse_with_options(data, skeleton, assume_meters, DEFAULT_MODEL_LIMIT)
}

pub fn parse_with_options(data: &[u8], skeleton: &crate::Skeleton, assume_meters: bool, model_limit: u32) -> Result<Scene, AssetError> {
    let root = xml::parse(data)?;
    if tag(&root) != "COLLADA" {
        return Err(AssetError::invalid("expected Collada document"));
    }
    let mut ids = HashMap::new();
    index_ids(&root, &mut ids)?;
    let asset = child(&root, "asset");
    let meter = asset
        .and_then(|a| child(a, "unit"))
        .and_then(|u| u.attr("meter"))
        .unwrap_or("1")
        .parse::<f32>()
        .map_err(|_| AssetError::invalid("Collada units"))?;
    let meter = if assume_meters { 1.0 } else { meter };
    if !meter.is_finite() || meter <= 0.0 {
        return Err(AssetError::invalid("Collada unit scale"));
    }
    let up = asset
        .and_then(|a| child(a, "up_axis"))
        .map(|a| a.text())
        .unwrap_or_else(|| "Y_UP".into());
    let rotation = match up.trim() {
        "Z_UP" => Quat::IDENTITY,
        "Y_UP" => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        "X_UP" => Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
        _ => return Err(AssetError::invalid("Collada up axis")),
    };
    let basis = Mat4::from_scale_rotation_translation(Vec3::splat(meter), rotation, Vec3::ZERO);
    let mut scene = Scene::default();
    let mut materials = HashMap::new();
    if let Some(library) = child(&root, "library_images") {
        for image in library.elements().filter(|e| tag(e) == "image") {
            if let (Some(id), Some(init)) = (image.attr("id"), child(image, "init_from")) {
                let index = scene.image_uris.len();
                scene.image_uris.insert(index, init.text().trim().into());
                materials.insert(format!("image:{id}"), index);
            }
        }
    }
    if let Some(library) = child(&root, "library_materials") {
        for mat in library.elements().filter(|e| tag(e) == "material") {
            let effect = child(mat, "instance_effect")
                .and_then(|e| e.attr("url"))
                .map(|u| reference(&ids, u))
                .transpose()?;
            let mut value = PbrMaterial {
                metallic_factor: 0.0,
                ..Default::default()
            };
            let mut images = [None; 4];
            if let Some(effect) = effect
                && let Some(diffuse) = descendant(effect, "diffuse")
            {
                if let Some(color) = child(diffuse, "color") {
                    let c = numbers(&color.text())?;
                    if c.len() == 4 {
                        value.base_color_factor.copy_from_slice(&c);
                    }
                }
                if let Some(texture) = child(diffuse, "texture").and_then(|e| e.attr("texture")) {
                    let mut id = texture.to_owned();
                    for _ in 0..4 {
                        let param = effect
                            .elements()
                            .flat_map(|e| e.elements())
                            .find(|e| tag(e) == "newparam" && e.attr("sid") == Some(id.as_str()));
                        if let Some(param) = param {
                            let next = descendant(param, "source").or_else(|| descendant(param, "init_from"));
                            if let Some(next) = next {
                                id = next.text().trim().into();
                                continue;
                            }
                        }
                        break;
                    }
                    images[0] = materials.get(&format!("image:{id}")).copied();
                }
            }
            let index = scene.materials.len();
            let id = mat.attr("id").ok_or_else(|| AssetError::invalid("Collada material ID"))?;
            materials.insert(id.into(), index);
            scene.materials.push(Material {
                name: mat.attr("name").unwrap_or(id).into(),
                value,
                images,
            });
        }
    }
    let default = scene.materials.len();
    scene.materials.push(Material {
        name: "Default".into(),
        value: PbrMaterial::default(),
        images: [None; 4],
    });
    let visual = descendant(&root, "instance_visual_scene")
        .and_then(|e| e.attr("url"))
        .map(|u| reference(&ids, u))
        .transpose()?
        .or_else(|| descendant(&root, "visual_scene"))
        .ok_or_else(|| AssetError::invalid("Collada visual scene"))?;
    let mut geometries = HashMap::<String, Vec<usize>>::new();
    let mut joint_nodes = HashMap::new();
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        if tag(node) == "node" {
            for name in [node.attr("sid"), node.attr("id"), node.attr("name")].into_iter().flatten() {
                let name = skeleton.find(name).map_or(name, |i| skeleton.joints[i].name.as_str());
                joint_nodes.insert(name.to_owned(), node);
            }
        }
        pending.extend(node.elements().collect::<Vec<_>>().into_iter().rev());
    }
    let mut symbols = HashMap::<usize, Vec<String>>::new();
    let mut floats = HashMap::new();
    let mesh_count = ids
        .values()
        .filter(|node| tag(node) == "geometry" && child(node, "mesh").is_some())
        .count()
        .max(1);
    let face_limit = (model_limit as usize / mesh_count + 1).saturating_mul(8);
    fn visit(
        node: &Element,
        parent: Mat4,
        basis: Mat4,
        ids: &HashMap<String, &Element>,
        floats: &mut HashMap<String, FloatSource>,
        geometries: &mut HashMap<String, Vec<usize>>,
        symbols: &mut HashMap<usize, Vec<String>>,
        materials: &HashMap<String, usize>,
        default: usize,
        skeleton: &crate::Skeleton,
        joint_nodes: &HashMap<String, &Element>,
        scene: &mut Scene,
        visiting: &mut HashSet<String>,
        depth: usize,
        face_limit: usize,
    ) -> Result<(), AssetError> {
        if depth > 256 {
            return Err(AssetError::TooLarge("Collada hierarchy"));
        }
        let mut local = Mat4::IDENTITY;
        for transform in node.elements() {
            let numbers = match tag(transform) {
                "matrix" | "translate" | "rotate" | "scale" => numbers(&transform.text())?,
                _ => continue,
            };
            let matrix = match tag(transform) {
                "matrix" if numbers.len() == 16 => {
                    let values: [f32; 16] = numbers.try_into().map_err(|_| AssetError::invalid("matrix"))?;
                    Mat4::from_cols_array(&values).transpose()
                }
                "translate" if numbers.len() == 3 => Mat4::from_translation(Vec3::new(numbers[0], numbers[1], numbers[2])),
                "scale" if numbers.len() == 3 => Mat4::from_scale(Vec3::new(numbers[0], numbers[1], numbers[2])),
                "rotate" if numbers.len() == 4 => {
                    let axis = Vec3::new(numbers[0], numbers[1], numbers[2]).normalize_or_zero();
                    if axis == Vec3::ZERO {
                        return Err(AssetError::invalid("rotation axis"));
                    }
                    Mat4::from_quat(Quat::from_axis_angle(axis, numbers[3].to_radians()))
                }
                _ => return Err(AssetError::invalid("Collada transform size")),
            };
            local *= matrix;
        }
        let global = parent * local;
        if !global.is_finite() || global.determinant() <= 0.0 {
            return Err(AssetError::invalid("mirrored or singular Collada transform"));
        }
        for instance in node
            .elements()
            .filter(|e| matches!(tag(e), "instance_geometry" | "instance_controller"))
        {
            let uri = instance.attr("url").ok_or_else(|| AssetError::invalid("geometry instance URI"))?;
            let referenced = reference(ids, uri)?;
            let controller = if tag(referenced) == "controller" {
                Some(child(referenced, "skin").ok_or_else(|| AssetError::Unsupported("Collada morph controller".into()))?)
            } else {
                None
            };
            let geometry = if let Some(controller) = controller {
                reference(ids, controller.attr("source").ok_or_else(|| AssetError::invalid("skin source"))?)?
            } else {
                referenced
            };
            let key = if controller.is_some() {
                uri.to_owned()
            } else {
                geometry.attr("id").unwrap_or(uri).into()
            };
            if !geometries.contains_key(&key) {
                // processDomModel keeps a rigged DAE's authored bind frame.
                // up_axis/unit conversion belongs to its object instance;
                // converting bone-local offsets as world vectors rotates
                // the SL skeleton a second time when weights are enabled.
                let geometry_basis = if controller.is_some() { Mat4::IDENTITY } else { basis };
                let (faces, face_symbols, position_indices) = read_geometry(geometry, geometry_basis, ids, floats, default)?;
                let skin = if let Some(controller) = controller {
                    Some(read_skin(
                        controller,
                        geometry_basis,
                        ids,
                        floats,
                        skeleton,
                        joint_nodes,
                        &mut scene.warnings,
                    )?)
                } else {
                    None
                };
                let mut faces = faces;
                if let Some((_, weights)) = &skin {
                    // LLDAELoader writes mSkinWeights[position] in source
                    // order: the last source vertex at a position wins.
                    let mut last = HashMap::new();
                    for (face, original) in faces.iter().zip(&position_indices) {
                        for (vertex, original) in face.vertices.iter().zip(original) {
                            let entry = last.entry(vertex.position.map(|v| (v + 0.0).to_bits())).or_insert(*original);
                            *entry = (*entry).max(*original);
                        }
                    }
                    for (face, original) in faces.iter_mut().zip(position_indices) {
                        for (vertex, original) in face.vertices.iter_mut().zip(original) {
                            let original = last.get(&vertex.position.map(|v| (v + 0.0).to_bits())).copied().unwrap_or(original);
                            let influences = weights
                                .get(original)
                                .ok_or_else(|| AssetError::invalid("skin weights do not match geometry"))?;
                            vertex.joints = influences.0;
                            vertex.weights = influences.1;
                        }
                    }
                }
                let mut split_faces = Vec::new();
                let mut split_symbols = Vec::new();
                for (face, symbol) in faces.into_iter().zip(face_symbols) {
                    let parts = split_face(face.vertices, face.indices, face.material)?;
                    split_symbols.extend(std::iter::repeat_n(symbol, parts.len()));
                    split_faces.extend(parts);
                }
                let mut ordered: Vec<_> = split_faces.into_iter().zip(split_symbols).collect();
                ordered.sort_by(|a, b| a.1.cmp(&b.1));
                if ordered.len() > face_limit {
                    scene.warnings.push(format!(
                        "Generated model limit reached for {}",
                        geometry.attr("name").unwrap_or("Mesh")
                    ));
                    ordered.truncate(face_limit);
                }
                let (mut faces, face_symbols): (Vec<_>, Vec<_>) = ordered.into_iter().unzip();
                let mut models = Vec::new();
                let (center, size) = normalization_with_flat_scale(&faces, geometry_basis.x_axis.truncate().length())?;
                if skin.is_some() {
                    super::skin::share_weights(&mut faces, center, size);
                    super::skin::complete_weights(&mut faces, size);
                }
                for (index, group) in faces.chunks(8).enumerate() {
                    let mut model = Model {
                        name: geometry.attr("name").or_else(|| geometry.attr("id")).unwrap_or("Mesh").into(),
                        submodel: index as u32,
                        faces: group.to_vec(),
                        skin: skin.as_ref().map(|s| s.0.clone()),
                        center: Vec3::ZERO,
                        size: Vec3::ONE,
                    };
                    model.normalize_to(center, size)?;
                    let id = scene.models.len();
                    scene.normalized_bounds.insert(id, model.normalized_bounds());
                    symbols.insert(id, face_symbols[index * 8..(index * 8 + group.len())].to_vec());
                    scene.material_bindings.insert(id, symbols[&id].clone());
                    models.push(id);
                    scene.models.push(model);
                }
                geometries.insert(key.clone(), models);
            }
            let mut bindings = HashMap::new();
            if let Some(common) = descendant(instance, "technique_common") {
                for bind in common.elements().filter(|e| tag(e) == "instance_material") {
                    if let (Some(symbol), Some(target)) = (bind.attr("symbol"), bind.attr("target"))
                        && let Some(material) = materials.get(target.trim_start_matches('#'))
                    {
                        bindings.insert(symbol.to_owned(), *material);
                    }
                }
            }
            for model in &geometries[&key] {
                let source = &scene.models[*model];
                let normalized = Mat4::from_scale_rotation_translation(source.size, Quat::IDENTITY, source.center);
                scene.instances.push(Instance {
                    model: *model,
                    transform: basis
                        * global
                        * if controller.is_some() {
                            normalized
                        } else {
                            basis.inverse() * normalized
                        },
                    name: node.attr("name").or_else(|| node.attr("id")).unwrap_or(&source.name).into(),
                    materials: symbols[model]
                        .iter()
                        .map(|symbol| {
                            bindings
                                .get(symbol)
                                .copied()
                                .or_else(|| materials.get(symbol).copied())
                                .unwrap_or(default)
                        })
                        .collect(),
                });
            }
        }
        for nested in node.elements() {
            match tag(nested) {
                "node" => visit(
                    nested,
                    global,
                    basis,
                    ids,
                    floats,
                    geometries,
                    symbols,
                    materials,
                    default,
                    skeleton,
                    joint_nodes,
                    scene,
                    visiting,
                    depth + 1,
                    face_limit,
                )?,
                "instance_node" => {
                    let uri = nested.attr("url").ok_or_else(|| AssetError::invalid("node instance URI"))?;
                    if !visiting.insert(uri.into()) {
                        return Err(AssetError::invalid("cyclic Collada node instance"));
                    }
                    visit(
                        reference(ids, uri)?,
                        global,
                        basis,
                        ids,
                        floats,
                        geometries,
                        symbols,
                        materials,
                        default,
                        skeleton,
                        joint_nodes,
                        scene,
                        visiting,
                        depth + 1,
                        face_limit,
                    )?;
                    visiting.remove(uri);
                }
                _ => {}
            }
        }
        Ok(())
    }
    for node in visual.elements().filter(|e| tag(e) == "node") {
        visit(
            node,
            Mat4::IDENTITY,
            basis,
            &ids,
            &mut floats,
            &mut geometries,
            &mut symbols,
            &materials,
            default,
            skeleton,
            &joint_nodes,
            &mut scene,
            &mut HashSet::new(),
            0,
            face_limit,
        )?;
    }
    // LLDAELoader::OpenFile / processDomModel also adds skinned geometry
    // directly from library_controllers. Some exporters write only the joint
    // hierarchy in visual_scene, with no instance_controller at all. Route
    // these implicit instances through the same decoder and normalization;
    // controllers already instanced by the scene must not be added twice.
    if let Some(controllers) = child(&root, "library_controllers") {
        for controller in controllers
            .elements()
            .filter(|e| tag(e) == "controller" && child(e, "skin").is_some())
        {
            let id = controller.attr("id").ok_or_else(|| AssetError::invalid("Collada controller ID"))?;
            let uri = format!("#{id}");
            if geometries.contains_key(&uri) {
                continue;
            }
            let node = Element {
                name: "node".into(),
                children: vec![xml::Node::Element(Element {
                    name: "instance_controller".into(),
                    attrs: vec![("url".into(), uri)],
                    ..Default::default()
                })],
                ..Default::default()
            };
            visit(
                &node,
                Mat4::IDENTITY,
                basis,
                &ids,
                &mut floats,
                &mut geometries,
                &mut symbols,
                &materials,
                default,
                skeleton,
                &joint_nodes,
                &mut scene,
                &mut HashSet::new(),
                0,
                face_limit,
            )?;
        }
    }
    if scene.models.is_empty() {
        return Err(AssetError::invalid("Collada scene has no mesh geometry"));
    }
    Ok(scene)
}

type Geometry = (Vec<Face>, Vec<String>, Vec<Vec<usize>>);
fn read_geometry(
    geometry: &Element,
    basis: Mat4,
    ids: &HashMap<String, &Element>,
    cache: &mut HashMap<String, FloatSource>,
    default: usize,
) -> Result<Geometry, AssetError> {
    let mesh = child(geometry, "mesh").ok_or_else(|| AssetError::invalid("Collada geometry has no mesh"))?;
    let mut faces = Vec::new();
    let mut symbols = Vec::new();
    let mut originals = Vec::new();
    for primitive in mesh.elements().filter(|e| matches!(tag(e), "triangles" | "polylist" | "polygons")) {
        let mut inputs = Vec::new();
        let mut stride = 0;
        for input in primitive.elements().filter(|e| tag(e) == "input") {
            let semantic = input.attr("semantic").unwrap_or("");
            let id = input
                .attr("source")
                .ok_or_else(|| AssetError::invalid("primitive input source"))?
                .to_owned();
            let offset = count(input, "offset", 0)?;
            if offset > 32 {
                return Err(AssetError::invalid("primitive input offset"));
            }
            stride = stride.max(offset + 1);
            if semantic == "VERTEX" {
                let vertices = reference(ids, &id)?;
                for input in vertices.elements().filter(|e| tag(e) == "input") {
                    let semantic = input.attr("semantic").unwrap_or("");
                    if matches!(semantic, "POSITION" | "NORMAL") || (semantic == "TEXCOORD" && count(input, "set", 0)? == 0) {
                        let id = input.attr("source").ok_or_else(|| AssetError::invalid("vertex input source"))?;
                        source(id, ids, cache)?;
                        inputs.push((semantic.to_owned(), id.to_owned(), offset));
                    }
                }
                continue;
            }
            if matches!(semantic, "VERTEX" | "POSITION" | "NORMAL") || (semantic == "TEXCOORD" && count(input, "set", 0)? == 0) {
                source(&id, ids, cache)?;
                inputs.push((semantic.to_owned(), id, offset));
            }
        }
        if stride == 0 || !inputs.iter().any(|(semantic, _, _)| semantic == "VERTEX" || semantic == "POSITION") {
            return Err(AssetError::invalid("mesh primitive has no position input"));
        }
        let mut polygons = Vec::new();
        match tag(primitive) {
            "triangles" => {
                let p = child(primitive, "p").ok_or_else(|| AssetError::invalid("triangle indices"))?;
                let indices = integers(&p.text())?;
                if indices.len() % (stride * 3) != 0 {
                    return Err(AssetError::invalid("incomplete triangle indices"));
                }
                polygons.extend(indices.chunks(stride * 3).map(|p| p.to_vec()));
            }
            "polylist" => {
                let indices = integers(&child(primitive, "p").ok_or_else(|| AssetError::invalid("polylist indices"))?.text())?;
                let counts = integers(
                    &child(primitive, "vcount")
                        .ok_or_else(|| AssetError::invalid("polylist counts"))?
                        .text(),
                )?;
                let mut start = 0usize;
                for count in counts {
                    if !(3..=4096).contains(&count) {
                        return Err(AssetError::invalid("polygon size"));
                    }
                    let end = count
                        .checked_mul(stride)
                        .and_then(|n| start.checked_add(n))
                        .filter(|n| *n <= indices.len())
                        .ok_or(AssetError::Truncated("polylist"))?;
                    polygons.push(indices[start..end].to_vec());
                    start = end;
                }
                if start != indices.len() {
                    return Err(AssetError::invalid("extra polylist indices"));
                }
            }
            _ => {
                for p in primitive.elements() {
                    if tag(p) == "ph" {
                        return Err(AssetError::Unsupported("Collada polygon holes".into()));
                    }
                    if tag(p) == "p" {
                        let indices = integers(&p.text())?;
                        if indices.len() % stride != 0 {
                            return Err(AssetError::invalid("polygon indices"));
                        }
                        polygons.push(indices);
                    }
                }
            }
        }
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut map = HashMap::<Vec<usize>, u32>::new();
        let mut position_indices = Vec::new();
        for polygon in polygons {
            let mut corners = Vec::new();
            for tuple in polygon.chunks_exact(stride) {
                let index = if let Some(index) = map.get(tuple) {
                    *index
                } else {
                    let mut vertex = Vertex {
                        position: [0.0; 3],
                        normal: [0.0; 3],
                        uv: [0.0; 2],
                        joints: [0; 4],
                        weights: [0.0; 4],
                    };
                    let mut original = 0;
                    for (semantic, id, offset) in &inputs {
                        let index = tuple[*offset];
                        let source = &cache[id];
                        match semantic.as_str() {
                            "VERTEX" | "POSITION" => {
                                vertex.position = basis.transform_point3(Vec3::from_array(source.row::<3>(index)?)).to_array();
                                original = index;
                            }
                            "NORMAL" => {
                                vertex.normal = basis
                                    .transform_vector3(Vec3::from_array(source.row::<3>(index)?))
                                    .normalize_or_zero()
                                    .to_array()
                            }
                            "TEXCOORD" => vertex.uv = source.row::<2>(index)?,
                            _ => {}
                        }
                    }
                    let index = u32::try_from(vertices.len()).map_err(|_| AssetError::TooLarge("mesh vertices"))?;
                    vertices.push(vertex);
                    position_indices.push(original);
                    map.insert(tuple.to_vec(), index);
                    index
                };
                corners.push(index);
            }
            if corners.len() < 3 || corners.len() > 4096 {
                return Err(AssetError::invalid("polygon vertex count"));
            }
            for triangle in 1..corners.len() - 1 {
                indices.extend_from_slice(&[corners[0], corners[triangle], corners[triangle + 1]]);
            }
        }
        let mut face = Face {
            vertices,
            indices,
            material: default,
        };
        if face.vertices.iter().all(|v| v.normal == [0.0; 3]) {
            process::missing_normals(&mut face)?;
        }
        face.validate_geometry()?;
        faces.push(face);
        symbols.push(primitive.attr("material").unwrap_or("").into());
        originals.push(position_indices);
    }
    if faces.is_empty() {
        return Err(AssetError::invalid("Collada mesh has no triangle faces"));
    }
    Ok((faces, symbols, originals))
}

type Influences = Vec<([u16; 4], [f32; 4])>;
fn read_skin(
    skin: &Element,
    basis: Mat4,
    ids: &HashMap<String, &Element>,
    cache: &mut HashMap<String, FloatSource>,
    skeleton: &crate::Skeleton,
    joint_nodes: &HashMap<String, &Element>,
    warnings: &mut Vec<String>,
) -> Result<(Skin, Influences), AssetError> {
    let joints = child(skin, "joints").ok_or_else(|| AssetError::invalid("skin joints"))?;
    let input = |semantic| {
        joints
            .elements()
            .find(|e| tag(e) == "input" && e.attr("semantic") == Some(semantic))
            .and_then(|e| e.attr("source"))
            .ok_or_else(|| AssetError::invalid("skin input"))
    };
    let names_source = reference(ids, input("JOINT")?)?;
    let names = child(names_source, "Name_array")
        .or_else(|| child(names_source, "IDREF_array"))
        .ok_or_else(|| AssetError::invalid("skin joint names"))?
        .text();
    let mut output_names = Vec::new();
    for name in names.split_whitespace() {
        if output_names.len() >= u16::MAX as usize {
            return Err(AssetError::TooLarge("skin joints"));
        }
        output_names.push(skeleton.find(name).map(|i| skeleton.joints[i].name.clone()).unwrap_or_else(|| {
            warnings.push(format!("Unknown joint: {name}"));
            name.into()
        }));
    }
    let bind_source = input("INV_BIND_MATRIX")?;
    source(bind_source, ids, cache)?;
    let inverse = &cache[bind_source];
    if inverse.count != output_names.len() {
        return Err(AssetError::invalid("skin bind matrix count"));
    }
    let inverse_bind = (0..inverse.count)
        .map(|i| {
            inverse
                .row::<16>(i)
                .map(|v| basis * Mat4::from_cols_array(&v).transpose() * basis.inverse())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bind = child(skin, "bind_shape_matrix")
        .map(|e| numbers(&e.text()))
        .transpose()?
        .unwrap_or_else(|| Mat4::IDENTITY.to_cols_array().to_vec());
    let mut alternate_bind = Vec::with_capacity(output_names.len());
    for (name, inverse) in output_names.iter().zip(&inverse_bind) {
        let node = joint_nodes.get(name).copied();
        let offset = if let Some(node) = node {
            if let Some(translation) = node
                .elements()
                .find(|e| tag(e) == "translate" && e.attr("sid").is_some_and(|sid| sid == "translate" || sid == "location"))
                .or_else(|| child(node, "translate"))
            {
                let values: [f32; 3] = numbers(&translation.text())?
                    .try_into()
                    .map_err(|_| AssetError::invalid("joint translation"))?;
                basis.transform_vector3(Vec3::from_array(values))
            } else if let Some(matrix) = child(node, "matrix").or_else(|| node.elements().find(|e| e.attr("sid") == Some("transform"))) {
                let values: [f32; 16] = numbers(&matrix.text())?
                    .try_into()
                    .map_err(|_| AssetError::invalid("joint matrix"))?;
                basis.transform_vector3(Mat4::from_cols_array(&values).transpose().w_axis.truncate())
            } else {
                Vec3::ZERO
            }
        } else {
            skeleton.find(name).map(|i| skeleton.joints[i].pos).unwrap_or(Vec3::ZERO)
        };
        let mut alternate = *inverse;
        alternate.w_axis = offset.extend(1.0);
        alternate_bind.push(alternate);
    }
    let bind: [f32; 16] = bind.try_into().map_err(|_| AssetError::invalid("skin bind shape matrix"))?;
    let vertex_weights = child(skin, "vertex_weights").ok_or_else(|| AssetError::invalid("skin vertex weights"))?;
    let mut stride = 0;
    let mut joint_offset = None;
    let mut weight_offset = None;
    let mut weight_source = None;
    for input in vertex_weights.elements().filter(|e| tag(e) == "input") {
        let offset = count(input, "offset", 0)?;
        if offset > 16 {
            return Err(AssetError::invalid("skin weight offset"));
        }
        stride = stride.max(offset + 1);
        match input.attr("semantic") {
            Some("JOINT") => joint_offset = Some(offset),
            Some("WEIGHT") => {
                weight_offset = Some(offset);
                weight_source = input.attr("source");
            }
            _ => {}
        }
    }
    let joint_offset = joint_offset.ok_or_else(|| AssetError::invalid("skin joint weight input"))?;
    let weight_offset = weight_offset.ok_or_else(|| AssetError::invalid("skin weight input"))?;
    let weight_source = weight_source.ok_or_else(|| AssetError::invalid("skin weight source"))?;
    source(weight_source, ids, cache)?;
    let counts = integers(
        &child(vertex_weights, "vcount")
            .ok_or_else(|| AssetError::invalid("skin counts"))?
            .text(),
    )?;
    let indices = integers(
        &child(vertex_weights, "v")
            .ok_or_else(|| AssetError::invalid("skin influence indices"))?
            .text(),
    )?;
    let mut influences = Vec::new();
    let mut start = 0usize;
    for count in counts {
        let end = count
            .checked_mul(stride)
            .and_then(|n| start.checked_add(n))
            .filter(|n| *n <= indices.len())
            .ok_or(AssetError::Truncated("skin influences"))?;
        let mut weighted = Vec::new();
        for tuple in indices[start..end].chunks_exact(stride) {
            let joint = tuple[joint_offset];
            if joint >= output_names.len() {
                return Err(AssetError::invalid("skin joint weight index"));
            }
            let weight = cache[weight_source].row::<1>(tuple[weight_offset])?[0];
            if weight < 0.0 {
                return Err(AssetError::invalid("negative skin weight"));
            }
            if weight > 0.0 {
                weighted.push((joint as u16, weight));
            }
        }
        weighted.sort_by(|a, b| b.1.total_cmp(&a.1));
        weighted.truncate(4);
        let total: f32 = weighted.iter().map(|v| v.1).sum();
        let mut ids = [0; 4];
        let mut weights = [0.0; 4];
        if total > 0.0 {
            for (slot, (id, weight)) in weighted.into_iter().enumerate() {
                ids[slot] = id;
                weights[slot] = weight / total;
            }
        }
        influences.push((ids, weights));
        start = end;
    }
    if start != indices.len() {
        return Err(AssetError::invalid("extra skin weights"));
    }
    Ok((
        Skin {
            names: output_names,
            alternate_bind,
            inverse_bind,
            bind_shape: basis * Mat4::from_cols_array(&bind).transpose() * basis.inverse(),
            pelvis_offset: 0.0,
        },
        influences,
    ))
}
