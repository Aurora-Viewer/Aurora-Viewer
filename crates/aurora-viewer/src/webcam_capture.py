"""Local companion for Aurora's webcam tracking."""

import argparse
import base64
from collections import deque
from concurrent.futures import ThreadPoolExecutor
import json
from itertools import count
import math
import os
from pathlib import Path
from queue import Empty, Queue
import re
import shutil
from statistics import median
import subprocess
import sys
from tempfile import TemporaryDirectory
from threading import Condition, Lock, Thread
import time
from urllib.parse import parse_qs, urlsplit
from urllib.request import Request, urlopen


FACE_INDICES = (263, 33, 10, 152, 13, 14, 61, 291, 362, 386, 374, 473, 133, 159, 145, 468)
# Arms, paired hips, knees, ankles and toe landmarks.
BODY_INDICES = (11, 13, 15, 12, 14, 16, 23, 24, 25, 26, 27, 28, 31, 32)
PREVIEW_SIZE = 384
TRACKING_SIZE = 480
FILE_TRACKING_SIZE = 720
OUTPUT_INTERVAL = 1 / 30
FILE_LOOKAHEAD = 0.28
CONTACT_IDS = count(1)
MEDIA_LIMIT = 128 * 1024 * 1024
SCRIPT_BOOT = "import sys; _aurora_script = sys.stdin.buffer.read(int(sys.stdin.buffer.readline())); exec(compile(_aurora_script, '<aurora-webcam>', 'exec'))"


def points(landmarks, indices, aspect=1.0):
    # SL avatar coordinates: +X forward, +Y avatar left, +Z up.
    return [[-p.z, p.x, -p.y * aspect] for p in (landmarks.landmark[i] for i in indices)]


def list_cameras():
    import cv2
    from cv2_enumerate_cameras import enumerate_cameras

    # OBS Virtual Camera is available through DirectShow on Windows.
    backend = {
        "win32": cv2.CAP_DSHOW,
        "linux": cv2.CAP_V4L2,
        "darwin": cv2.CAP_AVFOUNDATION,
    }.get(sys.platform, cv2.CAP_ANY)
    print(json.dumps([
        {"index": device.index, "backend": backend, "name": device.name}
        for device in enumerate_cameras(backend)
    ]))


def resize(cv2, image, limit):
    height, width = image.shape[:2]
    longest = max(width, height)
    if longest <= limit:
        return image
    ratio = limit / longest
    return cv2.resize(image, (max(1, round(width * ratio)), max(1, round(height * ratio))))


def dots(landmarks, step=1, visible_only=False):
    if landmarks is None:
        return []
    return [
        [round(p.x, 4), round(p.y, 4)]
        for p in landmarks.landmark[::step]
        if not visible_only or p.visibility >= 0.55
    ]


def visible(point, threshold=0.55):
    return point.visibility >= threshold and 0 <= point.x <= 1 and 0 <= point.y <= 1


def distance(a, b, aspect):
    return math.hypot(a.x - b.x, (a.y - b.y) * aspect)


def head_crop(cv2, image, pose):
    if not pose:
        return image, None
    p = pose.landmark
    anchor = next((p[i] for i in (0, 7, 8) if visible(p[i], 0.4)), None)
    if anchor is None:
        return image, None
    h, w = image.shape[:2]
    aspect = h / w
    eye_span = distance(p[2], p[5], aspect)
    ear_span = distance(p[7], p[8], aspect)
    shoulder_span = distance(p[11], p[12], aspect)
    side = max(min(w, h) * 0.12, w * eye_span * 5.5, w * ear_span * 2.2, w * shoulder_span * 0.55)
    x = round(anchor.x * w)
    y = round(anchor.y * h)
    half = round(side / 2)
    x0, x1 = max(0, x - half), min(w, x + half)
    y0, y1 = max(0, y - half), min(h, y + half)
    crop = image[y0:y1, x0:x1]
    if crop.size == 0:
        return image, None
    if max(crop.shape[:2]) < 192:
        factor = 192 / max(crop.shape[:2])
        crop = cv2.resize(crop, None, fx=factor, fy=factor)
    elif not crop.flags.c_contiguous:
        crop = crop.copy()
    return crop, (x0 / w, y0 / h, (x1 - x0) / w, (y1 - y0) / h)


def anchored_face(face, region, pose, aspect):
    if not face.multi_face_landmarks:
        return None
    face_points = face.multi_face_landmarks[0]
    f = face_points.landmark
    if region:
        x, y, w, h = region
        for point in f:
            point.x = x + point.x * w
            point.y = y + point.y * h
            point.z *= w
    if not pose:
        return face_points
    p = pose.landmark
    if not visible(p[0], 0.40):
        return None if visible(p[11]) and visible(p[12]) else face_points
    eyes = max(distance(p[2], p[5], aspect), 0.012)
    face_width = distance(f[33], f[263], aspect)
    if distance(p[0], f[1], aspect) > max(0.02, eyes * 1.25) or not 0.35 * eyes <= face_width <= 2.8 * eyes:
        return None
    return face_points


def anchored_hands(hands, pose, aspect):
    hand_points = hands.multi_hand_landmarks or ()
    identities = [
        ("left" if value.classification[0].label == "Right" else "right", value.classification[0].score)
        for value in (hands.multi_handedness or ())
    ]
    p = pose.landmark if pose else None
    if p is None or not (visible(p[15]) or visible(p[16]) or (visible(p[11]) and visible(p[12]))):
        # Labels assume a horizontally mirrored selfie.
        return {
            side: hand for hand, (side, confidence) in zip(hand_points, identities) if confidence >= 0.65
        }
    candidates = {"left": [(0.20, -1)], "right": [(0.20, -1)]}
    for i, hand in enumerate(hand_points):
        for side, wrist, elbow in (("left", 15, 13), ("right", 16, 14)):
            w = p[wrist]
            if visible(w) or (w.visibility >= 0.35 and visible(p[elbow]) and 0 <= w.x <= 1 and 0 <= w.y <= 1):
                separation = distance(p[wrist], hand.landmark[0], aspect)
                if separation <= max(0.04, min(0.12, distance(p[wrist], p[elbow], aspect) * 0.75)):
                    label, confidence = identities[i] if i < len(identities) else (side, 0.0)
                    # Handedness breaks ties at crossed wrists. A clear
                    # pose anchor outweighs the model's selfie label.
                    penalty = 0.024 * confidence if label != side else 0.0
                    candidates[side].append((separation + penalty, i))
    _, left, right = min(
        (a + b, left, right)
        for a, left in candidates["left"]
        for b, right in candidates["right"]
        if left != right or left == -1
    )
    return {side: hand_points[i] for side, i in (("left", left), ("right", right)) if i >= 0}


