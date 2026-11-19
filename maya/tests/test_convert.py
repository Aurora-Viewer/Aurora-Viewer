"""Tests of the Maya plug-in's pure rules, without Maya: python maya/tests/test_convert.py"""
import base64
import importlib.util
import json
import math
from pathlib import Path
import socket
import struct
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


convert = load("aurora_maya_convert", ROOT / "maya" / "aurora_live_maya" / "convert.py")
import sys
sys.path.insert(0, str(ROOT / "maya"))
from aurora_live_maya import shape_rules  # noqa: E402  (relative imports need the package)
from aurora_live_maya import convert as package_convert  # noqa: E402
client = load("aurora_maya_client", ROOT / "blender" / "aurora_live" / "client.py")


def close(a, b, places=6):
    return all(abs(x - y) < 10 ** -places for x, y in zip(a, b))


class Frames(unittest.TestCase):
    def test_y_up_centimetres_become_z_up_metres_like_a_y_up_collada_upload(self):
        self.assertTrue(close(convert.point(convert.TO_SL, (0, 100, 0)), (0, 0, 1)))
        self.assertTrue(close(convert.point(convert.TO_SL, (0, 0, 100)), (0, -1, 0)))
        self.assertTrue(close(convert.point(convert.TO_SL, (100, 0, 0)), (1, 0, 0)))
        self.assertAlmostEqual(convert.determinant3(convert.Y_UP), 1.0)

    def test_a_z_up_scene_is_not_turned_and_takes_its_unit(self):
        frame = convert.base_frame("z", 1.0)
        self.assertTrue(close(convert.point(frame, (0, 0, 1.067)), (0, 0, 1.067)))
        self.assertTrue(close(convert.point(convert.base_frame("y", 1.0), (0, 1.067, 0)), (0, 0, 1.067)))

    def test_rig_unit_follows_the_pelvis_height(self):
        # A Z-up kit modelled at one unit per metre, Maya set to centimetres
        # (human report, 2026-11-18): pelvis 1.067 units above the origin.
        metre_kit = {"mPelvis": (0, 0, 1.067), "mFootLeft": (0.1, 0, 0.08), "mFootRight": (-0.1, 0, 0.08)}
        self.assertEqual(convert.rig_unit(metre_kit, "z"), 1.0)
        # The same rig at real size in centimetres, Y up.
        cm_kit = {"mPelvis": (0, 106.7, 0), "mFootLeft": (10, 8, 0)}
        self.assertEqual(convert.rig_unit(cm_kit, "y"), 0.01)
        # A pelvis alone gives no height above the feet.
        self.assertIsNone(convert.rig_unit({"mPelvis": (0, 0, 1.067)}, "z"))
        self.assertEqual(convert.rig_unit({"mPelvis": (0, 0, 1.067), "mChest": (0, 0, 1.3), "mHipLeft": (0, 0.1, 0)}, "z"), 1.0)
        self.assertIsNone(convert.rig_unit({"mChest": (0, 0, 1)}, "z"))

    def test_a_metre_kit_in_a_z_up_scene_reaches_the_viewer_at_full_size(self):
        base = convert.base_frame("z", convert.rig_unit({"mPelvis": (0, 0, 1.067), "mToeLeft": (0.1, 0.1, 0.0)}, "z"))
        bind_pre = {"mPelvis": convert.translation((0, 0, -1.067))}
        [values] = convert.inverse_binds(["mPelvis"], bind_pre, base)
        # A hip vertex 15 cm to the side of the pelvis stays 15 cm away.
        local = convert.point(convert.from_column_major(values), convert.point(base, (0.15, 0, 1.067)))
        self.assertTrue(close(local, (0.15, 0, 0)))

    def test_maya_matrices_are_transposed_to_column_vectors(self):
        maya = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 5, 6, 7, 1]
        matrix = convert.from_maya(maya)
        self.assertEqual(convert.translation_of(matrix), [5, 6, 7])
        # Column-major output is Maya's own flat order.
        self.assertEqual(convert.column_major(matrix), [float(v) for v in maya])
        with self.assertRaises(ValueError):
            convert.from_maya([math.nan] + [0] * 15)

    def test_inverse_round_trips_and_refuses_singular(self):
        matrix = convert.chain(convert.translation((1, 2, 3)), convert.rotation_z(0.7), convert.scale(2))
        product = convert.multiply(matrix, convert.inverse(matrix))
        for row in range(4):
            self.assertTrue(close(product[row], convert.identity()[row]))
        with self.assertRaises(ValueError):
            convert.inverse(convert.scale(0))

    def test_facing_turns_the_rigs_left_to_sl_plus_y(self):
        # A rig facing Maya +Z has its left at +X; after the Y-up turn it faces
        # SL -Y, and a quarter turn brings it to face +X.
        heads = {"mShoulderLeft": convert.point(convert.TO_SL, (20, 140, 0)),
                 "mShoulderRight": convert.point(convert.TO_SL, (-20, 140, 0))}
        rotation = convert.facing(heads)
        self.assertTrue(close(convert.direction(rotation, (1, 0, 0)), (0, 1, 0)))
        # Already facing +X (left at +Y): no turn.
        self.assertEqual(convert.facing({"mHipLeft": (0, 0.1, 1), "mHipRight": (0, -0.1, 1)}), convert.identity())
        self.assertEqual(convert.facing({}), convert.identity())

    def test_joint_names_drop_namespaces_and_paths_or_take_the_override(self):
        self.assertEqual(convert.joint_name("|rig|ns:mPelvis"), "mPelvis")
        self.assertEqual(convert.joint_name("Hips", "mPelvis"), "mPelvis")


