"""Large inputs for the waveform tabs (run.sh big). Usage: python3 -I big.py OUTDIR [POINTS]

family/pNNNN/out.raw: POINTS run points (default 600) of 2000 samples each on grids of
their own; 5 are garbage, 5 have no raw file, 5 are cut short.
wide.raw: 300 signals of 20000 samples (for hundreds of panes).
"""
import math, os, random, struct, sys

out = sys.argv[1]
n = int(sys.argv[2]) if len(sys.argv) > 2 else 600
random.seed(7)


def raw(path, xs, names, f):
    h = "Title: big\nDate: today\nPlotname: Transient Analysis\nFlags: real\n"
    h += "No. Variables: %d\nNo. Points: %d\nVariables:\n\t0\ttime\ttime\n" % (len(names) + 1, len(xs))
    for i, nm in enumerate(names):
        h += "\t%d\t%s\tvoltage\n" % (i + 1, nm)
    rows = []
    for k, x in enumerate(xs):
        rows.append(x)
        rows.extend(f(j, k, x) for j in range(len(names)))
    with open(path, "wb") as fh:
        fh.write((h + "Binary:\n").encode())
        fh.write(struct.pack("<%dd" % len(rows), *rows))


fam = os.path.join(out, "family")
bad = set(random.sample(range(n), 15))
garbage, missing, cut = sorted(bad)[:5], sorted(bad)[5:10], sorted(bad)[10:]
for p in range(n):
    d = os.path.join(fam, "p%04d" % p)
    os.makedirs(d, exist_ok=True)
    if p in missing:
        continue
    f = os.path.join(d, "out.raw")
    if p in garbage:
        with open(f, "wb") as fh:
            fh.write(os.urandom(4096))
        continue
    m = 2000 + random.randint(-300, 300)
    t1 = 1e-6 * (1 + random.uniform(-0.05, 0.05))
    xs = [t1 * i / (m - 1) for i in range(m)]
    g = 1 + random.gauss(0, 0.05)
    raw(f, xs, ["v(out)", "v(in)", "i(vdd)"],
        lambda j, k, x, g=g: (g * math.tanh((x - 5e-7) * 1e7), 1.0 if x > 4.5e-7 else 0.0, -1e-3 * g)[j])
    if p in cut:
        os.truncate(f, os.path.getsize(f) * 2 // 3)
m = 20000
raw(os.path.join(out, "wide.raw"), [i * 1e-10 for i in range(m)], ["v(n%03d)" % i for i in range(300)],
    lambda j, k, x: math.sin(k * 0.001 * (1 + j * 0.01)))
print("garbage", garbage, "missing", missing, "cut", cut)