def pose_is_hand(pose, hands):
    """An isolated hand can resemble a tiny full skeleton to Pose."""
    candidates = hands.multi_hand_landmarks or ()
    if not pose or len(candidates) != 1:
        return False
    hand = candidates[0].landmark
    x0, x1 = min(p.x for p in hand), max(p.x for p in hand)
    y0, y1 = min(p.y for p in hand), max(p.y for p in hand)
    if max(x1 - x0, y1 - y0) < 0.30:
        return False
    torso = (pose.landmark[i] for i in (0, 11, 12, 23, 24))
    return sum(x0 <= p.x <= x1 and y0 <= p.y <= y1 for p in torso) >= 4


def landmarks(body, hands, face_points, aspect, file_motion=False):
    frame = {}
    pose2d = body.pose_landmarks
    if pose_is_hand(pose2d, hands):
        pose2d = None
        face_points = None
    if body.pose_world_landmarks and pose2d:
        pose = body.pose_world_landmarks
        frame["body"] = [
            xyz if visible(pose2d.landmark[i]) and all(math.isfinite(value) for value in xyz) else None
            for i, xyz in zip(BODY_INDICES, points(pose, BODY_INDICES))
        ]
        p = pose2d.landmark
        if file_motion:
            screen = [screen_point(p[i], aspect) if xyz is not None else None
                      for i, xyz in zip(BODY_INDICES, frame["body"])]
            frame["_screen"] = screen
            frame["profile"] = stabilize_profile(frame["body"], screen)
        if visible(p[0], 0.50) and (visible(p[7]) or visible(p[8])) and visible(p[7], 0.25) and visible(p[8], 0.25):
            head = points(pose, (0, 7, 8))
            if all(math.isfinite(value) for point in head for value in point):
                frame["head"] = head
    hand_points = anchored_hands(hands, pose2d, aspect)
    for side, hand in hand_points.items():
        frame[side] = points(hand, range(21), aspect)
    if face_points:
        frame["face"] = points(face_points, FACE_INDICES, aspect)
    groups = [
        dots(pose2d, visible_only=True),
        dots(face_points, 5),
        *(dots(hand_points.get(side)) for side in ("left", "right")),
    ]
    return frame, groups


def screen_point(point, aspect):
    # Visibility can be high just beyond the crop. Such a joint is unobserved.
    return [0.0, point.x / aspect, -point.y] if 0 <= point.x <= 1 and 0 <= point.y <= 1 else None


def mix(a, b, amount):
    return [x + (y - x) * amount for x, y in zip(a, b)]


def stabilize_profile(body, screen):
    shoulders = (0, 3)
    hips = (6, 7)
    if not all(screen[i] is not None and body[i] is not None for i in shoulders + hips):
        return 0.0
    top = mix(screen[0], screen[3], 0.5)
    center = mix(screen[6], screen[7], 0.5)
    height = math.dist(top, center)
    if height < 0.08:
        return 0.0
    width = abs(screen[0][1] - screen[3][1]) / height
    weight = max(0.0, min(0.85, (0.53 - width) * 2.4))
    if not weight:
        return 0.0
    hip = mix(body[6], body[7], 0.5)
    # Side views reveal projected bend, while inferred lateral lean is unstable.
    length = math.dist(mix(body[0], body[3], 0.5), hip)
    scale = max(0.35, min(3.0, length / height))
    for i, point in enumerate(screen):
        if point is not None:
            depth = 0.28 if i in (8, 9, 10, 11, 12, 13) else 0.60
            estimate = [hip[0] + (body[i][0] - hip[0]) * depth,
                        hip[1] + (point[1] - center[1]) * scale,
                        hip[2] + (point[2] - center[2]) * scale]
            body[i] = mix(body[i], estimate, weight)
    return weight


class StableLandmarks:
    """Ride through brief occlusions and require a second observation for large jumps."""

    def __init__(self):
        self.body = {}
        self.pending = {}
        self.detail = {}

    def apply(self, frame, timestamp):
        body = frame.get("body", [None] * len(BODY_INDICES))
        predicted = set()
        for i, position in enumerate(body):
            previous = self.body.get(i)
            age = timestamp - previous[0] if previous else math.inf
            if position is None:
                self.pending.pop(i, None)
            elif age < 0.18 and math.dist(previous[1], position) > 0.30 + 2.0 * age:
                candidate = self.pending.get(i)
                self.pending[i] = (timestamp, position)
                if not candidate or timestamp - candidate[0] > 0.15 or math.dist(candidate[1], position) > 0.18:
                    position = None
            if position is not None:
                velocity = [0.0, 0.0, 0.0]
                if 0.008 <= age <= 0.12:
                    change = [(new - old) / age for new, old in zip(position, previous[1])]
                    speed = math.hypot(*change)
                    weight = min(0.6, 1.2 / max(speed, 0.001))
                    velocity = [old * 0.4 + new * weight for old, new in zip(previous[2], change)]
                self.body[i] = (timestamp, position, velocity)
                self.pending.pop(i, None)
            elif age <= 0.12:
                horizon = min(age, 0.055)
                body[i] = [point + speed * horizon for point, speed in zip(previous[1], previous[2])]
                predicted.add(i)
            else:
                body[i] = None
        # Keep synthesized points near the last measured limb length. Measured
        # joints remain untouched, even if they are adjacent to a prediction.
        for a, b in ((0, 1), (1, 2), (3, 4), (4, 5), (6, 8), (8, 10), (10, 12), (7, 9), (9, 11), (11, 13)):
            if body[a] is None or body[b] is None or not (a in predicted or b in predicted):
                continue
            if a not in self.body or b not in self.body:
                continue
            reference = math.dist(self.body[a][1], self.body[b][1])
            length = math.dist(body[a], body[b])
            if min(reference, length) < 0.015:
                continue
            target = max(reference * 0.90, min(reference * 1.10, length))
            moving, fixed = (b, a) if b in predicted else (a, b)
            body[moving] = [x + (y - x) * target / length for x, y in zip(body[fixed], body[moving])]
        if any(position is not None for position in body):
            frame["body"] = body
        for name in ("face", "head", "left", "right"):
            if name in frame:
                self.detail[name] = (timestamp, frame[name])
            elif name in self.detail and timestamp - self.detail[name][0] <= 0.12:
                frame[name] = self.detail[name][1]