class Binds(unittest.TestCase):
    def test_a_vertex_at_the_bind_head_lands_on_the_joint_origin(self):
        frame = convert.multiply(convert.rotation_z(math.pi / 2), convert.TO_SL)
        head = (0.0, 100.0, 5.0)
        bind_pre = {"mPelvis": convert.translation([-value for value in head])}
        [values] = convert.inverse_binds(["mPelvis"], bind_pre, frame)
        inverse_bind = convert.from_column_major(values)
        local = convert.point(inverse_bind, convert.point(frame, head))
        self.assertTrue(close(local, (0, 0, 0)))
        # A centimetre in Maya is a centimetre in the joint's frame.
        moved = convert.point(inverse_bind, convert.point(frame, (0.0, 101.0, 5.0)))
        self.assertAlmostEqual(math.dist(moved, (0, 0, 0)), 0.01)

    def test_axes_mode_keeps_the_position_and_takes_the_viewers_axes(self):
        rotated = convert.chain(convert.translation((0, 100, 0)), convert.rotation_z(0.5))
        bind_pre = {"mChest": convert.inverse(rotated)}
        axes = convert.column_major(convert.rotation_z(0.25))
        [values] = convert.inverse_binds(["mChest"], bind_pre, convert.TO_SL, True, {"mChest": axes})
        expected = convert.multiply(convert.rotation_z(0.25), convert.translation((0, 0, -1)))
        self.assertTrue(close(values, convert.column_major(expected)))
        # Without axes from the viewer: identity rotation.
        [values] = convert.inverse_binds(["mChest"], bind_pre, convert.TO_SL, True, {})
        self.assertTrue(close(values, convert.column_major(convert.translation((0, 0, -1)))))


