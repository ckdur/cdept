"""A very large binary ngspice raw file for the waveform tabs (run.sh huge). Usage:
python3 -I huge.py FILE POINTS [SIGNALS]

time = i * 1 ns; v(sig) a 1 MHz sine with a slow 3 kHz ripple; v(spike) zero but for one
sample of 1.0 at POINTS * 0.618 (a narrow spike that must stay visible at any zoom);
more signals v(nK) are scaled sines. Written in blocks with numpy (about 1 GB a few
seconds); without numpy it says so and fails.
"""
import sys

try:
    import numpy as np
except ImportError:
    sys.exit("huge.py needs numpy")

path, points = sys.argv[1], int(float(sys.argv[2]))
signals = int(sys.argv[3]) if len(sys.argv) > 3 else 2
names = ["v(sig)", "v(spike)"] + ["v(n%d)" % k for k in range(3, signals + 1)]
names = names[:max(signals, 1)]
spike = int(points * 0.618)
with open(path, "wb") as fh:
    h = "Title: huge\nDate: today\nPlotname: Transient Analysis\nFlags: real\n"
    h += "No. Variables: %d\nNo. Points: %d\nVariables:\n\t0\ttime\ttime\n" % (len(names) + 1, points)
    for i, n in enumerate(names):
        h += "\t%d\t%s\tvoltage\n" % (i + 1, n)
    fh.write((h + "Binary:\n").encode())
    block = 4_000_000
    for start in range(0, points, block):
        n = min(block, points - start)
        i = np.arange(start, start + n, dtype=np.float64)
        t = i * 1e-9
        cols = [t]
        for k, name in enumerate(names):
            if name == "v(spike)":
                c = np.zeros(n)
                if start <= spike < start + n:
                    c[spike - start] = 1.0
            elif name == "v(sig)":
                c = np.sin(2 * np.pi * 1e6 * t) + 0.1 * np.sin(2 * np.pi * 3e3 * t)
            else:
                c = (k + 1) * 0.1 * np.sin(2 * np.pi * 1e5 * (k + 1) * t)
            cols.append(c)
        np.stack(cols, axis=1).astype("<f8").tofile(fh)
print("%s: %d points, %d signals, spike at sample %d (t = %.9g s)" % (path, points, len(names), spike, spike * 1e-9))