ARM_LINKS = ((0, 1), (1, 2), (3, 4), (4, 5))
LEG_LINKS = ((6, 8), (8, 10), (7, 9), (9, 11))
BODY_LINKS = ARM_LINKS + LEG_LINKS + ((10, 12), (11, 13))
BODY_GRAPH = BODY_LINKS + ((0, 3), (6, 7), (0, 6), (3, 7))


class FootContacts:
    """Recognize stationary ankles in the image, before filling missing joints."""

    def __init__(self):
        self.previous = [None, None]
        self.locks = [None, None]

    def update(self, frame, now):
        screen = frame.get("_screen", [None] * 14)
        body = frame.get("body", [None] * 14)
        upright = all(screen[i] is not None for i in (0, 3, 6, 7))
        if upright:
            shoulders = mix(screen[0], screen[3], 0.5)
            hips = mix(screen[6], screen[7], 0.5)
            height = shoulders[2] - hips[2]
            upright = height > 0.08 and height > abs(shoulders[1] - hips[1]) * 1.25
        result = [None, None]
        for side in range(2):
            hip, knee, ankle = (body[i + side] for i in (6, 8, 10))
            point, other = screen[10 + side], screen[11 - side]
            if (not upright or not all(x is not None for x in (hip, knee, ankle, point, other))
                    or point[2] - other[2] > height * 0.24 or hips[2] - point[2] < height * 0.64):
                self.previous[side] = self.locks[side] = None
                continue
            previous = self.previous[side]
            self.previous[side] = (now, point)
            lock = self.locks[side]
            if lock and math.dist(point, lock[1]) <= height * 0.105:
                result[side] = {"id": lock[2], "weight": min(1.0, (now - lock[0]) / 0.095)}
            elif previous and 0.012 <= now - previous[0] <= 0.15 and math.dist(point, previous[1]) <= height * 0.045:
                self.locks[side] = (now, point, next(CONTACT_IDS))
            else:
                self.locks[side] = None
        return result


class FileMotion:
    """Bounded lookahead in media time, reset at each loop or source change."""

    def __init__(self):
        self.samples = deque()
        self.contacts = FootContacts()

    def add(self, now, frame, groups):
        screen = frame.get("_screen")
        body = frame.get("body")
        if screen and body:
            for i in (1, 2, 4, 5, 8, 9, 10, 11, 12, 13):
                if screen[i] is None:
                    body[i] = None
        frame["feet"] = self.contacts.update(frame, now)
        if len(self.samples) >= 2 and body:
            first_at, first, _ = self.samples[-2]
            middle_at, middle, _ = self.samples[-1]
            earlier = first.get("body")
            middle_body = middle.get("body")
            mode = frame.get("planar", False)
            if earlier and middle_body and now - first_at < 0.21 and all(
                sample.get("planar", False) == mode for sample in (first, middle)
            ):
                amount = (middle_at - first_at) / (now - first_at)
                for i, (a, b, c) in enumerate(zip(earlier, middle_body, body)):
                    limit = 0.17 if mode else 0.24
                    if a is not None and b is not None and c is not None and math.dist(a, c) < limit * 0.65:
                        if math.dist(b, mix(a, c, amount)) > limit:
                            middle_body[i] = None
                            if "_screen" in middle:
                                middle["_screen"][i] = None
        self.samples.append((now, frame, groups))
        while self.samples and now - self.samples[0][0] > 1.0:
            self.samples.popleft()

    def at(self, now):
        if not self.samples:
            return None, [], []
        elapsed = [sample for sample in self.samples if sample[0] <= now + 1e-5]
        nearest = elapsed[-1] if elapsed else self.samples[0]
        mode = nearest[1].get("planar", False)
        # Restrict interpolation to the current detector run. 2D and 3D
        # have different origins and scales.
        samples = list(self.samples)
        center = len(elapsed) - 1 if elapsed else 0
        first, last = center, center + 1
        while first and samples[first - 1][1].get("planar", False) == mode:
            first -= 1
        while last < len(samples) and samples[last][1].get("planar", False) == mode:
            last += 1
        samples = samples[first:last]
        before = [x for x in samples if x[0] <= now + 1e-5]
        after = [x for x in samples if x[0] > now + 1e-5]
        output = {"planar": mode}
        output["profile"] = nearest[1].get("profile", 0.0)
        synthesized = set()

        def evaluate(channel, index=None):
            def point(frame):
                group = frame.get(channel)
                return group if index is None or group is None else group[index]

            left = [(t, point(f)) for t, f, _ in reversed(before) if now - t <= 0.27]
            right = [(t, point(f)) for t, f, _ in after if t - now <= 0.27]
            a = next(((t, p) for t, p in left if p is not None), None)
            b = next(((t, p) for t, p in right if p is not None), None)
            if a and b and b[0] - a[0] <= 0.36:
                if channel == "body" and (not before or point(before[-1][1]) is None):
                    synthesized.add(index)
                weight = max(0, min(1, (now - a[0]) / (b[0] - a[0])))
                return mix(a[1], b[1], weight) if index is not None else [mix(x, y, weight) for x, y in zip(a[1], b[1])]
            if a and (now - a[0] < 0.075 or (channel == "body" and index in (1, 2, 4, 5) and now - a[0] < 0.15)):
                if channel == "body" and index in (1, 2, 4, 5) and now - a[0] > 0.001:
                    old = next(((t, p) for t, p in left if p is not None and 0.023 <= a[0] - t <= 0.14), None)
                    if old:
                        synthesized.add(index)
                        speed = math.dist(old[1], a[1]) / (a[0] - old[0])
                        step = min(now - a[0], 0.055) * min(1, 1.0 / max(speed, 0.001))
                        return mix(a[1], old[1], -step / (a[0] - old[0]))
                return a[1]
            return b[1] if b and b[0] - now <= 0.045 else None

        body = [evaluate("body", i) for i in range(14)]
        screen = [evaluate("_screen", i) for i in range(14)]
        # When a wrist leaves the picture for longer than velocity can cover,
        # briefly carry its last forearm direction with the observed elbow.
        for elbow, wrist in ((1, 2), (4, 5)):
            if body[elbow] is None or body[wrist] is not None:
                continue
            for t, frame, _ in reversed(before):
                if now - t >= 0.24:
                    break
                old = frame.get("body")
                if old and old[elbow] is not None and old[wrist] is not None:
                    offset = [w - e for w, e in zip(old[wrist], old[elbow])]
                    length = math.hypot(*offset)
                    offset = mix(offset, [0.0, 0.0, -length], min(0.7, (now - t - 0.10) * 5))
                    body[wrist] = [x + y for x, y in zip(body[elbow], offset)]
                    synthesized.add(wrist)
                    break
        for a, b in BODY_LINKS:
            if body[a] is None or body[b] is None or b not in synthesized:
                continue
            lengths = [math.dist(p[a], p[b]) for _, f, _ in samples
                       if (p := f.get("body")) and p[a] is not None and p[b] is not None]
            length = math.dist(body[a], body[b])
            if lengths and length > 0.01:
                limit = median(lengths) * 1.18
                if length > limit:
                    body[b] = mix(body[a], body[b], limit / length)
        output["body"] = body
        for key in ("head", "face", "left", "right"):
            if (group := evaluate(key)) is not None:
                output[key] = group
        # Fade out a planted target before the first future swing sample.
        feet = list(nearest[1]["feet"])
        for side, foot in enumerate(feet):
            if foot is not None:
                release = next((t for t, f, _ in after if f["feet"][side] is None), None)
                weight = min(foot["weight"], max(0, (release - now) / 0.09)) if release is not None else foot["weight"]
                feet[side] = {**foot, "weight": weight}
        output["feet"] = feet
        return output, nearest[2], screen


