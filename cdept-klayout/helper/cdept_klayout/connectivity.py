# cdept-klayout: the connectivity of a layout, from an LVS database, as JSON.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Batch script for CDePT's net comparison (docs/LINKING.md §8); the work is in
`cdept_klayout.l2n`, which describes the JSON.

  klayout -b -r connectivity.py -rd lvsdb=<file.lvsdb> -rd input=<stream> \
          -rd top=<cell> -rd out=<file.json> [-rd pins=<pins.json>]

`pins`: the layout's pins, [{name, point, layer}] (docs/LINKING.md §9).
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from cdept_klayout import l2n  # noqa: E402


def main():
    g = globals()
    lvsdb, stream, top, out = (g.get(k) for k in ("lvsdb", "input", "top", "out"))
    if not (lvsdb and stream and top and out):
        raise ValueError("usage: klayout -b -r connectivity.py -rd lvsdb=<file> "
                         "-rd input=<stream> -rd top=<cell> -rd out=<file.json>")
    pins = None
    if g.get("pins"):
        with open(g["pins"]) as f:
            pins = json.load(f)
    with open(out, "w") as f:
        json.dump(l2n.Connectivity(lvsdb, top).describe(stream, pins), f, indent=1)


main()
