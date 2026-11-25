"""Summarise a VIctory run log (stderr): per-minute world fps and frame split, counts of problems.

Usage: python tools/perf/analyze_run.py run.log
"""
import re
import statistics
import sys
from datetime import datetime

FPS = re.compile(
    r"^(\S+)Z\s+INFO victory_gpu::host: ([\d.]+) fps, ([\d.]+) ms/frame \(([\d.]+) preparing: copies ([\d.]+) ms ([\d.]+) MB, "
    r"textures ([\d.]+) ms ([\d.]+) MB; ([\d.]+) on the GPU; ([\d.]+) copying back ([\d.]+) MB\), ([\d.]+) draws/frame, "
    r"dropped ([\d.]+) without a host pipeline \((\d+) being made\) and ([\d.]+) indirect"
)


def ts(s):
    return datetime.fromisoformat(s[:26])


def main(path):
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    t0 = ts(lines[0].split("Z")[0])
    rows = []
    counts = {"untranslated": set(), "refused": set(), "errors": 0, "slow submissions": 0, "device": 0}
    for line in lines:
        m = FPS.match(line)
        if m:
            g = m.groups()
            rows.append(((ts(g[0]) - t0).total_seconds(), *map(float, g[1:])))
            continue
        if "shader does not translate" in line:
            h = re.search(r"hash=(\w+)", line)
            counts["untranslated"].add(h.group(1) if h else line)
        elif "host pipeline refused" in line:
            counts["refused"].add(re.search(r"shaders=\[(.*?)\]", line).group(1))
        elif " ERROR " in line:
            counts["errors"] += 1
        elif "slow host submission" in line:
            counts["slow submissions"] += 1
        if re.search(r"DEVICE_(HUNG|REMOVED)|device removed|unhandled exception", line):
            counts["device"] += 1
    last = (ts(lines[-1].split("Z")[0]) - t0).total_seconds()
    print(f"log span {last/60:.1f} min, {len(rows)} fps lines")
    world = [r for r in rows if r[11] > 500]
    if world:
        print(f"first world frame (draws>500) at {world[0][0]/60:.1f} min")
    print("min | fps med | ms med | prep | copies | textures | gpu | back | draws | dropped (making) | indirect")
    by_min = {}
    for r in world:
        by_min.setdefault(int(r[0] // 60), []).append(r)
    for minute, rs in sorted(by_min.items()):
        med = lambda i: statistics.median(x[i] for x in rs)
        print(f"{minute:3d} | {med(1):5.2f} | {med(2):7.1f} | {med(3):6.1f} | {med(4):5.1f} | {med(6):6.1f} | {med(8):5.1f} | "
              f"{med(9):5.1f} | {med(11):5.0f} | {med(12):5.0f} ({med(13):.0f}) | {med(14):.0f}")
    tail = [r for r in world if r[0] >= last - 180]
    if tail:
        med = lambda i: statistics.median(x[i] for x in tail)
        best = max(r[1] for r in tail)
        print(f"last 3 min: fps median {med(1):.2f} (best {best:.2f}), {med(2):.0f} ms/frame = prep {med(3):.0f} "
              f"(copies {med(4):.0f}, textures {med(6):.0f}) + gpu {med(8):.0f} + back {med(9):.0f}; draws {med(11):.0f}, "
              f"dropped {med(12):.0f}, making {med(13):.0f}")
    print(f"untranslated shaders {len(counts['untranslated'])}, refused pipelines {len(counts['refused'])} "
          f"{sorted(counts['refused'])}, ERROR lines {counts['errors']}, slow submissions {counts['slow submissions']}, "
          f"device/crash lines {counts['device']}")


if __name__ == "__main__":
    main(sys.argv[1])