def limb_count(frame):
    body = frame.get("body")
    return sum(body[a] is not None and body[b] is not None for a, b in ARM_LINKS + LEG_LINKS) if body else 0


def face_fraction(frame, aspect):
    face = frame.get("face")
    return abs(face[2][2] - face[3][2]) / aspect if face else 0


def usable_planar_pose(candidate, face_height):
    if limb_count(candidate) < 4:
        return False
    body = candidate["body"]
    if not any(all(body[i] for i in indices) for indices in ((6, 8, 10), (7, 9, 11))):
        return False
    ys = [point[2] for point in body if point is not None]
    height = (max(ys) - min(ys)) / 1.65
    # A close-up face can produce hallucinated limbs. Require room for a
    # body below it when the primary detector has already found a face.
    return height >= 0.25 and face_height <= height * 0.40


class PlanarPose:
    """Top-down RTMPose fallback for a single subject filling a file's picture."""

    def __init__(self, cv2, np, path):
        self.cv2 = cv2
        self.np = np
        self.path = path
        self.net = None
        self.failed = False
        self.crop = None
        self.mean = np.array([123.675, 116.28, 103.53], dtype=np.float32)
        self.std = np.array([58.395, 57.12, 57.375], dtype=np.float32)

    def track(self, rgb):
        if self.failed:
            return {}, []
        cv2, np = self.cv2, self.np
        try:
            if self.net is None:
                self.net = cv2.dnn.readNetFromONNX(self.path)
            h, w = rgb.shape[:2]
            center_x, center_y, crop_w, crop_h = self.crop or (w / 2, h / 2, w * 1.15, h * 1.15)
            scale = min(192 / crop_w, 256 / crop_h)
            offset = np.array([96 - center_x * scale, 128 - center_y * scale], dtype=np.float32)
            transform = np.array([[scale, 0, offset[0]], [0, scale, offset[1]]], dtype=np.float32)
            image = cv2.warpAffine(rgb, transform, (192, 256), flags=cv2.INTER_CUBIC,
                                   borderMode=cv2.BORDER_CONSTANT, borderValue=(128, 128, 128)).astype(np.float32)
            self.net.setInput(np.ascontiguousarray(((image - self.mean) / self.std).transpose(2, 0, 1)[None]))
            xs, ys = self.net.forward(["simcc_x", "simcc_y"])
            if xs.shape != (1, 17, 384) or ys.shape != (1, 17, 512):
                raise ValueError("unexpected RTMPose output shape")
            scores = np.minimum(xs.max(axis=-1), ys.max(axis=-1))[0]
            xy = (np.stack([xs.argmax(axis=-1), ys.argmax(axis=-1)], axis=-1)[0] / 2 - offset) / scale
        except (cv2.error, OSError, ValueError):
            self.failed = True
            return {}, []
        points = [(float(x), float(y)) if score >= 0.28 and 0 <= x < w and 0 <= y < h else None
                  for (x, y), score in zip(xy, scores)]
        order = (5, 7, 9, 6, 8, 10, 11, 12, 13, 14, 15, 16)
        core = [points[i] for i in order if points[i]]
        if len(core) >= 9:
            xx, yy = zip(*core)
            if max(yy) - min(yy) >= h * 0.23:
                desired = ((max(xx) + min(xx)) / 2, (max(yy) + min(yy)) / 2,
                           max(w * 0.45, (max(xx) - min(xx)) * 1.68),
                           max(h * 0.48, (max(yy) - min(yy)) * 1.55))
                self.crop = mix(self.crop, desired, 0.30) if self.crop else desired
        elif len(core) < 6:
            self.crop = None
        # Planar estimates do not contain depth or reliable toe/face detail.
        body = [[0.0, (pt[0] - w / 2) / h * 1.65, (h / 2 - pt[1]) / h * 1.65] if pt else None
                for pt in (points[i] for i in order)] + [None, None]
        groups = [[(pt[0] / w, pt[1] / h) for pt in (points[i] for i in order) if pt]]
        screen = [[0.0, pt[0] / h, -pt[1] / h] if pt else None for pt in (points[i] for i in order)] + [None, None]
        return {"body": body, "_screen": screen, "planar": True}, groups


