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
from .live import Live

HIGHLIGHT = "#ffd400"
FLYLINE = "#ff40ff"
# KLayout's built-in line styles (indexes of LayoutView's line style list).
LINE_STYLES = {"solid": 0, "dotted": 1, "dashed": 2, "dash-dotted": 3}
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

    def add(self, view, shape, color, width=1, dither=-1, halo=0, line_style=0):
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
        marker.line_style = line_style
        self.markers.append(marker)


def _color(text, default):
    try:
        return int(str(text or default).lstrip("#"), 16) & 0xffffff
    except ValueError:
        return int(default.lstrip("#"), 16)


def _background(view):
    try:
        text = view.get_config("background-color")
        if text:
            return int(text.lstrip("#"), 16) & 0xffffff
    except Exception:
        pass
    return 0xffffff


def _blend(color, background, opacity):
    """`color` over `background` at `opacity` (markers have no alpha of their own)."""
    if opacity is None or opacity >= 1:
        return color
    a = max(0.0, float(opacity))
    out = 0
    for shift in (16, 8, 0):
        c = (color >> shift) & 0xff
        b = (background >> shift) & 0xff
        out |= int(round(c * a + b * (1 - a))) << shift
    return out


class Nets:
    def __init__(self):
        self.data = None
        self.source = None
        self.highlight = Group()
        self.flylines = Group()
        self.live = Live()
        # How flylines are drawn (nets_flylines' style, kept for later calls).
        self.style = {"color": FLYLINE, "width": 2, "style": "solid", "opacity": 1.0}

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
        live = p.get("live") and self.live.traces
        for i in ids:
            if live:
                polygons = self.live.polygons(i) if isinstance(i, int) else []
                if polygons is not None:
                    for shape in polygons[:MAX_POLYGONS - count]:
                        self.highlight.add(view, shape, color, width=2, dither=5, halo=1)
                        box += shape.bbox()
                        count += 1
                    continue
                i = self.live.extracted_id(i)
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
        """Draw flylines: `lines` [[x1, y1, x2, y2], …] (µm); empty: clear. Style
        (kept for the next calls): `color` "#rrggbb", `width` (pixels), `style`
        (solid, dotted, dashed, dash-dotted), `opacity` (0…1, blended with the
        background: markers have no alpha)."""
        self.flylines.clear()
        for key in ("color", "width", "style", "opacity"):
            if p.get(key) is not None:
                self.style[key] = p[key]
        st = self.style
        color = _blend(_color(st["color"], FLYLINE), _background(view), st.get("opacity"))
        width = max(1, min(16, int(st.get("width") or 1)))
        line_style = LINE_STYLES.get(str(st.get("style")), 0)
        for line in p.get("lines") or []:
            x1, y1, x2, y2 = line[:4]
            self.flylines.add(view, pya.DEdge(x1, y1, x2, y2), color, width=width, halo=0,
                              line_style=line_style)
        return {"markers": len(self.flylines.markers), "color": "#%06x" % color,
                "width": width, "line_style": line_style}

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
        if p.get("live") and self.live.traces:
            found = self.live.probe(editor.doc.layout, editor.doc.top_cell(), point)
            if found is not None:
                net, trace = found
                if net >= 0:
                    return {"net": net, "name": self._live_name(net), "live": True,
                            "point": [round(point[0], 6), round(point[1], 6)]}
                # A net without terminals of the extraction: show it, no id.
                self.highlight.clear()
                for shape in trace.polygons[:MAX_POLYGONS]:
                    self.highlight.add(editor.require(), shape, _color(None, HIGHLIGHT),
                                       width=2, dither=5, halo=1)
                return {"net": None, "name": "", "live": True, "unbound": True,
                        "point": [round(point[0], 6), round(point[1], 6)]}
        net = data.net_at(point)
        name = l2n._name(data.nets[net]) if net is not None else ""
        return {"net": net, "name": name, "point": [round(point[0], 6), round(point[1], 6)]}

    def _live_name(self, net):
        result = self.live.result or {}
        nets = result.get("nets") or []
        return nets[net]["name"] if 0 <= net < len(nets) else ""

    def clear(self):
        self.highlight.clear()
        self.flylines.clear()
