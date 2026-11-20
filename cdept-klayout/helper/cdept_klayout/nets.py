# cdept-klayout: nets over the layout: highlights, flylines, probing.
# SPDX-License-Identifier: GPL-3.0-or-later
"""The editor's side of CDePT's net comparison (docs/LINKING.md §8).

CDePT extracts the layout with the PDK's LVS deck (a KLayout of its own, in batch),
compares the nets with the schematic's, and then asks the editor to
- load the LVS database (`nets_load`): the nets, numbered as `l2n.Connectivity` does;
- draw the shapes of some nets (`nets_highlight`) and flylines between the pieces of
  open nets (`nets_flylines`), as markers: not part of the layout, never saved;
- say which net is under a point (`nets_probe`).

The database is as old as the last extraction: edits made since are not in it.
"""

import pya

from . import l2n

HIGHLIGHT = "#ffd400"
FLYLINE = "#ff40ff"
# Polygons drawn for a highlight at most (the rest are left out).
MAX_POLYGONS = 20000
MARGIN = 0.5


class Group:
    """Markers that come and go together."""

    def __init__(self):
        self.view = None
        self.markers = []

    def clear(self):
        for marker in self.markers:
            try:
                marker._destroy()
            except Exception:
                pass
        self.markers = []

    def add(self, view, shape, color, width=1, dither=-1, halo=0):
        if view is not self.view:
            self.markers = []
            self.view = view
        marker = pya.Marker(view)
        marker.set(shape)
        marker.color = color
        marker.frame_color = color
        marker.line_width = width
        marker.vertex_size = 0
        marker.dither_pattern = dither
        marker.halo = halo
        self.markers.append(marker)


def _color(text, default):
    try:
        return int(str(text or default).lstrip("#"), 16)
    except ValueError:
        return int(default.lstrip("#"), 16)


class Nets:
    def __init__(self):
        self.data = None
        self.source = None
        self.highlight = Group()
        self.flylines = Group()

    def load(self, p):
        lvsdb = p.get("lvsdb")
        if not lvsdb:
            raise ValueError("nets_load needs lvsdb")
        self.data = l2n.Connectivity(lvsdb, p.get("top"))
        self.source = lvsdb
        return {"nets": len(self.data.nets), "top": self.data.circuit.name}

    def _require(self):
        if self.data is None:
            raise RuntimeError("no connectivity is loaded (nets_load)")
        return self.data

    def show(self, view, p):
        """Draw the shapes of the nets `nets` (ids); empty: clear."""
        self.highlight.clear()
        ids = p.get("nets") or []
        if not ids:
            return {"markers": 0}
        data = self._require()
        color = _color(p.get("color"), HIGHLIGHT)
        box = pya.DBox()
        count = 0
        for i in ids:
            if not isinstance(i, int) or not 0 <= i < len(data.nets):
                continue
            for _, region in data.shapes(data.nets[i]):
                for polygon in region.merged().each():
                    if count >= MAX_POLYGONS:
                        break
                    shape = polygon.to_dtype(data.dbu)
                    self.highlight.add(view, shape, color, width=2, dither=5, halo=1)
                    box += shape.bbox()
                    count += 1
        if p.get("zoom") and not box.empty():
            margin = max(MARGIN, 0.2 * max(box.width(), box.height()))
            view.zoom_box(box.enlarged(margin, margin))
        return {"markers": count}

    def lines(self, view, p):
        """Draw flylines: `lines` [[x1, y1, x2, y2], …] (µm); empty: clear."""
        self.flylines.clear()
        color = _color(p.get("color"), FLYLINE)
        for line in p.get("lines") or []:
            x1, y1, x2, y2 = line[:4]
            self.flylines.add(view, pya.DEdge(x1, y1, x2, y2), color, width=2, halo=1)
        return {"markers": len(self.flylines.markers)}

    def probe(self, editor, p):
        """The net under `point` (µm) or `pixel` ([x, y] in the frame)."""
        data = self._require()
        if "pixel" in p:
            x, y = p["pixel"]
            view = editor.require()
            at = view.viewport_trans().inverted() * pya.DPoint(x, editor.height - 1 - y)
            point = (at.x, at.y)
        else:
            point = tuple(p["point"])
        net = data.net_at(point)
        name = l2n._name(data.nets[net]) if net is not None else ""
        return {"net": net, "name": name, "point": [round(point[0], 6), round(point[1], 6)]}

    def clear(self):
        self.highlight.clear()
        self.flylines.clear()