class Tracking:
    """One pending image. Capture can overwrite it while the models are busy."""

    def __init__(self):
        self.changed = Condition()
        self.pending = None
        self.closed = False
        self.overlay = None
        self.file_motion = FileMotion()
        self.revision = 0

    def submit(self, source_id, timestamp, image, still_preview=None, is_file=False):
        with self.changed:
            self.pending = (source_id, self.revision, timestamp, image, still_preview, is_file, self.file_motion)
            self.changed.notify()

    def clear(self):
        with self.changed:
            self.pending = None
            self.overlay = None
            self.file_motion = FileMotion()
            self.revision += 1

    def close(self):
        with self.changed:
            self.closed = True
            self.changed.notify()

    def run(self, cv2, emit):
        import mediapipe as mp
        import numpy as np

        planar_model = os.environ.get("AURORA_WEBCAM_PLANAR_MODEL")
        planar = PlanarPose(cv2, np, planar_model) if planar_model else None

        with (
            mp.solutions.pose.Pose(model_complexity=1) as pose,
            mp.solutions.hands.Hands(model_complexity=0) as hands,
            mp.solutions.face_mesh.FaceMesh(refine_landmarks=True) as face,
            ThreadPoolExecutor(max_workers=3) as pool,
        ):
            trackers = (pose, hands, face)
            blank = np.zeros((256, 256, 3), dtype=np.uint8)
            for warmup in [pool.submit(tracker.process, blank) for tracker in trackers]:
                warmup.result()
            emit({"ready": True})
            last_source = None
            previous_pose = None
            cropped = False
            stable = StableLandmarks()
            misses = 0
            use_planar = False
            saw_primary = False
            while True:
                with self.changed:
                    self.changed.wait_for(lambda: self.pending is not None or self.closed)
                    if self.closed:
                        return
                    source_id, revision, timestamp, image, still_preview, is_file, file_motion = self.pending
                    self.pending = None
                source = (source_id, revision)
                if last_source is not None and last_source != source:
                    for tracker in trackers:
                        tracker.reset()
                    previous_pose = None
                    cropped = False
                    stable = StableLandmarks()
                    if planar is not None:
                        planar.crop = None
                    if last_source[0] != source_id:
                        misses = 0
                        use_planar = False
                        saw_primary = False
                last_source = source
                height, width = image.shape[:2]
                rgb = cv2.cvtColor(resize(cv2, image, FILE_TRACKING_SIZE if is_file else TRACKING_SIZE), cv2.COLOR_BGR2RGB)
                rgb.flags.writeable = False
                if use_planar:
                    frame, groups = planar.track(rgb)
                    frame["planar"] = True
                else:
                    body_task = pool.submit(pose.process, rgb)
                    hand_task = pool.submit(hands.process, rgb)
                    anchor = previous_pose if previous_pose is not None else body_task.result().pose_landmarks
                    face_image, region = head_crop(cv2, rgb, anchor)
                    if bool(region) != cropped:
                        face.reset()
                        cropped = bool(region)
                    face_task = pool.submit(face.process, face_image)
                    body = body_task.result()
                    previous_pose = body.pose_landmarks
                    face_points = anchored_face(face_task.result(), region, previous_pose, height / width)
                    frame, groups = landmarks(body, hand_task.result(), face_points, height / width, is_file and not still_preview)
                    links = limb_count(frame)
                    saw_primary |= links >= 4
                    if is_file and planar is not None and not planar.failed and not saw_primary:
                        misses = misses + 1 if links < 2 and not any(key in frame for key in ("left", "right")) else 0
                        if (misses >= 4 and misses % 4 == 0) or (misses and still_preview):
                            face_height = face_fraction(frame, height / width)
                            if face_height < 0.40:
                                candidate, overlay = planar.track(rgb)
                                if usable_planar_pose(candidate, face_height):
                                    frame, groups = candidate, overlay
                                    stable = StableLandmarks()
                                    use_planar = True
                if not is_file and still_preview is None:
                    stable.apply(frame, timestamp)
                sample = {"id": source_id, "frame": frame}
                if still_preview:
                    sample["preview"] = {"jpeg": still_preview, "groups": groups}
                with self.changed:
                    if is_file and still_preview is None:
                        file_motion.add(timestamp, frame, groups)
                    elif revision == self.revision:
                        frame.pop("_screen", None)
                        self.overlay = (source_id, timestamp, groups)
                        emit(sample)


def read_commands(commands):
    try:
        for line in sys.stdin.buffer:
            command = json.loads(line)
            if command.get("shutdown"):
                break
            commands.put(command)
    finally:
        commands.put(None)


def preview_jpeg(cv2, image):
    ok, jpeg = cv2.imencode(".jpg", resize(cv2, image, PREVIEW_SIZE), [cv2.IMWRITE_JPEG_QUALITY, 68])
    return base64.b64encode(jpeg).decode("ascii") if ok else None


class FileCycle:
    def __init__(self, start, motion):
        self.start = start
        self.motion = motion
        self.preview = deque()
        self.end = None
        self.inferred_at = -1.0
        self.previewed_at = -1.0


def file_playback(cv2, commands, tracking, emit, source, read, rewind, has_audio):
    source_id = source["id"]
    offset = source.get("offset", 0.0)
    first_offset = offset
    out = source.get("out")
    loop_in = source.get("loop_in", 0.0)
    rate = source.get("rate", 1.0)
    step = OUTPUT_INTERVAL * rate
    cycles = deque([FileCycle(0.0, tracking.file_motion)])
    capture_at = output_at = 0.0
    latest_jpeg = None
    played = None
    clock = time.monotonic()
    while True:
        delay = min(capture_at, FILE_LOOKAHEAD + output_at)
        try:
            command = commands.get(timeout=max(0.0, clock + delay / rate - time.monotonic()))
            if command is not None and "loop_setting" in command:
                source["loop_video"] = command["loop_setting"]
                continue
            return command
        except Empty:
            pass
        now = (time.monotonic() - clock) * rate
        if capture_at <= now + 0.001:
            cycle = cycles[-1]
            media_at = first_offset + capture_at - cycle.start
            item = read() if out is None or media_at < out - 0.001 else None
            if item is None:
                if cycle.previewed_at < 0:
                    raise RuntimeError("The selected input did not return a frame.")
                cycle.end = capture_at
                if source["loop_video"]:
                    rewind(loop_in)
                    first_offset = loop_in
                    tracking.clear()
                    cycles.append(FileCycle(capture_at, tracking.file_motion))
                else:
                    capture_at = math.inf
            else:
                image, duration = item
                # Limit model and JPEG work on high-rate files.
                relative = capture_at - cycle.start
                if relative - cycle.inferred_at >= 0.029 * rate:
                    tracking.submit(source_id, relative, image, is_file=True)
                    cycle.inferred_at = relative
                if relative - cycle.previewed_at >= min(OUTPUT_INTERVAL, step) - 0.001:
                    if jpeg := preview_jpeg(cv2, image):
                        cycle.preview.append((capture_at, jpeg, image.shape[0] / image.shape[1]))
                        if latest_jpeg is None:
                            latest_jpeg = jpeg
                            emit({"id": source_id, "preview": {"jpeg": jpeg, "groups": []}})
                    cycle.previewed_at = relative
                capture_at += duration if out is None else min(duration, out - media_at)
        if now + 0.001 < FILE_LOOKAHEAD + output_at:
            continue
        # Skip presentation ticks after a stall without queuing stale poses.
        # Catch the decoder up only as far as frames actually read.
        skip = min(now - FILE_LOOKAHEAD, capture_at) - output_at
        output_at += max(0, int(skip / step)) * step
        while cycles[0].end is not None and output_at >= cycles[0].end - 0.001:
            cycles.popleft()
            if not cycles:
                emit({"id": source_id, "ended": True})
                return {"stop": True}
        cycle = cycles[0]
        if played is not cycle:
            if has_audio:
                start = offset if cycle.start == 0 else loop_in
                emit({"id": source_id, "video_started": start + output_at - cycle.start})
            played = cycle
        with tracking.changed:
            frame, groups, screen = cycle.motion.at(output_at - cycle.start)
        sample = {"id": source_id}
        if frame is not None:
            sample["frame"] = frame
        preview = None
        while cycle.preview and cycle.preview[0][0] <= output_at + 0.001:
            preview = cycle.preview.popleft()
        if preview:
            _, jpeg, aspect = preview
            groups = list(groups) + [[] for _ in range(4 - len(groups))]
            if screen:
                groups[0] = [(p[1] * aspect, -p[2]) for p in screen if p is not None]
                groups.append([(screen[i][1] * aspect, -screen[i][2]) for a, b in BODY_GRAPH
                               if screen[a] is not None and screen[b] is not None for i in (a, b)])
            sample["preview"] = {"jpeg": jpeg, "groups": groups}
        if len(sample) > 1:
            emit(sample)
        output_at += step


