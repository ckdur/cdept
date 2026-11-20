# cdept-klayout: dragging the guiding shapes of a PCell instance (docs/KLAYOUT.md §4).
# SPDX-License-Identifier: GPL-3.0-or-later
"""Handles of PCell shape parameters, dragged with the mouse.

KLayout edits guiding shapes in its partial mode, which does not finish a drag without
a window. Here, in select mode, a selected PCell instance with shape parameters (Basic
CIRCLE's radius handle, a STROKED_BOX's box, a ROUND_POLYGON's points, …) shows a
square at every handle point. Pressing on one and dragging moves it; releasing changes
the parameter, as one undoable step, with the PCell's own coercion (CIRCLE's radius
follows its handle).
"""

import pya

from .editor import owner
from .properties import TYPE_NAMES, coerce

# How near (pixels) a press must be to a handle to take it.
CATCH_PIXELS = 7
# Size of a handle's square (pixels) and colours.
VERTEX_SIZE = 7
HANDLE_COLOR = 0x0080ff
DRAG_COLOR = 0xff8000


def _points(value):
    """The draggable points of a shape value, in the PCell's coordinates."""
    if isinstance(value, pya.DPoint):
        return [value]
    if isinstance(value, pya.DBox):
        return [value.p1, pya.DPoint(value.right, value.bottom), value.p2,
                pya.DPoint(value.left, value.top)]
    if isinstance(value, (pya.DPolygon, pya.DSimplePolygon)):
        return list(value.each_point_hull())
    if isinstance(value, pya.DPath):
        return list(value.each_point())
    if isinstance(value, pya.DEdge):
        return [value.p1, value.p2]
    return []


def _moved(value, index, p):
    """`value` with its point `index` at `p`."""
    if isinstance(value, pya.DPoint):
        return pya.DPoint(p.x, p.y)
    if isinstance(value, pya.DBox):
        # The corner moves; the opposite one stays.
        corners = _points(value)
        o = corners[(index + 2) % 4]
        return pya.DBox(min(o.x, p.x), min(o.y, p.y), max(o.x, p.x), max(o.y, p.y))
    if isinstance(value, (pya.DPolygon, pya.DSimplePolygon)):
        points = list(value.each_point_hull())
        points[index] = p
        return pya.DPolygon(points)
    if isinstance(value, pya.DPath):
        points = list(value.each_point())
        points[index] = p
        return pya.DPath(points, value.width, value.bgn_ext, value.end_ext, value.round)
    if isinstance(value, pya.DEdge):
        return pya.DEdge(p, value.p2) if index == 0 else pya.DEdge(value.p1, p)
    return value


def _outline(value):
    """What the marker of a dragged shape shows."""
    if isinstance(value, pya.DPoint):
        return pya.DEdge(value, value)
    if isinstance(value, (pya.DSimplePolygon,)):
        return pya.DPolygon(list(value.each_point()))
    return value