class Weights(unittest.TestCase):
    def test_rank_keeps_the_four_strongest_normalized_ties_to_lower_index(self):
        joints, weights = convert.rank([(5, 0.1), (1, 0.4), (2, 0.4), (3, 0.05), (4, 0.2), (6, 0.0)])
        self.assertEqual(joints, (1, 2, 4, 5))
        self.assertAlmostEqual(sum(weights), 1.0)
        self.assertAlmostEqual(weights[0], 0.4 / 1.1)
        self.assertEqual(convert.rank([]), ((0, 0, 0, 0), (0.0, 0.0, 0.0, 0.0)))
        with self.assertRaises(ValueError):
            convert.rank([(0, math.inf)])

    def test_unweighted_vertices_take_the_nearest_weighted_ones(self):
        positions = [(0, 0, 0), (1, 0, 0), (0.1, 0, 0), (0.9, 0, 0), (5, 5, 5)]
        joints = [(0, 0, 0, 0), (1, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)]
        weights = [(1.0, 0, 0, 0), (1.0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)]
        convert.complete(joints, weights, positions, [0, 1, 2, 3])
        self.assertEqual(joints[2][0], 0)
        self.assertEqual(joints[3][0], 1)
        self.assertEqual(weights[4], (0, 0, 0, 0))  # Not referenced: untouched.

    def test_nearest_search_matches_brute_force(self):
        import random
        generator = random.Random(4)
        positions = [(generator.random() * 3, generator.random(), generator.random() * 0.2) for _ in range(400)]
        joints = [((index % 7), 0, 0, 0) if index % 3 == 0 else (0, 0, 0, 0) for index in range(400)]
        weights = [(1.0, 0, 0, 0) if index % 3 == 0 else (0, 0, 0, 0) for index in range(400)]
        expected = list(joints)
        extent = [max(p[a] for p in positions) - min(p[a] for p in positions) for a in range(3)]
        weighted = [index for index in range(400) if index % 3 == 0]
        for index in range(400):
            if index % 3:
                nearest = min(weighted, key=lambda other: (sum(((positions[index][a] - positions[other][a]) / extent[a]) ** 2
                                                              for a in range(3)), other))
                expected[index] = joints[nearest]
        convert.complete(joints, weights, positions, list(range(400)))
        self.assertEqual(joints, expected)

    def test_a_skin_without_any_weight_goes_whole_to_the_first_joint(self):
        joints, weights = [(0, 0, 0, 0)] * 2, [(0.0, 0.0, 0.0, 0.0)] * 2
        convert.complete(joints, weights, [(0, 0, 0), (1, 1, 1)], [0, 1])
        self.assertEqual(weights, [(1.0, 0.0, 0.0, 0.0)] * 2)


class Surfaces(unittest.TestCase):
    def test_triangle_offsets_global_or_relative(self):
        # A quad then a triangle: global face-vertex offsets...
        triangles, polygons = convert.triangle_corners([2, 1], [0, 1, 2, 0, 2, 3, 4, 5, 6], [4, 3])
        self.assertEqual(triangles, [(0, 1, 2), (0, 2, 3), (4, 5, 6)])
        self.assertEqual(polygons, [0, 0, 1])
        # ...and the same offsets relative to their polygon.
        triangles, _ = convert.triangle_corners([2, 1], [0, 1, 2, 0, 2, 3, 0, 1, 2], [4, 3])
        self.assertEqual(triangles, [(0, 1, 2), (0, 2, 3), (4, 5, 6)])

    def test_corners_are_shared_per_slot_and_packed_for_the_wire(self):
        positions = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
        normals = [(0, 0, 1)]
        uvs = [(0, 0), (1, 0), (1, 1), (0, 1)]
        fv_vertex, fv_normal, fv_uv = [0, 1, 2, 3], [0, 0, 0, 0], [0, 1, 2, 3]
        packed = convert.surfaces(positions, normals, uvs, fv_vertex, fv_normal, fv_uv, [(0, 1, 2), (0, 2, 3)], [0, 0], False)
        records, indices = packed[0]
        self.assertEqual(len(records), 4 * 52)
        self.assertEqual(struct.unpack("<6I", indices), (0, 1, 2, 0, 2, 3))
        flipped = convert.surfaces(positions, normals, uvs, fv_vertex, fv_normal, fv_uv, [(0, 1, 2)], [0], True)
        first = convert.VERTEX.unpack(flipped[0][0][:52])
        self.assertEqual(first[:3], (1.0, 1.0, 0.0))  # Winding reversed.
        two = convert.surfaces(positions, normals, uvs, fv_vertex, fv_normal, fv_uv, [(0, 1, 2), (0, 2, 3)], [0, 3], False)
        self.assertEqual(sorted(two), [0, 3])
        self.assertEqual(convert.corner_count(two), 6)

    def test_packed_surfaces_go_through_the_shared_client_in_both_wire_forms(self):
        packed = convert.surfaces([(0, 0, 0), (1, 0, 0), (0, 1, 0)], [(0, 0, 1)], [], [0, 1, 2], [0, 0, 0], [-1, -1, -1],
                                  [(0, 1, 2)], [0], False, [(0, 0, 0, 0)] * 3, [(1.0, 0, 0, 0)] * 3)
        records, indices = packed[0]
        color, material = convert.default_material()
        snapshot = {"protocol": 1, "revision": 1, "images": [], "meshes": [{
            "name": "Cube", "source_id": "6c7e3b04-4e8c-6e3a-b3b1-4c8a5d9f2e4b", "joints": ["mPelvis"],
            "inverse_bind": [convert.column_major(convert.identity())],
            "faces": [{"color": color, "material": material, "_buffers": (records, indices)}]}]}
        wire = client.wire_snapshot(snapshot, ["packed-geometry", "source-ids"])
        face = wire["meshes"][0]["faces"][0]
        self.assertEqual(base64.b64decode(face["packed"]["vertices"]), records)
        self.assertNotIn("_buffers", face)
        old = client.wire_snapshot(snapshot, [])
        face = old["meshes"][0]["faces"][0]
        self.assertEqual(face["indices"], [0, 1, 2])
        self.assertEqual(face["vertices"][0]["weights"], [1.0, 0.0, 0.0, 0.0])
        json.dumps(old, allow_nan=False)