def image_playback(cv2, commands, tracking, emit, source):
    import numpy as np
    from PIL import Image, ImageOps

    source_id = source["id"]
    try:
        with Image.open(source["image"]) as picture:
            if picture.width * picture.height > 32_000_000:
                raise RuntimeError("Image is larger than 32 megapixels.")
            animated = picture.format == "GIF" and picture.is_animated
            frame_index = 0

            def read():
                nonlocal frame_index
                try:
                    picture.seek(frame_index)
                except EOFError:
                    return None
                frame = picture if animated else ImageOps.exif_transpose(picture)
                rgba = frame.convert("RGBA")
                rgba.thumbnail((FILE_TRACKING_SIZE, FILE_TRACKING_SIZE))
                background = Image.new("RGBA", rgba.size, (128, 128, 128, 255))
                rgb = Image.alpha_composite(background, rgba).convert("RGB")
                image = cv2.cvtColor(np.asarray(rgb), cv2.COLOR_RGB2BGR)
                frame_index += 1
                return image, max(10, picture.info.get("duration") or 100) / 1000

            def rewind(_offset):
                nonlocal frame_index
                frame_index = 0

            if animated:
                return file_playback(cv2, commands, tracking, emit, source, read, rewind, False)
            image, _ = read()
            jpeg = preview_jpeg(cv2, image)
            if jpeg:
                emit({"id": source_id, "still": True, "preview": {"jpeg": jpeg, "groups": []}})
                tracking.submit(source_id, time.monotonic(), image, jpeg, is_file=True)
            else:
                raise RuntimeError("Cannot preview the selected image.")
        return commands.get()
    except (OSError, Image.DecompressionBombError) as error:
        raise RuntimeError(f"Cannot read the selected image: {error}") from error
    finally:
        tracking.clear()


def playback(cv2, commands, tracking, emit, source):
    if "image" in source:
        return image_playback(cv2, commands, tracking, emit, source)
    source_id = source["id"]
    video = source.get("video")
    if video is not None and hasattr(cv2, "CAP_PROP_N_THREADS"):
        # Frame-threaded decoding introduces buffering and can delay a switch.
        camera = cv2.VideoCapture(video, cv2.CAP_FFMPEG, [cv2.CAP_PROP_N_THREADS, 1])
        if not camera.isOpened():
            camera.open(video)
    elif video is not None:
        camera = cv2.VideoCapture(video)
    else:
        camera = cv2.VideoCapture(source["camera"], source["backend"])
    try:
        if not camera.isOpened():
            raise RuntimeError("Cannot open the selected input.")
        if video is None:
            camera.set(cv2.CAP_PROP_FRAME_WIDTH, 640)
            camera.set(cv2.CAP_PROP_FRAME_HEIGHT, 480)
        fps = camera.get(cv2.CAP_PROP_FPS)
        interval = 1.0 / fps if 1 <= fps <= 240 else 1.0 / 30
        if video is not None:
            def read():
                ok, image = camera.read()
                return (image, interval) if ok else None

            count = camera.get(cv2.CAP_PROP_FRAME_COUNT)
            duration = count * interval if math.isfinite(count) and count > 0 else 0.0
            emit({"id": source_id, "video_duration": duration})

            def rewind(offset):
                frame = int(offset / interval)
                if duration:
                    frame = min(frame, max(0, int(count) - 1))
                if not camera.set(cv2.CAP_PROP_POS_FRAMES, frame):
                    raise RuntimeError("Cannot seek in the selected video.")

            offset = source.get("offset", 0.0)
            if offset > 0:
                rewind(offset)
            if source.get("paused"):
                item = read()
                if item is None:
                    raise RuntimeError("Cannot preview the selected video frame.")
                image, _ = item
                if jpeg := preview_jpeg(cv2, image):
                    emit({"id": source_id, "preview": {"jpeg": jpeg, "groups": []}})
                    tracking.submit(source_id, time.monotonic(), image, jpeg, is_file=True)
                return commands.get()

            return file_playback(cv2, commands, tracking, emit, source, read, rewind, True)
        next_preview = 0.0
        while True:
            try:
                return commands.get_nowait()
            except Empty:
                pass
            ok, image = camera.read()
            if not ok:
                raise RuntimeError("The selected input did not return a frame.")
            timestamp = time.monotonic()
            tracking.submit(source_id, timestamp, image)
            if timestamp >= next_preview:
                jpeg = preview_jpeg(cv2, image)
                if jpeg:
                    overlay = tracking.overlay
                    groups = overlay[2] if overlay and overlay[0] == source_id and timestamp - overlay[1] < 0.15 else []
                    emit({
                        "id": source_id,
                        "preview": {"jpeg": jpeg, "groups": groups},
                    })
                # Keep a phase on typical 24/30 FPS sources without starving high-rate cameras.
                preview_interval = 1.0 / 30
                next_preview = max(next_preview + preview_interval, timestamp - preview_interval * 0.1) if next_preview else timestamp
    finally:
        camera.release()
        tracking.clear()


