"""Inputs for the waveform tabs' analyses test (run.sh analyses): AC, eye, sweeps that go
back, hostile complex files, a family of mixed analyses. Usage: python3 -I analyses.py OUTDIR

eye.raw        NRZ data at 1 Gb/s through a first-order low pass (tau 0.25 ns), 64
               samples a bit, 3000 bits, edges with 3 % jitter, 15 mV of noise: v(rx);
               v(clk) a 1 GHz clock.
hyst.raw       a DC sweep 0 -> 1.8 -> 0 V (there and back): v(out) = 2 v-sweep + 0.1 (up)
               and 2 v-sweep - 0.1 (down).
param.raw      a parametric DC sweep: 4 sweeps of 1.8 -> 0 V, v(out) = v-sweep * (k + 1).
ac_*.raw       AC files: ac_delay (a low pass with a 2 ns delay: the phase wraps), ac_zero
               (every value 0: dB of 0), ac_naninf (NaN and Inf in it), ac_f0 (frequency
               from 0 Hz, then negative ones), ac_cut (cut in the middle of a value),
               ac_lies (says 10^12 points), ac_half (a pair without its imaginary part),
               ac_unicode (names ä and 出力).
mixed/pN/out.raw  a family whose points hold different analyses (tran, ac, dc, op) and a
               broken one.
"""
import cmath, math, os, random, struct, sys

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
random.seed(4)


def header(name, flags, x, xtype, cols, points, mode="Binary"):
    h = "Title: analyses test\nDate: today\nPlotname: %s\nFlags: %s\n" % (name, flags)
    h += "No. Variables: %d\nNo. Points: %s\nVariables:\n\t0\t%s\t%s\n" % (len(cols) + 1, points, x, xtype)
    for i, (n, kind) in enumerate(cols):
        h += "\t%d\t%s\t%s\n" % (i + 1, n, kind)
    return h + mode + ":\n"


def real_raw(path, plot, x, xtype, cols, rows):
    with open(path, "wb") as fh:
        fh.write(header(plot, "real", x, xtype, cols, len(rows)).encode())
        for r in rows:
            fh.write(struct.pack("<%dd" % len(r), *r))


def ac_raw(path, cols, freqs, fn, points_text=None, ascii=False):
    """cols: names; fn(f) -> list of complex values (one per column)."""
    with open(path, "wb") as fh:
        h = header("AC Analysis", "complex", "frequency", "frequency grid=3",
                   [(c, "voltage") for c in cols], points_text or len(freqs),
                   "Values" if ascii else "Binary")
        fh.write(h.encode())
        for k, f in enumerate(freqs):
            vals = [complex(f, 0)] + fn(f)
            if ascii:
                for i, v in enumerate(vals):
                    fh.write(((" %d\t" % k) if i == 0 else "\t").encode())
                    fh.write(("%.15e,%.15e\n" % (v.real, v.imag)).encode())
                fh.write(b"\n")
            else:
                for v in vals:
                    fh.write(struct.pack("<2d", v.real, v.imag))


# --- eye ---
bits, spb, ui = 3000, 64, 1e-9
dt = ui / spb
level, rows = 0.0, []
data = [random.randint(0, 1) for _ in range(bits)]
tau = 0.25e-9
a = math.exp(-dt / tau)
for b in range(bits):
    # each edge a little early or late (jitter of 3 % of a bit)
    shift = int(round(random.gauss(0, 0.03) * spb))
    for s in range(spb):
        t = (b * spb + s) * dt
        target = data[b] if s >= shift or b == 0 else data[b - 1]
        level = level * a + (1 - a) * target
        rows.append((t, level + random.gauss(0, 0.015), 1.0 if (t % ui) < ui / 2 else 0.0))
real_raw(os.path.join(out, "eye.raw"), "Transient Analysis", "time", "time",
         [("v(rx)", "voltage"), ("v(clk)", "voltage")], rows)