class Materials(unittest.TestCase):
    def test_place2d_repeat_and_offset_become_the_viewers_transform(self):
        self.assertEqual(convert.uv_transform(), [0.0, 0.0, 1.0, 1.0, 0.0])
        self.assertEqual(convert.uv_transform((2, 2), (0.25, 0.5)), [0.25, -1.5, 2.0, 2.0, 0.0])

    def test_colour_tables_keep_alpha_and_ends(self):
        self.assertEqual((convert.TO_LINEAR[0], convert.TO_LINEAR[255], convert.TO_SRGB[255]), (0, 255, 255))
        self.assertEqual(convert.TO_LINEAR[188], 128)  # sRGB 0.737 is about linear 0.5.
        pixels = bytes([188, 188, 188, 7])
        self.assertEqual(convert.recode(pixels, convert.TO_LINEAR), bytes([128, 128, 128, 7]))
        self.assertEqual(convert.channel(bytes([1, 2, 3, 4]), 3), bytes([4, 4, 4, 255]))
        self.assertEqual(convert.opaque(bytes([1, 2, 3, 4])), bytes([1, 2, 3, 255]))

    def test_material_factors_stay_in_the_viewers_ranges(self):
        color, material = convert.default_material((1.2, -0.1, 0.5, 1.0))
        material.update(metallic=3.0, roughness=math.nan, emissive=[100.0, -1.0, 2.0])
        color, material = convert.clamp_material(color, material)
        self.assertEqual(color, [1.0, 0.0, 0.5, 1.0])
        self.assertEqual((material["metallic"], material["roughness"]), (1.0, 0.0))
        self.assertEqual(material["emissive"], [64.0, 0.0, 2.0])

    def test_orm_packs_roughness_green_metallic_blue(self):
        rough, metal = bytes([10, 0, 0, 0, 20, 0, 0, 0]), bytes([30, 0, 0, 0, 40, 0, 0, 0])
        self.assertEqual(convert.orm((2, 1), (rough, 0), (metal, 0)), bytes([255, 10, 30, 255, 255, 20, 40, 255]))
        self.assertEqual(convert.orm((1, 1), None, (bytes([0, 9, 0, 0]), 1)), bytes([255, 255, 9, 255]))
        with self.assertRaises(ValueError):
            convert.orm((2, 1), (bytes(4), 0))

    def test_images_are_deduplicated_bounded_and_rolled_back(self):
        images = convert.Images()
        self.assertEqual(images.store(1, 1, bytes(4)), 0)
        self.assertEqual(images.store(1, 1, bytes(4)), 0)
        mark = images.mark()
        self.assertEqual(images.store(1, 1, bytes([1, 2, 3, 4])), 1)
        images.rollback(mark)
        self.assertEqual(len(images.images), 1)
        for index in range(15):
            images.store(1, 1, bytes([index + 10, 0, 0, 0]))
        with self.assertRaises(ValueError):
            images.store(1, 1, bytes([200, 0, 0, 0]))
        self.assertEqual(convert.fit(4096, 2048), (1024, 512))
        self.assertEqual(convert.fit(10, 10), (10, 10))


