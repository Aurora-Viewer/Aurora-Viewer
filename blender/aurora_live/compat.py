"""Blender 2.93+ API boundaries; keep capture and rigging behavior shared."""

# Principled inputs were renamed in Blender 4.0.
# developer.blender.org/docs/release_notes/4.0/python_api/
_OLD_INPUTS = {
    "Emission Color": "Emission", "Subsurface Weight": "Subsurface",
    "Coat Weight": "Clearcoat", "Sheen Weight": "Sheen",
    "Transmission Weight": "Transmission",
}


def principled_input(shader, name):
    socket = shader.inputs.get(name)
    return socket if socket is not None else shader.inputs.get(_OLD_INPUTS.get(name, name))


def organize_bones(rig, assignments):
    """Use collections in 4.0+, or named pose groups and layers in older rigs."""
    data = rig.data
    visible = {"Body", "Bento Hands"}
    if hasattr(data, "collections"):
        for bone, name in assignments:
            group = data.collections.get(name) or data.collections.new(name)
            group.assign(bone)
            group.is_visible = name in visible
    else:
        # The legacy API has 32 unnamed layers. Keep deterministic indices and
        # matching pose groups so all Bento/Fitted sets remain accessible.
        names = ["Body", "Bento Hands", "Bento Face"]
        names += sorted({name for _, name in assignments} - set(names) - {"Fitted Mesh"})
        names.append("Fitted Mesh")
        layers = {name: index for index, name in enumerate(names)}
        groups = {name: rig.pose.bone_groups.new(name=name) for name in names}
        for bone, name in assignments:
            bone.layers = tuple(index == layers[name] for index in range(32))
            rig.pose.bones[bone.name].bone_group = groups[name]
        data.layers = tuple(index in {layers[name] for name in visible} for index in range(32))
        rig["aurora_bone_layers"] = layers
