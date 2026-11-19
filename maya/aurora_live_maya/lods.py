"""Prepare the four-level Import / LOD bundle Aurora asks for when its Import
button is clicked: the Blender plug-in's rules (blender/aurora_live/lods.py).

Each lower level is an existing mesh (picked, or named after High with the
level's suffix: Body_medium, Body_low, Body_lowest) or a reduced copy of High
at rest made with polyReduce. A reduced copy of a skinned mesh takes each
vertex's weights from the nearest High vertex, so no skinCluster is created.
The copies are deleted, without an undo step; the synchronized meshes are not
changed.
"""

import uuid

import maya.api.OpenMaya as om
from maya import cmds

from . import capture, client, convert


def existing(high, spec):
    """The level's existing mesh transform, or None."""
    name = spec["object"].strip()
    if name:
        found = cmds.ls(name, long=True, type="transform") or []
        if not found or capture.shape_of(found[0]) is None:
            raise ValueError("LOD mesh not found: " + name)
        if found[0] == high:
            raise ValueError("Choose a different LOD mesh than High")
        return found[0]
    target = convert.lod_name(high.rsplit("|", 1)[-1], spec["suffix"]).casefold()
    found = [node for node in cmds.ls(type="transform", long=True) or []
             if node != high and node.rsplit("|", 1)[-1].rsplit(":", 1)[-1].casefold() == target
             and capture.shape_of(node) is not None]
    if len(found) > 1:
        raise ValueError("Ambiguous LOD mesh name: " + target)
    return found[0] if found else None


def _reduced(source, target):
    """A disposable copy of High at rest, reduced to about `target` triangles.
    Returns its transform; the caller deletes it."""
    copy = cmds.duplicate(source.transform, returnRootsOnly=True)[0]
    copy = cmds.ls(copy, long=True)[0]
    try:
        for shape in cmds.listRelatives(copy, shapes=True, fullPath=True) or []:
            if cmds.getAttr(shape + ".intermediateObject"):
                cmds.delete(shape)
        shape = capture.shape_of(copy)
        if shape is None:
            raise ValueError("Cannot copy the High mesh")
        fn = om.MFnMesh(capture.dag(shape))
        if source.skin:
            # The duplicate holds the posed shape; the skin's input is the rest.
            rest = source.fn.getPoints(om.MSpace.kObject)
            if len(rest) != fn.numVertices:
                raise ValueError("The skin's input mesh and the shape differ; delete non-deformer history")
            fn.setPoints(rest, om.MSpace.kObject)
        triangles = sum(fn.getTriangles()[0])
        if triangles > target:
            cmds.polyReduce(shape, version=1, termination=2, triangleCount=int(target), keepQuadsWeight=0,
                            keepBorder=True, keepMapBorder=1, keepHardEdge=True, replaceOriginal=True,
                            constructionHistory=False)
        return copy
    except Exception:
        cmds.delete(copy)
        raise


def _plain(mesh):
    """A wire mesh with vertex lists: the LOD bundle is JSON without packed
    geometry, as the viewer's import decoder counts its corners."""
    return client.wire_snapshot({"meshes": [mesh]}, ["source-ids", "static-meshes", "object-links"])["meshes"][0]


def prepare(request, transform, settings, supported, rest_axes, base, axes_mode):
    """The bundle for one Import request: four levels, Lowest to High."""
    uuid.UUID(request["id"])
    source_id = str(uuid.UUID(request["source"]))
    images = convert.Images()
    warnings = []
    source = capture.Source(transform, supported, base)
    geometry = capture.Geometry(source.fn, source.shape_fn, source.shape_path, source.matrix)
    if source.skin:
        names, joints, weights = capture.skin_weights(source.per_vertex, geometry)
    else:
        names, joints, weights = [], None, None
    inverse_bind = convert.inverse_binds(names, source.bind_pre, source.frame, axes_mode, rest_axes) if names else []
    packed = geometry.pack(joints, weights)
    high_faces, high_groups = capture.faces(packed, geometry, source.shape, images, warnings)
    high = _plain({"name": source.name, "source_id": source_id, "joints": names, "inverse_bind": inverse_bind,
                   "faces": high_faces})
    high_triangles = len(geometry.triangles)
    levels = {"high": {"mesh": high, "slots": list(range(len(high_faces))), "origin": source.name[:64]}}
    undo = cmds.undoInfo(query=True, stateWithoutFlush=True)
    for level, _ratio in convert.LOD_LEVELS:
        spec = settings.get(level) or convert.lod_default(level)
        found = existing(transform, spec) if spec["mode"] != "auto" else None
        if found is None and spec["mode"] == "existing":
            raise ValueError("Missing LOD mesh: " + source.name + " / " + level)
        if found is not None:
            groups = []
            mesh, _count = capture.mesh(found, supported, rest_axes, images, warnings, axes_mode, base, groups)
            if bool(mesh["joints"]) != bool(names):
                raise ValueError("LOD meshes must use the same avatar rig as High")
            origin = found.rsplit("|", 1)[-1]
        else:
            copy = None
            try:
                # The copy is scratch work: keep it out of the undo queue.
                cmds.undoInfo(stateWithoutFlush=False)
                copy = _reduced(source, convert.lod_target(high_triangles, spec))
                shape = capture.shape_of(copy)
                path = capture.dag(shape)
                reduced = capture.Geometry(om.MFnMesh(path), om.MFnMesh(path), path, source.matrix)
                level_joints = level_weights = None
                if source.skin:
                    level_joints, level_weights = convert.transfer(geometry.positions, joints, weights,
                                                                   geometry.referenced, reduced.positions)
                level_faces, groups = capture.faces(reduced.pack(level_joints, level_weights), reduced, shape, images,
                                                    warnings)
                mesh = {"name": source.name, "joints": names, "inverse_bind": inverse_bind, "faces": level_faces}
            finally:
                if copy is not None and cmds.objExists(copy):
                    cmds.delete(copy)
                cmds.undoInfo(stateWithoutFlush=undo)
            origin = "Automatic"
        mesh["name"], mesh["source_id"] = source.name, source_id
        levels[level] = {"mesh": _plain(mesh), "slots": convert.slot_mapping(groups, high_groups), "origin": origin[:64]}
    ordered = [levels[name] for name in ("lowest", "low", "medium", "high")]
    if sum(len(face["vertices"]) for level in ordered for face in level["mesh"]["faces"]) > convert.MAX_CORNERS:
        raise ValueError("Prepared LODs exceed 600,000 mesh corners")
    convert.merge_palettes([level["mesh"] for level in ordered])
    return {"id": request["id"], "source": source_id, "levels": ordered, "images": images.images}
