"""Author a native local cannonball clip; never uploads or touches inventory.

Requires numpy/scipy. The reference skeleton is data, as is the optional
underwater-idle clip whose first pose supplies the final recovery join.
"""
import argparse
import hashlib
import json
import math
import struct
from pathlib import Path

import numpy as np
from scipy.spatial.transform import Rotation as R, RotationSpline

SIDES = [("Left", 1), ("Right", -1)]
BODY = ["mPelvis", "mTorso", "mChest", "mNeck", "mHead"] + [
    "m" + part + side for side, _ in SIDES
    for part in ["Collar", "Shoulder", "Elbow", "Wrist", "Hip", "Knee", "Ankle", "Foot"]
]
FPS, DURATION, LOOP_IN, LOOP_OUT = 30, 2.8, 0.6, 1.2


def euler(x=0, y=0, z=0):
    return R.from_euler("xyz", [x, y, z], degrees=True)


def smooth(u):
    u = np.clip(u, 0, 1)
    return u * u * (3 - 2 * u)


def mix(a, b, u):
    return a * R.from_rotvec((a.inv() * b).as_rotvec() * u)


def unit(v):
    return np.asarray(v) / max(np.linalg.norm(v), 1e-12)


def frame(direction, normal):
    a = unit(direction)
    b = unit(np.asarray(normal) - a * np.dot(a, normal))
    return np.column_stack([a, b, np.cross(a, b)])


def fk(skeleton, pose):
    result = {}
    for bone, geometry in skeleton.items():
        parent = geometry["parent"]
        if parent:
            rotation, position = result[parent]
            result[bone] = (rotation * pose[bone], position + rotation.apply(geometry["pos"]))
        else:
            result[bone] = (pose[bone], np.zeros(3))
    return result


def aim_arm(skeleton, q, side, sign, target, pole):
    upper = np.asarray(skeleton["mElbow" + side]["pos"])
    lower = np.asarray(skeleton["mWrist" + side]["pos"])
    a, b = np.linalg.norm(upper), np.linalg.norm(lower)
    d = np.clip(np.linalg.norm(target), abs(a - b) + .008, a + b - .004)
    direction = unit(target)
    bend = unit(np.asarray(pole) - direction * np.dot(direction, pole))
    along = (a*a - b*b + d*d) / (2*d)
    elbow = direction * along + bend * math.sqrt(max(0, a*a - along*along))
    fore = direction*d - elbow
    normal = unit(np.cross(elbow, fore))
    shoulder = R.from_matrix(frame(elbow, normal) @ frame(upper, [0, 0, -sign]).T)
    a0, b0 = unit(lower), unit(shoulder.inv().apply(fore))
    quaternion = np.r_[np.cross(a0, b0), 1 + np.dot(a0, b0)]
    q["mShoulder" + side] = shoulder
    q["mElbow" + side] = R.from_quat(quaternion / np.linalg.norm(quaternion))
    q["mWrist" + side] = euler(y=-20)


def pose(skeleton, pelvis, torso, chest, hip, knee, grasp):
    q = {bone: R.identity() for bone in BODY}
    q.update(mPelvis=euler(y=pelvis), mTorso=euler(y=torso), mChest=euler(y=chest),
             mNeck=euler(y=-8), mHead=euler(y=-6))
    for side, sign in SIDES:
        q["mCollar" + side] = euler(sign*2, 0, -sign*2)
        q["mHip" + side] = euler(sign*4, -hip, sign*2)
        q["mKnee" + side] = euler(y=knee)
        q["mAnkle" + side] = euler(y=12)
    placed = fk(skeleton, q)
    for side, sign in SIDES:
        rotation, shoulder = placed["mCollar" + side]
        shoulder = shoulder + rotation.apply(skeleton["mShoulder" + side]["pos"])
        knee_position = placed["mKnee" + side][1]
        held = rotation.inv().apply(knee_position + [0.015, sign*.035, -.055] - shoulder)
        free = np.array([.12, sign*.12, -.34])
        target = free * (1-grasp) + held * grasp
        aim_arm(skeleton, q, side, sign, target, [.15, sign, -.15])
    return q