class MediaLinkError(Exception):
    pass


def media_url(raw):
    try:
        url = urlsplit(raw)
        if (url.scheme not in ("http", "https") or not url.hostname or url.username is not None
                or url.password is not None or len(raw) > 4096 or any(c.isspace() for c in raw)):
            raise ValueError
        return url
    except ValueError:
        raise MediaLinkError("unsupported") from None


def youtube_video(url):
    host = url.hostname
    parts = url.path.strip("/").split("/")
    if host in ("youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com"):
        if parts == ["watch"]:
            video = parse_qs(url.query).get("v", [""])[0]
        elif len(parts) == 2 and parts[0] in ("shorts", "embed", "live"):
            video = parts[1]
        else:
            video = ""
    elif host == "youtu.be":
        video = parts[0] if len(parts) == 1 else ""
    else:
        return None
    if not re.fullmatch(r"[a-zA-Z0-9_-]{11}", video):
        raise MediaLinkError("unsupported")
    return f"https://www.youtube.com/watch?v={video}"


def social_video(url):
    """Accept individual posts and the platforms' own short share links."""
    host, path = url.hostname, url.path.rstrip("/")
    if host in ("tiktok.com", "www.tiktok.com", "m.tiktok.com", "vm.tiktok.com", "vt.tiktok.com"):
        if ((host in ("vm.tiktok.com", "vt.tiktok.com") and re.fullmatch(r"/\w+", path))
                or re.fullmatch(r"/t/\w+", path)):
            return "tiktok", url.geturl(), False  # yt-dlp has a short-link extractor.
        if match := re.fullmatch(r"/(@[\w.-]+/video|embed(?:/v2)?|share/video)/([0-9]+)", path):
            stem, video = match.groups()
            stem = "embed" if stem.startswith("embed") else stem
            return "tiktok", f"https://www.tiktok.com/{stem}/{video}", False
    elif host in ("instagram.com", "www.instagram.com", "m.instagram.com"):
        if re.fullmatch(r"/share/(?:reel/|p/)?[\w-]+", path):
            return "instagram", url.geturl(), True
        if match := re.fullmatch(r"(?:/[\w.]+)?/(reels?|p|tv)/([\w-]+)", path):
            kind, video = match.groups()
            return "instagram", f"https://www.instagram.com/{'reel' if kind == 'reels' else kind}/{video}/", False
    elif host in ("facebook.com", "www.facebook.com", "m.facebook.com", "web.facebook.com", "fb.watch"):
        if ((host == "fb.watch" and re.fullmatch(r"/[\w-]+", path))
                or re.fullmatch(r"/share/(?:r/|v/)[\w-]+", path)):
            return "facebook", url.geturl(), True
        if re.fullmatch(r"/(?:reel|[\w.]+/videos(?:/[^/]+)?)/[0-9]+", path):
            return "facebook", f"https://www.facebook.com{path}", False
        if path in ("/watch", "/video.php", "/video/video.php"):
            video = parse_qs(url.query).get("v", [""])[0]
            if re.fullmatch(r"[0-9]+", video):
                return "facebook", f"https://www.facebook.com/watch/?v={video}", False
    elif host in ("x.com", "www.x.com", "m.x.com", "twitter.com", "www.twitter.com", "mobile.twitter.com", "m.twitter.com"):
        if re.fullmatch(r"/(?:i/web/status|[\w]+/status|statuses)/[0-9]+(?:/video/[0-9]+)?", path):
            return "x", f"https://x.com{path}", False
    else:
        return None
    raise MediaLinkError("unsupported")


def expand_social_share(platform, url):
    request = Request(url, headers={"User-Agent": "Mozilla/5.0", "Range": "bytes=0-0"})
    try:
        with urlopen(request, timeout=8) as response:
            result = social_video(media_url(response.geturl()))
    except (OSError, MediaLinkError):
        raise MediaLinkError("unavailable") from None
    if not result or result[0] != platform or result[2]:
        raise MediaLinkError("unavailable")
    return result[1]


def media_extension(data):
    # Sniff actual containers, including CDN links without file extensions.
    # Web pages and stream manifests are never passed to the local decoder.
    if data.startswith((b"GIF87a", b"GIF89a")):
        return "gif"
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png"
    if data.startswith(b"\xff\xd8\xff"):
        return "jpg"
    if data.startswith(b"BM"):
        return "bmp"
    if data.startswith(b"RIFF"):
        return {b"WEBP": "webp", b"AVI ": "avi"}.get(data[8:12])
    if data.startswith(b"\x1a\x45\xdf\xa3"):
        return "webm"
    if data[4:8] in (b"ftyp", b"moov", b"mdat", b"wide"):
        return "mp4"
    return None


def direct_media(url, folder):
    request = Request(url, headers={"User-Agent": "Aurora-Viewer Webcam Tracking"})
    with urlopen(request, timeout=8) as response:
        media_url(response.geturl())
        size = response.headers.get("Content-Length")
        if size and size.isdecimal() and int(size) > MEDIA_LIMIT:
            raise MediaLinkError("too_large")
        header = response.read(64)
        extension = media_extension(header)
        if extension is None:
            raise MediaLinkError("unsupported")
        path = folder / f"media.{extension}"
        with path.open("wb") as output:
            output.write(header)
            remaining = MEDIA_LIMIT - len(header)
            while chunk := response.read(128 * 1024):
                remaining -= len(chunk)
                if remaining < 0:
                    raise MediaLinkError("too_large")
                output.write(chunk)
    return path