class Links(unittest.TestCase):
    def test_static_parenting_resolves_to_the_highest_synchronized_ancestor(self):
        roots = convert.static_roots({"a": None, "b": "a", "c": "b", "d": None})
        self.assertEqual(roots, {"a": "a", "b": "a", "c": "a", "d": "d"})
        with self.assertRaises(ValueError):
            convert.static_roots({"a": "b", "b": "a"})


class Lods(unittest.TestCase):
    def test_settings_round_trip_with_namespaces_and_defaults(self):
        settings = {"ID-1": {"medium": dict(convert.lod_default("medium"), object="ns:Body_mid", mode="existing"),
                             "low": dict(convert.lod_default("low"), budget="triangles", value=1500),
                             "lowest": convert.lod_default("lowest")}}
        text = convert.lod_settings_format(settings)
        self.assertNotIn('"', text)
        self.assertEqual(convert.lod_settings_parse(text), settings)
        # Damaged text falls back to the defaults, level by level.
        parsed = convert.lod_settings_parse("ID-2=bogus,auto||_low|ratio|nan;=x;")
        self.assertEqual(parsed["ID-2"]["medium"], convert.lod_default("medium"))
        self.assertEqual(parsed["ID-2"]["low"]["mode"], "auto")
        self.assertEqual(parsed["ID-2"]["low"]["value"], 25.0)
        self.assertEqual(convert.lod_default("lowest")["value"], 12.5)

    def test_names_and_targets_follow_the_blender_plugin(self):
        self.assertEqual(convert.lod_name("Body_HIGH", "_low"), "Body_low")
        self.assertEqual(convert.lod_name("Body", "_medium"), "Body_medium")
        self.assertEqual(convert.lod_target(1000, convert.lod_default("medium")), 500)
        self.assertEqual(convert.lod_target(1000, dict(convert.lod_default("low"), budget="triangles", value=40)), 40)
        self.assertEqual(convert.lod_target(3, convert.lod_default("lowest")), 1)

    def test_palettes_merge_and_refuse_disagreeing_binds(self):
        identity = convert.column_major(convert.identity())
        moved = convert.column_major(convert.translation((0, 0, -1)))
        vertex = lambda joints: {"position": [0, 0, 0], "normal": [0, 0, 1], "uv": [0, 0], "joints": joints,
                                 "weights": [1, 0, 0, 0]}
        high = {"joints": ["mChest", "mPelvis"], "inverse_bind": [identity, moved], "faces": [{"vertices": [vertex([1, 0, 0, 0])]}]}
        low = {"joints": ["mPelvis"], "inverse_bind": [moved], "faces": [{"vertices": [vertex([0, 0, 0, 0])]}]}
        convert.merge_palettes([low, high])
        self.assertEqual(low["joints"], ["mPelvis", "mChest"])
        self.assertEqual(high["joints"], low["joints"])
        self.assertEqual(high["faces"][0]["vertices"][0]["joints"][0], 0)  # mPelvis, renumbered
        self.assertEqual(low["faces"][0]["vertices"][0]["joints"][0], 0)
        bad = {"joints": ["mPelvis"], "inverse_bind": [identity], "faces": []}
        with self.assertRaises(ValueError):
            convert.merge_palettes([bad, high])
        static = [{"joints": [], "inverse_bind": [], "faces": [{"vertices": [vertex([0, 0, 0, 0])]}]}]
        convert.merge_palettes(static)
        self.assertEqual(static[0]["joints"], [])

    def test_lod_surfaces_map_to_high_by_shading_group(self):
        self.assertEqual(convert.slot_mapping(["skinSG", "nailsSG"], ["nailsSG", "skinSG", "eyesSG"]), [1, 0])
        self.assertEqual(convert.slot_mapping([None], [None]), [0])
        with self.assertRaises(ValueError):
            convert.slot_mapping(["otherSG"], ["skinSG"])
        with self.assertRaises(ValueError):
            convert.slot_mapping(["skinSG", "skinSG"], ["skinSG"])

    def test_reduced_vertices_take_the_nearest_high_weights(self):
        high = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (5, 5, 5)]
        joints = [(0, 0, 0, 0), (1, 0, 0, 0), (2, 0, 0, 0), (3, 0, 0, 0)]
        weights = [(1.0, 0, 0, 0)] * 4
        result, _weights = convert.transfer(high, joints, weights, [0, 1, 2], [(0.9, 0.1, 0), (0.1, 0.8, 0), (-9, 0, 0)])
        self.assertEqual([j[0] for j in result], [1, 2, 0])


