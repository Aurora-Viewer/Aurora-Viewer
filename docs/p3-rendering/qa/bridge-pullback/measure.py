"""Measure fixed receiver points visible at every position in the pullback."""
from bisect import bisect_right
from statistics import fmean, median


def bridge_footprint(y, half_width):
    # This is the near edge of the 12-segment rendered ribbon in local XY.
    nodes = [index * .004 for index in range(-6, 7)]
    left = nodes[bisect_right(nodes, y) - 1]
    right = left + .004
    t = (y - left) / .004
    def centre(at):
        return .010 + .11 * at + 13 * at**2
    return (1 - t) * centre(left) + t * centre(right) - half_width


def measure(capture):
    rows, columns = capture["ny"], capture["nx"]
    dx = capture["dx"]
    x = [capture["x0"] + i * dx for i in range(columns)]
    truth = capture["truth256"]
    frames = capture["frames"]
    common = [all(visibility) for visibility in zip(*(frame["visible"] for frame in frames))]
    gap_rows = []
    core = []
    for j in range(rows):
        start = j * columns
        y = capture["y0"] + j * capture["dy"]
        rim_edge = bridge_footprint(y, capture["rim_width"] / 2)
        cast_edge = max(x[i] for i in range(columns) if truth[start + i] < 256)
        width = rim_edge - cast_edge
        # Exclude the ends of the gap so ordinary edge smoothing is not scored as a filled gap.
        gap_rows.append([start + i for i in range(columns) if common[start + i]
                         and truth[start + i] == 256 and .15 < (x[i] - cast_edge) / width < .85])
        core.extend(start + i for i in range(columns) if common[start + i] and truth[start + i] == 0)
    gap = [i for row in gap_rows for i in row]
    if min(len(gap), len(core)) < 150:
        raise ValueError("Insufficient ground-truthed receiver samples shared across poses")
    output = []
    channels = (("map", "map", 65535), ("raw_contact", "bend_raw", 255),
                ("filtered_contact", "bend", 255), ("combined", "combined", 65535))
    for frame in frames:
        entry = {"distance_m": round(frame["eye"][2], 3)}
        for label, name, maximum in channels:
            occlusion = [1 - value / maximum for value in frame[name]]
            dark_width = [sum(occlusion[i] > .10 for i in row) * dx * 1000 for row in gap_rows]
            entry[label] = {
                "false_gap_occlusion_pct": round(100 * fmean(occlusion[i] for i in gap), 3),
                "median_width_above_10pct_mm": round(median(dark_width), 3),
                "true_core_occlusion_pct": round(100 * fmean(occlusion[i] for i in core), 3),
            }
        output.append(entry)
    return {"adapter": capture["adapter"], "gap_samples": len(gap), "core_samples": len(core), "poses": output}


def verify(report):
    poses = report["poses"]
    resolved = [p for p in poses if p["distance_m"] <= .49]
    errors = []
    # Raw hit accuracy and composed display quality are independent. The
    # farthest poses allow more reconstruction spill as the rim goes subpixel.
    for subset, channel, limit in [(poses, "raw_contact", 10), (resolved, "combined", 20),
                                    (poses, "combined", 25)]:
        worst = max(subset, key=lambda p: p[channel]["false_gap_occlusion_pct"])
        value = worst[channel]["false_gap_occlusion_pct"]
        if value > limit:
            errors.append(f"{channel} false gap occlusion {value:.1f}% at {worst['distance_m']} m exceeds {limit}%")
    # An all-lit contact mask would pass the false-shadow tests. Exercise its
    # actual occluded interior independently of what the directional map draws.
    weakest = min(p["raw_contact"]["true_core_occlusion_pct"] for p in resolved)
    if weakest < 40:
        errors.append(f"Resolved raw contact-shadow core fell to {weakest:.1f}% (minimum 40%)")
    close = poses[0]["combined"]["true_core_occlusion_pct"]
    if close < 65:
        errors.append(f"The true close-up shadow core reached only {close:.1f}% occlusion (minimum 65%)")
    if errors:
        raise SystemExit("\n".join(errors))
