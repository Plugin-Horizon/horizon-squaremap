#!/usr/bin/env python3
"""Compares two squaremap tile folders (`tiles/<world>/<zoom>/`) pixel by pixel.

Usage: compare_tiles.py REFERENCE_DIR PORT_DIR [--diff OUT.png]

Only pixels both renders drew (alpha > 0 in both) are compared. Prints, per tile and in
total: pixels compared, exactly equal, within 8 per channel, and the mean channel difference.
"""
import os
import sys

import numpy as np
from PIL import Image


def load(path):
    return np.asarray(Image.open(path).convert("RGBA"), dtype=np.int16)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    diff_out = sys.argv[sys.argv.index("--diff") + 1] if "--diff" in sys.argv else None
    if diff_out in args:
        args.remove(diff_out)
    ref_dir, port_dir = args
    total = equal = close = 0
    channel_sum = 0.0
    only_ref = only_port = 0
    worst = None
    for name in sorted(os.listdir(ref_dir)):
        if not name.endswith(".png") or not os.path.exists(os.path.join(port_dir, name)):
            continue
        a, b = load(os.path.join(ref_dir, name)), load(os.path.join(port_dir, name))
        drawn_a, drawn_b = a[..., 3] > 0, b[..., 3] > 0
        both = drawn_a & drawn_b
        only_ref += int((drawn_a & ~drawn_b).sum())
        only_port += int((drawn_b & ~drawn_a).sum())
        n = int(both.sum())
        if n == 0:
            continue
        d = np.abs(a[both][:, :3] - b[both][:, :3])
        eq = int((d.max(axis=1) == 0).sum())
        cl = int((d.max(axis=1) <= 8).sum())
        total, equal, close = total + n, equal + eq, close + cl
        channel_sum += float(d.mean()) * n
        print(f"{name}: {n} px, equal {eq / n:.2%}, within 8 {cl / n:.2%}, mean diff {d.mean():.2f}")
        if diff_out and (worst is None or n - eq > worst[0]):
            mask = np.zeros(a.shape[:2], dtype=np.uint8)
            mask[both] = np.minimum(np.abs(a - b)[both][:, :3].max(axis=1) * 8, 255)
            worst = (n - eq, mask)
    if total == 0:
        print("no pixel drawn in both")
        return 1
    print(f"TOTAL: {total} px compared, equal {equal / total:.2%}, within 8 {close / total:.2%}, "
          f"mean channel diff {channel_sum / total:.3f}; drawn only in reference {only_ref}, only in port {only_port}")
    if diff_out and worst:
        Image.fromarray(worst[1], "L").save(diff_out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