def shape_payload(action="create"):
    joints = shape_rules.skeleton()
    names = [name for name, _parent in joints]
    entries = []
    for index, name in enumerate(names):
        head = [0.01 * index, 0.0, 1.0]
        entries.append({"name": name, "head": head, "tail": [head[0], 0.0, 1.03], "rotation": [0, 0, 0, 1],
                        "inverse_bind": convert.column_major(convert.translation([-v for v in head])),
                        "reference_bind": convert.column_major(convert.identity())})
    part = {"name": "upper", "positions": [[0, 0, 1], [0.1, 0, 1], [0, 0.1, 1]], "uvs": [[0, 0], [1, 0], [0, 1]],
            "weights": [[[0, 1.0]], [[0, 0.5], [1, 0.5]], [[2, 1.0]]], "triangles": [[0, 1, 2]]}
    return {"id": "3f1c9a52-7d10-4f4e-9a55-2a1b3c4d5e6f", "name": "My shape", "action": action, "params": {"80": 1.0},
            "names": names, "joints": entries, "parts": [part]}


class Shape(unittest.TestCase):
    def test_the_bundled_skeleton_has_every_sl_joint_in_pre_order(self):
        joints = shape_rules.skeleton()
        self.assertEqual(len(joints), 159)
        self.assertEqual(joints[0], ("mPelvis", None))
        parents = {name: parent for name, parent in joints}
        seen = set()
        for name, parent in joints:
            self.assertTrue(parent is None or parent in seen, name)  # parents come first
            seen.add(name)
        self.assertEqual(parents["mChest"], "mSpine4")  # Bento spine joints sit between

    def test_a_valid_transfer_passes_and_bad_ones_are_refused(self):
        joints = shape_rules.skeleton()
        shape_rules.validate(shape_payload(), joints)
        shape_rules.validate(shape_payload("apply"), joints)
        cases = []
        bad = shape_payload(); bad["names"] = bad["names"][:-1]; cases.append(bad)
        bad = shape_payload(); bad["joints"][3]["rotation"] = [0, 0, 0, 2]; cases.append(bad)
        bad = shape_payload(); bad["joints"][3]["inverse_bind"] = convert.column_major(convert.identity()); cases.append(bad)
        bad = shape_payload(); bad["joints"][3]["head"] = [math.nan, 0, 0]; cases.append(bad)
        bad = shape_payload(); bad["parts"][0]["weights"][1] = [[0, 0.5], [1, 0.4]]; cases.append(bad)
        bad = shape_payload(); bad["parts"][0]["triangles"] = [[0, 1, 9]]; cases.append(bad)
        bad = shape_payload(); bad["parts"][0]["name"] = "hair"; cases.append(bad)
        bad = shape_payload(); bad["action"] = "delete"; cases.append(bad)
        bad = shape_payload(); bad["params"] = {"x": 1}; cases.append(bad)
        bad = shape_payload(); bad["id"] = "not a uuid"; cases.append(bad)
        bad = shape_payload("apply"); bad["joints"][0]["reference_bind"] = [0] * 16; cases.append(bad)
        for case in cases:
            with self.assertRaises(ValueError):
                shape_rules.validate(case, joints)

    def test_a_new_skeleton_comes_back_to_the_shapes_positions(self):
        # Built with placement(), captured with the frame capture finds for
        # it (base, then the facing turn): the SL positions round trip.
        left, right, pelvis = (0.0, 0.2, 1.4), (0.0, -0.2, 1.4), (0.0, 0.0, 1.0)
        for up, unit in (("y", 0.01), ("z", 0.01), ("y", 1.0), ("z", 1.0)):
            to_maya = shape_rules.placement(up, unit)
            maya = {name: package_convert.point(to_maya, p) for name, p in
                    (("mShoulderLeft", left), ("mShoulderRight", right), ("mPelvis", pelvis))}
            base = package_convert.base_frame(up, unit)
            heads = {name: package_convert.point(base, p) for name, p in maya.items()}
            frame = package_convert.multiply(package_convert.facing(heads), base)
            for name, sl in (("mShoulderLeft", left), ("mShoulderRight", right), ("mPelvis", pelvis)):
                self.assertTrue(close(package_convert.point(frame, maya[name]), sl), (up, unit, name))
        # A Y-up skeleton stands up Maya's Y and faces +Z, left on +X.
        to_maya = shape_rules.placement("y", 0.01)
        self.assertTrue(close(package_convert.point(to_maya, (0, 0, 1)), (0, 100, 0)))
        self.assertTrue(close(package_convert.point(to_maya, (1, 0, 0)), (0, 0, 100)))
        self.assertTrue(close(package_convert.point(to_maya, (0, 1, 0)), (100, 0, 0)))

    def test_shaped_joints_use_the_transferred_bind(self):
        frame = convert.TO_SL
        shaped_bind = convert.column_major(convert.chain(convert.scale(0.8), convert.translation((0, 0, -1.1))))
        # Bound where the shape put the joint: the transferred bind as it is.
        bind_pre = {"mPelvis": convert.translation((0, -110, 0))}
        [values] = convert.inverse_binds(["mPelvis"], bind_pre, frame, shaped={"mPelvis": (shaped_bind, [0, 0, 1.1])})
        self.assertTrue(close(values, shaped_bind))
        # The rig moved 1 m before binding: the offset is taken back out.
        moved = {"mPelvis": convert.translation((-100, -110, 0))}
        [values] = convert.inverse_binds(["mPelvis"], moved, frame, shaped={"mPelvis": (shaped_bind, [0, 0, 1.1])})
        local = convert.point(convert.from_column_major(values), (1.0, 0, 1.1))
        self.assertTrue(close(local, (0, 0, 0)))

    def test_node_names_are_safe(self):
        self.assertEqual(shape_rules.node_name("My shape (v2)"), "My_shape__v2")
        self.assertEqual(shape_rules.node_name("2nd"), "Shape_2nd")
        self.assertEqual(shape_rules.node_name("ゆき"), "Shape")