# --- sweeps that go back ---
up = [i * 0.01 for i in range(181)]
rows = [(v, 2 * v + 0.1) for v in up] + [(v, 2 * v - 0.1) for v in reversed(up[:-1])]
real_raw(os.path.join(out, "hyst.raw"), "DC transfer characteristic", "v(v-sweep)", "voltage",
         [("v(out)", "voltage")], rows)
rows = []
for k in range(4):
    rows += [(1.8 - i * 0.05, (1.8 - i * 0.05) * (k + 1)) for i in range(37)]
real_raw(os.path.join(out, "param.raw"), "DC transfer characteristic", "v(v-sweep)", "voltage",
         [("v(out)", "voltage")], rows)

# --- AC ---
freqs = [10 ** (i / 20) for i in range(201)]  # 1 Hz .. 10 GHz
lp = lambda f: 1 / (1 + 1j * f / 1e6)
ac_raw(os.path.join(out, "ac_delay.raw"), ["v(out)"], freqs,
       lambda f: [lp(f) * cmath.exp(-2j * math.pi * f * 2e-9)])
ac_raw(os.path.join(out, "ac_delay_ascii.raw"), ["v(out)"], freqs,
       lambda f: [lp(f) * cmath.exp(-2j * math.pi * f * 2e-9)], ascii=True)
ac_raw(os.path.join(out, "ac_zero.raw"), ["v(out)"], freqs, lambda f: [0j])
ac_raw(os.path.join(out, "ac_naninf.raw"), ["v(out)"], freqs,
       lambda f: [complex(float("nan"), 0) if 50 < f < 100 else (complex(float("inf"), 1) if 1e4 < f < 2e4 else lp(f))])
ac_raw(os.path.join(out, "ac_f0.raw"), ["v(out)"], [0.0, 1.0, 10.0, -5.0, 100.0, 1000.0],
       lambda f: [lp(abs(f))])
ac_raw(os.path.join(out, "ac_unicode.raw"), ["v(ausgang_ä)", "v(出力)"], freqs,
       lambda f: [lp(f), 2 * lp(f)])
with open(os.path.join(out, "ac_delay_ascii.raw"), "rb") as fh:
    good = fh.read()
with open(os.path.join(out, "ac_cut.raw"), "wb") as fh:
    fh.write(good[: len(good) // 2 + 7])
with open(os.path.join(out, "ac_lies.raw"), "wb") as fh:
    fh.write(good.replace(b"No. Points: 201", b"No. Points: 1000000000000", 1))
with open(os.path.join(out, "ac_half.raw"), "wb") as fh:
    i = good.index(b"\t", good.index(b"Values:") + 12)
    j = good.index(b",", i)
    k = good.index(b"\n", j)
    fh.write(good[:j] + good[k:])
with open(os.path.join(out, "ac_bin.raw"), "wb") as fh, open(os.path.join(out, "ac_delay.raw"), "rb") as src:
    b = src.read()
    fh.write(b[: len(b) - 5])  # cut inside a value

# --- a family of mixed analyses ---
mixed = os.path.join(out, "mixed")
for p in range(5):
    os.makedirs(os.path.join(mixed, "p%d" % p), exist_ok=True)
real_raw(os.path.join(mixed, "p0", "out.raw"), "Transient Analysis", "time", "time",
         [("v(out)", "voltage")], [(i * 1e-9, math.sin(i * 0.1)) for i in range(500)])
ac_raw(os.path.join(mixed, "p1", "out.raw"), ["v(out)"], freqs[:120], lambda f: [lp(f)])
real_raw(os.path.join(mixed, "p2", "out.raw"), "DC transfer characteristic", "v(v-sweep)", "voltage",
         [("v(out)", "voltage")], [(1.8 - i * 0.1, 2 * (1.8 - i * 0.1)) for i in range(19)])
real_raw(os.path.join(mixed, "p3", "out.raw"), "Operating Point", "v(in)", "voltage",
         [("v(out)", "voltage")], [(0.9, 0.45)])
with open(os.path.join(mixed, "p4", "out.raw"), "wb") as fh:
    fh.write(good[:300])
