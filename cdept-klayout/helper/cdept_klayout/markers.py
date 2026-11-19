# cdept-klayout: highlights of DRC and LVS results over the layout.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Markers drawn over the view (pya.Marker): not part of the layout, never saved.

`set(view, params)` replaces all markers. params:
  shapes: [{"kind": "polygon", "points": [[x, y], ...]},
           {"kind": "box", "box": [l, b, r, t]},
           {"kind": "edge", "points": [[x1, y1], [x2, y2]]},
           {"kind": "text", "at": [x, y], "text": "..."}]   (µm, top cell)
  color: "#rrggbb" (default orange-red)
  zoom: [l, b, r, t] to show, or true for the box around the shapes
An empty or missing `shapes` clears them.
"""

import pya

DEFAULT_COLOR = "#ff4000"
# Room around the shapes when zooming to them, as a part of their size (and at least
# this many µm).
MARGIN = 0.5
MIN_MARGIN = 0.5


class Markers:
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

    def set(self, view, params):
        if view is not self.view:
            # The markers of a view that went away went with it.
            self.markers = []
            self.view = view
        self.clear()
        color = _color(params.get("color") or DEFAULT_COLOR)
        box = pya.DBox()
        for shape in params.get("shapes") or []:
            target = _shape(shape)
            if target is None:
                continue
            marker = pya.Marker(view)
            marker.set(target)
            marker.color = color
            marker.frame_color = color
            marker.line_width = 2
            marker.vertex_size = 0
            marker.dither_pattern = 5
            marker.halo = 1
            self.markers.append(marker)
            box += target.bbox()
        zoom = params.get("zoom")
        if isinstance(zoom, list) and len(zoom) == 4:
            view.zoom_box(pya.DBox(*zoom))
        elif zoom and not box.empty():
            margin = max(MIN_MARGIN, MARGIN * max(box.width(), box.height()))
            view.zoom_box(box.enlarged(margin, margin))
        return {"markers": len(self.markers)}


def _color(text):
    try:
        return int(text.lstrip("#"), 16)
    except ValueError:
        return int(DEFAULT_COLOR.lstrip("#"), 16)


def _shape(shape):
    kind = shape.get("kind")
    if kind == "box":
        l, b, r, t = shape["box"]
        return pya.DBox(l, b, r, t)
    if kind == "polygon":
        points = [pya.DPoint(x, y) for x, y in shape.get("points") or []]
        return pya.DPolygon(points) if len(points) >= 3 else None
    if kind == "edge":
        (x1, y1), (x2, y2) = shape["points"][:2]
        return pya.DEdge(x1, y1, x2, y2)
    if kind == "text":
        x, y = shape["at"]
        return pya.DText(shape.get("text", ""), x, y)
    return None