class ShapeTransport(unittest.TestCase):
    def test_the_maya_client_receives_a_shape_and_answers(self):
        token = b"0123456789abcdef" * 4
        client.read_token = lambda port: token
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        received = {}

        def receive(connection, size):
            data = b""
            while len(data) < size:
                chunk = connection.recv(size - len(data))
                if not chunk:
                    raise EOFError
                data += chunk
            return data

        def frame(connection):
            return receive(connection, struct.unpack("<I", receive(connection, 4))[0])

        def viewer():
            connection, _ = server.accept()
            with connection:
                receive(connection, len(client.MAGIC) + len(token))
                hello = {"protocol": 1, "joints": ["mPelvis"],
                         "features": ["pbr-materials", "shape-transfer", "shape-apply", "import-lods"]}
                connection.sendall(json.dumps(hello).encode() + b"\n")
                received["control"] = json.loads(frame(connection))
                connection.sendall(json.dumps({"ok": True, "shape": {"id": "3f1c9a52-7d10-4f4e-9a55-2a1b3c4d5e6f",
                                                                     "action": "create"}}).encode() + b"\n")
                while True:
                    body = json.loads(frame(connection) or b"{}")
                    if body.get("type") == "shape-result":
                        received["result"] = body
                        connection.sendall(b'{"ok":true}\n')
                        return
                    connection.sendall(b'{"ok":true,"revision":1}\n')

        thread = threading.Thread(target=viewer)
        thread.start()
        live = client.Client(server.getsockname()[1], capabilities=("import-lods", "shape-transfer"))
        payload = None
        for _ in range(200):
            payload = live.take_shape()
            if payload:
                break
            threading.Event().wait(0.02)
        self.assertIsNotNone(payload)
        live.shape_result(payload["id"])
        thread.join(10)
        live.close()
        server.close()
        # Shape creation and Import / LOD; never shape-apply.
        self.assertEqual(received["control"], {"type": "features", "features": ["shape-transfer", "import-lods"]})
        self.assertEqual(received["result"], {"type": "shape-result", "id": payload["id"], "error": None})