def first_pose(path):
    data, offset = path.read_bytes(), 12

    def get(fmt):
        nonlocal offset
        values = struct.unpack_from("<" + fmt, data, offset)
        offset += struct.calcsize("<" + fmt)
        return values

    def text():
        nonlocal offset
        end = data.index(0, offset)
        value, offset = data[offset:end].decode("utf-8"), end + 1
        return value

    text()
    *_, count = get("ffiffII")
    rotations, pelvis = {}, np.zeros(3)
    for _ in range(count):
        bone = text()
        _, n = get("ii")
        keys = [get("4H") for _ in range(n)]
        if keys and bone in BODY:
            xyz = np.array(keys[0][1:]) / 65535 * 2 - 1
            rotations[bone] = R.from_quat(np.r_[xyz, math.sqrt(max(0, 1-np.dot(xyz, xyz)))])
        n, = get("i")
        keys = [get("4H") for _ in range(n)]
        if keys and bone == "mPelvis":
            pelvis = np.array(keys[0][1:]) / 65535 * 10 - 5
    if set(rotations) != set(BODY):
        raise ValueError("reference idle must contain the 21 standard body joints")
    return rotations, pelvis


def serialize(poses, positions):
    quantize = lambda v, lo, hi: round(float(np.clip((v-lo)/(hi-lo), 0, 1)) * 65535)
    duration = struct.unpack("<f", struct.pack("<f", DURATION))[0]
    keytime = lambda t: round(t / DURATION * 65535) / 65535 * duration
    data = bytearray(struct.pack("<HHif", 1, 0, 6, duration) + b"\0")
    data.extend(struct.pack("<ffiffII", keytime(LOOP_IN), keytime(LOOP_OUT), 1, .08, .25, 1, len(BODY)))
    last = len(poses) - 1
    for bone in BODY:
        data.extend(bone.encode() + b"\0" + struct.pack("<ii", 6, last+1))
        for i, q in enumerate(poses):
            quaternion = q[bone].as_quat()
            if quaternion[3] < 0:
                quaternion = -quaternion
            data.extend(struct.pack("<4H", round(i/last*65535), *[quantize(v, -1, 1) for v in quaternion[:3]]))
        data.extend(struct.pack("<i", last+1 if bone == "mPelvis" else 0))
        if bone == "mPelvis":
            for i, p in enumerate(positions):
                data.extend(struct.pack("<4H", round(i/last*65535), *[quantize(v, -5, 5) for v in p]))
    return data + struct.pack("<i", 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skeleton", type=Path, required=True)
    parser.add_argument("--idle", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    skeleton = json.loads(args.skeleton.read_text(encoding="utf-8"))
    idle, pelvis = first_pose(args.idle)
    air = pose(skeleton, 4, 4, 2, 12, 25, 0)
    half = pose(skeleton, 18, 10, 8, 70, 110, .65)
    tuck = pose(skeleton, 25, 15, 10, 115, 155, 1)
    open_pose = pose(skeleton, 20, 10, 8, 75, 115, .25)
    intro = {b: RotationSpline([0, .12, .35, LOOP_IN], R.from_quat([q[b].as_quat() for q in [air, air, half, tuck]])) for b in BODY}
    outro = {b: RotationSpline([LOOP_OUT, 1.65, 2.2, DURATION], R.from_quat([q[b].as_quat() for q in [tuck, open_pose, idle, idle]])) for b in BODY}
    poses, positions, distances = [], [], []
    for i in range(round(DURATION*FPS)+1):
        t = i/FPS
        if t < LOOP_IN:
            q = {b: mix(intro[b](t), tuck[b], smooth((t-.43)/.17)) for b in BODY}
        elif t <= LOOP_OUT:
            q = tuck
        else:
            q = {b: mix(tuck[b], outro[b](t), smooth((t-LOOP_OUT)/.15)) for b in BODY}
            q = {b: mix(q[b], idle[b], smooth((t-2.5)/.3)) for b in BODY}
        poses.append(q)
        positions.append(pelvis * smooth((t-LOOP_OUT)/(DURATION-LOOP_OUT)))
        placed = fk(skeleton, q)
        distances.append(float(np.linalg.norm(placed["mWristLeft"][1] - placed["mWristRight"][1])))
    data = serialize(poses, positions)
    assert min(distances) > .08, "hands must remain separate"
    assert all(np.isfinite(q[b].as_quat()).all() for q in poses for b in BODY)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(data)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_bytes(json.dumps({"duration":DURATION,"loop":[LOOP_IN,LOOP_OUT],"priority":6,"joints":BODY,
        "minimum_wrist_separation_m":min(distances),"position_channels":["mPelvis"],"frames":len(poses),
        "sha256":hashlib.sha256(data).hexdigest()},indent=2).encode("utf-8"))
    print(f"Wrote {args.output.name}: {len(data)} bytes, loop {LOOP_IN}..{LOOP_OUT}, wrist clearance {min(distances):.3f} m")


if __name__ == "__main__":
    main()