def platform_media(platform, url, folder):
    try:
        import yt_dlp
    except ImportError:
        raise MediaLinkError("dependencies") from None

    class Logger:
        def debug(self, _message):
            pass

        def warning(self, _message):
            pass

        def error(self, _message):
            pass

    blocked = None

    def check(info, *, incomplete):
        nonlocal blocked
        if info.get("is_live") or info.get("live_status") in ("is_live", "is_upcoming"):
            blocked = "unavailable"
        return blocked

    runtime = os.environ.get("AURORA_WEBCAM_DENO")
    has_deno = shutil.which(runtime or "deno") is not None
    can_merge = shutil.which("ffmpeg") is not None
    if platform == "youtube":
        formats = (
            "b[ext=mp4][acodec^=mp4a][height<=480][protocol=https]",
            "bv[ext=mp4][height<=480][protocol=https]+ba[ext=m4a][protocol=https]" if can_merge else None,
            "b[ext=mp4][height<=720][protocol=https]",
            "bv[height<=720][protocol=https]+ba[protocol=https]" if can_merge else None,
        )
    else:
        # Let muxed HLS and other available audio codecs outrank video only.
        formats = (
            "b[ext=mp4][acodec^=mp4a][protocol^=http]",
            "b[ext=mp4][protocol^=http]",
            "b[ext=mp4]",
            "bv[ext=mp4]+ba" if can_merge else None,
            "bv[ext=mp4]",
        )

    def limit_download(status):
        nonlocal blocked
        if status.get("downloaded_bytes", 0) > MEDIA_LIMIT:
            blocked = "too_large"
            raise MediaLinkError(blocked)

    # Prefer ready-to-play MP4 with audio. Fall back to merging separate
    # MP4/M4A tracks when the player only offers those formats.
    options = {
        "format": "/".join(part for part in formats if part),
        "merge_output_format": "mp4",
        "outtmpl": str(folder / "media.%(ext)s"),
        "noplaylist": True,
        "playlist_items": "1",
        "max_filesize": MEDIA_LIMIT,
        "socket_timeout": 8,
        "retries": 1,
        "fragment_retries": 1,
        "quiet": True,
        "noprogress": True,
        "cachedir": False,
        "logger": Logger(),
        "match_filter": check,
        "progress_hooks": [limit_download],
    }
    if platform == "youtube":
        options["js_runtimes"] = {"deno": {"path": runtime} if runtime else {}}
    else:
        options["format_sort"] = ["res:480", "fps:30"]
    try:
        with yt_dlp.YoutubeDL(options) as resolver:
            result = resolver.extract_info(url, download=True)
    except yt_dlp.utils.DownloadError:
        runtime_missing = platform == "youtube" and not (has_deno and can_merge)
        raise MediaLinkError(blocked or ("dependencies" if runtime_missing else "unavailable")) from None
    path = folder / "media.mp4"
    if not path.is_file():
        info = next(iter(result.get("entries", ())), None) if result and "entries" in result else result
        files = info.get("requested_downloads", ()) if info else ()
        too_large = any((file.get("filesize") or file.get("filesize_approx") or 0) > MEDIA_LIMIT for file in files)
        raise MediaLinkError(blocked or ("too_large" if too_large else "unavailable"))
    if path.stat().st_size > MEDIA_LIMIT:
        raise MediaLinkError("too_large")
    return path


def download_media(raw, folder):
    url = media_url(raw)
    youtube = youtube_video(url)
    if youtube:
        return platform_media("youtube", youtube, folder)
    if social := social_video(url):
        platform, canonical, share = social
        if share:
            canonical = expand_social_share(platform, canonical)
        return platform_media(platform, canonical, folder)
    return direct_media(raw, folder)


def resolve_media(commands, raw, folder):
    script = globals().get("_aurora_script") or Path(__file__).read_bytes()
    hide_window = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
    # A dedicated downloader can be canceled even during DNS, extraction or
    # a blocking network read without interrupting the pose models.
    with subprocess.Popen(
        [sys.executable, "-I", "-u", "-c", SCRIPT_BOOT, "--download-media", raw, str(folder)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, **hide_window,
    ) as process:
        try:
            process.stdin.write(f"{len(script)}\n".encode("ascii") + script)
            process.stdin.close()
            deadline = time.monotonic() + 150
            while process.poll() is None:
                try:
                    return None, commands.get(timeout=0.08)
                except Empty:
                    if time.monotonic() >= deadline:
                        raise MediaLinkError("download")
            packet = json.loads(process.stdout.read(8192))
            if "error" in packet:
                raise MediaLinkError(packet["error"])
            path = folder / Path(packet["file"]).name
            if not path.is_file():
                raise MediaLinkError("download")
            return path, None
        except (OSError, ValueError, KeyError):
            raise MediaLinkError("download") from None
        finally:
            if process.poll() is None:
                process.kill()


def worker():
    import cv2
    cv2.setNumThreads(1)

    output_lock = Lock()

    def emit(sample):
        line = json.dumps(sample, allow_nan=False, separators=(",", ":"))
        with output_lock:
            print(line, flush=True)

    commands = Queue()
    tracking = Tracking()

    def infer():
        try:
            tracking.run(cv2, emit)
        except Exception as error:
            emit({"fatal": str(error)})
            commands.put(None)

    Thread(target=read_commands, args=(commands,), daemon=True).start()
    inference = Thread(target=infer, name="landmarks")
    inference.start()
    folder = None
    cached = None
    try:
        source = commands.get()
        while source is not None:
            if source.get("stop") or "loop_setting" in source:
                source = commands.get()
                continue
            try:
                if raw := source.get("url"):
                    if not cached or cached[0] != raw:
                        if folder is not None:
                            folder.cleanup()
                        cached = None
                        folder = TemporaryDirectory(prefix="aurora-webcam-", ignore_cleanup_errors=True)
                        path, next_source = resolve_media(commands, raw, Path(folder.name))
                        if path is None:
                            source = next_source
                            continue
                        cached = raw, path
                    path = cached[1]
                    emit({"id": source["id"], "link_media": str(path)})
                    kind = "image" if path.suffix in (".gif", ".png", ".jpg", ".bmp", ".webp") else "video"
                    source[kind] = str(path)
                source = playback(cv2, commands, tracking, emit, source)
            except MediaLinkError as error:
                emit({"id": source["id"], "link_error": str(error)})
                source = commands.get()
            except (ImportError, RuntimeError, OSError, cv2.error) as error:
                emit({"id": source["id"], "error": str(error)})
                source = commands.get()
    finally:
        tracking.close()
        inference.join()
        if folder is not None:
            folder.cleanup()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--list-cameras", action="store_true")
    parser.add_argument("--download-media", nargs=2, metavar=("URL", "DIRECTORY"))
    options = parser.parse_args()
    try:
        if options.list_cameras:
            list_cameras()
        elif options.download_media:
            raw, folder = options.download_media
            try:
                path = download_media(raw, Path(folder))
                print(json.dumps({"file": path.name}))
            except MediaLinkError as error:
                print(json.dumps({"error": str(error)}))
            except (OSError, ValueError):
                print(json.dumps({"error": "download"}))
        else:
            worker()
    except (ImportError, RuntimeError) as error:
        sys.exit(str(error))