class Transport(unittest.TestCase):
    def test_the_maya_client_negotiates_no_blender_controls(self):
        token = b"0123456789abcdef" * 4
        client.read_token = lambda port: token
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        frames = []

        def viewer():
            connection, _ = server.accept()
            with connection:
                greeting = b""
                while len(greeting) < len(client.MAGIC) + len(token):
                    greeting += connection.recv(128)
                hello = {"protocol": 1, "joints": ["mPelvis"], "features": ["pbr-materials", "shape-transfer", "import-lods"]}
                connection.sendall(json.dumps(hello).encode() + b"\n")
                header = connection.recv(4)
                size = struct.unpack("<I", header)[0]
                body = b""
                while len(body) < size:
                    body += connection.recv(size - len(body))
                frames.append(json.loads(body))
                connection.sendall(b'{"ok":true,"revision":1}\n')

        thread = threading.Thread(target=viewer)
        thread.start()
        live = client.Client(server.getsockname()[1], capabilities=())
        live.submit({"protocol": 1, "revision": 1, "meshes": [], "images": []})
        thread.join(10)
        live.close()
        server.close()
        # The first frame is the snapshot, not a features control.
        self.assertEqual(frames[0].get("revision"), 1)
        self.assertNotIn("type", frames[0])


class ImportTransport(unittest.TestCase):
    def test_the_maya_client_negotiates_import_lods_and_answers_with_a_bundle(self):
        token = b"0123456789abcdef" * 4
        client.read_token = lambda port: token
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        received = {}

        def frame(connection):
            size = struct.unpack("<I", receive(connection, 4))[0]
            return receive(connection, size)

        def receive(connection, size):
            data = b""
            while len(data) < size:
                chunk = connection.recv(size - len(data))
                if not chunk:
                    raise EOFError
                data += chunk
            return data

        def viewer():
            connection, _ = server.accept()
            with connection:
                receive(connection, len(client.MAGIC) + len(token))
                hello = {"protocol": 1, "joints": ["mPelvis"],
                         "features": ["pbr-materials", "shape-transfer", "shape-apply", "import-lods"]}
                connection.sendall(json.dumps(hello).encode() + b"\n")
                received["control"] = json.loads(frame(connection))
                request = {"id": "5f0c1a0e-0000-4000-8000-000000000001", "source": "5f0c1a0e-0000-4000-8000-000000000002"}
                connection.sendall(json.dumps({"ok": True, "prepare_import": request}).encode() + b"\n")
                while True:
                    body = frame(connection)
                    if body.startswith(b"LODS"):
                        received["lods"] = json.loads(body[4:])
                        connection.sendall(b'{"ok":true}\n')
                        return
                    connection.sendall(b'{"ok":true,"revision":1}\n')

        thread = threading.Thread(target=viewer)
        thread.start()
        live = client.Client(server.getsockname()[1], capabilities=("import-lods",))
        request = None
        for _ in range(200):
            request = live.take_import()
            if request:
                break
            threading.Event().wait(0.02)
        self.assertIsNotNone(request)
        live.import_result(dict(request, error="test"))
        thread.join(10)
        live.close()
        server.close()
        self.assertEqual(received["control"], {"type": "features", "features": ["import-lods"]})
        self.assertEqual(received["lods"]["error"], "test")
        self.assertEqual(received["lods"]["source"], request["source"])


if __name__ == "__main__":
    unittest.main()
