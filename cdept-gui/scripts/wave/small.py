"""Small inputs for the waveform smoke test (CI and by hand). Usage: python3 -I small.py OUTDIR

small.raw: 2000 points, v(a) a sine, v(b) a ramp, i(vdd) a current.
family/pNNN/out.raw: 12 run points of one testbench, each on its own x grid; p005 is
garbage, p007 has no raw file, p009 is cut short.
"""
import math, os, struct, sys

out = sys.argv[1]
os.makedirs(out, exist_ok=True)


def raw(path, xs, cols):
    names = list(cols)
    h = "Title: small\nDate: today\nPlotname: Transient Analysis\nFlags: real\n"
    h += "No. Variables: %d\nNo. Points: %d\nVariables:\n\t0\ttime\ttime\n" % (len(names) + 1, len(xs))
    for i, n in enumerate(names):
        kind = "current" if n.startswith("i(") else "voltage"
        h += "\t%d\t%s\t%s\n" % (i + 1, n, kind)
    with open(path, "wb") as fh:
        fh.write((h + "Binary:\n").encode())
        for k, x in enumerate(xs):
            fh.write(struct.pack("<d", x))
            for n in names:
                fh.write(struct.pack("<d", cols[n](k, x)))


n = 2000
raw(os.path.join(out, "small.raw"), [i * 1e-9 for i in range(n)], {
    "v(a)": lambda k, x: math.sin(x * 2e7),
    "v(b)": lambda k, x: k / n,
    "i(vdd)": lambda k, x: -1e-3 * (1 + 0.5 * math.sin(x * 1e7)),
})
fam = os.path.join(out, "family")
for p in range(12):
    d = os.path.join(fam, "p%03d" % p)
    os.makedirs(d, exist_ok=True)
    if p == 7:
        continue
    if p == 5:
        with open(os.path.join(d, "out.raw"), "wb") as fh:
            fh.write(b"not a raw file\n" * 100)
        continue
    m = 500 + 37 * p
    xs = [i * 2e-6 / (m - 1) for i in range(m)]
    raw(os.path.join(d, "out.raw"), xs, {
        "v(out)": lambda k, x, p=p: (1 + 0.05 * p) * math.tanh(x * 2e6 - 2),
        "v(in)": lambda k, x: 1.0 if x > 0.5e-6 else 0.0,
    })
    if p == 9:
        f = os.path.join(d, "out.raw")
        os.truncate(f, os.path.getsize(f) // 2)
with open(os.path.join(out, "table.csv"), "w") as fh:
    fh.write("time,vout,iload\n")
    for i in range(200):
        fh.write("%g,%g,%g\n" % (i * 1e-9, math.sin(i * 0.1), 1e-3 * i))
os.makedirs(os.path.join(out, "proc"), exist_ok=True)
with open(os.path.join(out, "proc", "p.raw"), "wb") as fh, open(os.path.join(out, "small.raw"), "rb") as src:
    fh.write(src.read())
# hostile names: unicode, a very long one, signal names too
odd = os.path.join(out, "odd")
os.makedirs(odd, exist_ok=True)
long_sig = "v(" + "x" * 300 + ".out)"
raw(os.path.join(odd, "fam_ünïcødé_日本語.raw"), [i * 1e-9 for i in range(300)], {
    "v(ñodo_μ)": lambda k, x: math.sin(k * 0.1),
    long_sig: lambda k, x: math.cos(k * 0.1),
    "v(x1.x2.deep)": lambda k, x: k * 1e-3,
})
raw(os.path.join(odd, ("L" * 200) + ".raw"), [i * 1e-9 for i in range(300)], {
    "v(ñodo_μ)": lambda k, x: 0.5 * math.sin(k * 0.1),
})
