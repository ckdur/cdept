"""Torture inputs for the waveform tabs. Usage: python3 -I gen.py OUTDIR"""
import math, os, struct, sys
out = sys.argv[1]
os.makedirs(out, exist_ok=True)

def header(npts, names, pts_text=None, nvars_text=None, mode="Binary"):
    h = "Title: torture\nDate: today\nPlotname: Transient Analysis\nFlags: real\n"
    h += "No. Variables: %s\n" % (nvars_text or len(names) + 1)
    h += "No. Points: %s\n" % (pts_text or npts)
    h += "Variables:\n\t0\ttime\ttime\n"
    for i, n in enumerate(names):
        h += "\t%d\t%s\tvoltage\n" % (i + 1, n)
    return (h + mode + ":\n").encode()

def binary(path, npts, names, f, dt=1e-9, **kw):
    with open(path, "wb") as fh:
        fh.write(header(npts, names, **kw))
        chunk = 100000
        for start in range(0, npts, chunk):
            vals = []
            for i in range(start, min(npts, start + chunk)):
                vals.append(i * dt)
                vals.extend(f(k, i) for k in range(len(names)))
            fh.write(struct.pack("<%dd" % len(vals), *vals))

N = 10_000_000
SPIKE = 7_654_321
binary(os.path.join(out, "spike10m.raw"), N, ["v(out)", "v(sine)"],
       lambda k, i: (1.0 if i == SPIKE else 0.0) if k == 0 else math.sin(i * 2e-5))
binary(os.path.join(out, "good.raw"), 100000, ["v(a)", "v(b)"], lambda k, i: (k + 1) * (i % 1000) * 1e-3)
binary(os.path.join(out, "stale.raw"), 100000, ["v(a)"], lambda k, i: 1.0)
binary(os.path.join(out, "change.raw"), 100000, ["v(a)"], lambda k, i: 1.0)
binary(os.path.join(out, "loop.raw"), 200000, ["v(a)", "v(b)", "v(c)"], lambda k, i: math.sin(i * 1e-3 * (k + 1)))
# truncated: header promises 100000 points, half the bytes are there
p = os.path.join(out, "truncated.raw")
binary(p, 100000, ["v(a)"], lambda k, i: float(i))
os.truncate(p, os.path.getsize(p) - 800000 - 3)
# lying headers
binary(os.path.join(out, "lying_points.raw"), 1000, ["v(a)"], lambda k, i: float(i), pts_text="99999999999")
binary(os.path.join(out, "lying_vars.raw"), 1000, ["v(a)"], lambda k, i: float(i), nvars_text="1000000")
binary(os.path.join(out, "neg_points.raw"), 1000, ["v(a)"], lambda k, i: float(i), pts_text="-1")
with open(os.path.join(out, "garbage.raw"), "wb") as fh:
    fh.write(os.urandom(1 << 20))
with open(os.path.join(out, "nonl.raw"), "wb") as fh:
    fh.write(b"x" * (5 << 20))
open(os.path.join(out, "empty.raw"), "wb").close()
with open(os.path.join(out, "header_only.raw"), "wb") as fh:
    fh.write(header(10, ["v(a)"]))
# a read-only folder
ro = os.path.join(out, "readonly")
os.makedirs(ro, exist_ok=True)
binary(os.path.join(ro, "ro.raw"), 50000, ["v(a)"], lambda k, i: float(i % 7))
os.chmod(ro, 0o555)
# a non-ASCII path
na = os.path.join(out, "Ωmega été", "波形")
os.makedirs(na, exist_ok=True)
binary(os.path.join(na, "sortie é.raw"), 50000, ["v(é)"], lambda k, i: 2.0)
print("ok")