class Handles:
    """The handles of the selected PCell instance, and a drag of one of them."""

    def __init__(self, editor):
        self.editor = editor
        self.markers = []
        self.shown = None  # what the markers show: (view id, selection key)
        self.drag = None

    # -- what can be dragged ---------------------------------------------------

    def target(self):
        """(selection path, instance, declaration, [(name, index, point in µm of the
        view's top)], to-top transformation) of the one selected PCell instance with
        shape parameters that may be changed; None otherwise."""
        ed = self.editor
        view = ed.view
        if view is None or ed.doc is None or not ed.editable or view.mode_name() != "select":
            return None
        if view.selection_size() != 1:
            return None
        sel = next(iter(view.each_object_selected()), None)
        if sel is None or not sel.is_cell_inst() or ed._blocked(sel):
            return None
        if ed.doc.foreign.get(owner(sel)):
            return None
        inst = sel.inst()
        if not inst.is_pcell():
            return None
        decl = inst.pcell_declaration()
        values = inst.pcell_parameters_by_name()
        # Into the view's top: the path, the instance included (the array element the
        # selection names).
        to_top = sel.dtrans()
        handles = []
        for p in decl.get_parameters():
            if TYPE_NAMES.get(p.type) != "shape" or p.readonly or p.hidden:
                continue
            for index, q in enumerate(_points(values.get(p.name))):
                handles.append((p.name, index, to_top.trans(pya.DPoint(q.x, q.y))))
        if not handles:
            return None
        return sel, inst, decl, handles, to_top

    def _pixel(self, p):
        ed = self.editor
        q = ed.view.viewport_trans().trans(pya.DPoint(p.x, p.y))
        return pya.DPoint(q.x, ed.height - 1 - q.y)

    def _micron(self, point):
        ed = self.editor
        return ed.view.viewport_trans().inverted() * pya.DPoint(point.x, ed.height - 1 - point.y)

    def _snap(self, p):
        """`p` on the editing grid (else the database unit)."""
        grid = 0.0
        for name in ("edit-grid", "grid-micron"):
            try:
                grid = float(self.editor.view.get_config(name) or 0)
            except (TypeError, ValueError):
                grid = 0.0
            if grid > 0:
                break
        grid = grid if grid > 0 else self.editor.doc.layout.dbu
        return pya.DPoint(round(p.x / grid) * grid, round(p.y / grid) * grid)

    # -- markers -------------------------------------------------------------------

    def _clear(self):
        for m in self.markers:
            m._destroy() if hasattr(m, "_destroy") else None
        self.markers = []

    def _marker(self, shape, color):
        m = pya.Marker(self.editor.view)
        m.set(shape)
        m.color = color
        m.frame_color = color
        m.vertex_size = VERTEX_SIZE
        m.line_width = 1
        m.dither_pattern = 1  # hollow
        self.markers.append(m)
        return m

    def refresh(self):
        """Show the handles of the selected PCell instance (after a change of the
        selection or of the instance)."""
        if self.drag is not None:
            return
        target = self.target()
        key = None
        if target is not None:
            sel, inst, decl, handles, _ = target
            key = (id(self.editor.view), str(inst), tuple((n, i, str(p)) for n, i, p in handles))
        if key == self.shown:
            return
        self._clear()
        self.shown = key
        if target is not None:
            for _, _, p in target[3]:
                self._marker(pya.DEdge(p, p), HANDLE_COLOR)
        self.editor.dirty_frame = True

    # -- input -------------------------------------------------------------------

    def input(self, kind, event, point, mods):
        """Handle a mouse event when it is about a handle. True when it was."""
        if kind == "press":
            if event.get("button", "left") != "left" or mods:
                return False
            target = self.target()
            if target is None:
                return False
            best = None
            for name, index, p in target[3]:
                q = self._pixel(p)
                d = max(abs(q.x - point.x), abs(q.y - point.y))
                if d <= CATCH_PIXELS and (best is None or d < best[0]):
                    best = (d, name, index)
            if best is None:
                return False
            sel, inst, decl, _, to_top = target
            self.drag = {"sel": sel, "inst": inst, "decl": decl, "name": best[1],
                         "index": best[2], "to_cell": to_top.inverted(),
                         "to_top": to_top, "value": None}
            return True
        drag = self.drag
        if drag is None:
            return False
        if kind == "move":
            self._follow(point)
            return True
        if kind == "release":
            self._follow(point)
            self.drag = None
            self._clear()
            self.shown = None
            if drag["value"] is not None:
                self._apply(drag)
            self.refresh()
            return True
        if kind == "key" and event.get("key") == "Escape":
            self.cancel()
            return True
        return False

    def forget(self):
        """Another layout view: its markers are gone with the old one."""
        self.markers = []
        self.shown = None
        self.drag = None

    def cancel(self):
        if self.drag is not None:
            self.drag = None
            self._clear()
            self.shown = None
            self.refresh()

    def _follow(self, point):
        drag = self.drag
        p = self._snap(self._micron(point))
        values = drag["inst"].pcell_parameters_by_name()
        old = values.get(drag["name"])
        drag["value"] = _moved(old, drag["index"], drag["to_cell"] * p)
        self._clear()
        shape = _outline(drag["value"])
        self._marker(shape.transformed(drag["to_top"]) if hasattr(shape, "transformed")
                     else shape, DRAG_COLOR)
        self.editor.pointer = (round(p.x, 6), round(p.y, 6))
        self.editor.dirty_frame = True

    def _apply(self, drag):
        ed = self.editor
        inst = drag["inst"]
        ly = ed.doc.layout
        base = coerce(drag["decl"], ly, inst.pcell_parameters_by_name())
        base[drag["name"]] = drag["value"]
        values = coerce(drag["decl"], ly, base)
        ed.manager.transaction("Move PCell handle")
        try:
            inst.change_pcell_parameters(values)
        finally:
            ed.manager.commit()
        sel = drag["sel"]
        new = pya.ObjectInstPath()
        new.cv_index = sel.cv_index
        new.top = sel.top
        for element in list(sel.path)[:-1]:
            new.append_path(element)
        new.append_path(pya.InstElement(inst))
        ed.view.object_selection = [new]

