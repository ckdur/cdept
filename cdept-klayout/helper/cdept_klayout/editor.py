# cdept-klayout: the layout editor without a window (docs/KLAYOUT.md §1, §4).
# SPDX-License-Identifier: GPL-3.0-or-later
"""KLayout's LayoutView, fed with CDePT's input, drawing into a frame file."""

import mmap
import os
import tempfile

import pya

from .document import MARKER_LAYER, Document

# KLayout's ButtonState bits.
LEFT, MIDDLE, RIGHT = 8, 16, 32
SHIFT, CTRL, ALT = 1, 2, 4
BUTTONS = {"left": LEFT, "middle": MIDDLE, "right": RIGHT}

# Qt key codes for the named keys of the protocol.
KEYS = {
    "Escape": 0x01000000, "Tab": 0x01000001, "Backspace": 0x01000003,
    "Return": 0x01000004, "Enter": 0x01000005, "Insert": 0x01000006,
    "Delete": 0x01000007, "Home": 0x01000010, "End": 0x01000011,
    "Left": 0x01000012, "Up": 0x01000013, "Right": 0x01000014, "Down": 0x01000015,
    "PageUp": 0x01000016, "PageDown": 0x01000017,
}
for _n in range(1, 13):
    KEYS["F%d" % _n] = 0x01000030 + _n - 1

# Most selected objects described in a selection event.
MAX_SELECTION_ITEMS = 200


def modifiers(m):
    m = m or {}
    return (SHIFT if m.get("shift") else 0) | (CTRL if m.get("ctrl") else 0) | \
        (ALT if m.get("alt") else 0)


def color(value):
    return "#%06x" % (value & 0xffffff)


class Frame:
    """The frame file: `height` rows of `width * 4` bytes, B G R A."""

    def __init__(self, folder, serial, width, height):
        self.serial = serial
        self.width = width
        self.height = height
        self.stride = width * 4
        self.path = os.path.join(folder, "klayout-%d-%d.frame" % (os.getpid(), serial))
        size = self.stride * height
        with open(self.path, "wb") as f:
            f.truncate(size)
        self._file = open(self.path, "r+b")
        self.map = mmap.mmap(self._file.fileno(), size)

    def write(self, pixels):
        data = pixels.to_bytes()
        n = self.stride * self.height
        # 8 bytes of header (width, height), then the pixels.
        self.map[0:n] = data[8:8 + n]

    def close(self, remove=True):
        self.map.close()
        self._file.close()
        if remove:
            try:
                os.unlink(self.path)
            except OSError:
                pass

    def message(self):
        return {"type": "frame", "serial": self.serial, "path": self.path,
                "width": self.width, "height": self.height, "stride": self.stride,
                "format": "bgra"}


class Editor:
    """One open layout: view, document, frame, and the state last reported."""

    def __init__(self, send, frame_dir=None, display=False):
        self.send = send
        self.frame_dir = frame_dir
        self.display = display
        self.manager = pya.Manager()
        self.view = None
        self.doc = None
        self.editable = True
        self.width = 0
        self.height = 0
        self.frame = None
        self.serial = 0
        self.dirty_frame = False
        self.sent = {}  # kind -> last event sent
        self.pointer = (None, None)
        # Undo position counted by the helper: +1 per change seen after input or a
        # call, -1/+1 for undo/redo. `saved_depth` is the position of the last save
        # (None once a new change was made after undoing past it).
        self.depth = 0
        self.saved_depth = 0
        # Zoom to fit once the view has a size (open may come before resize).
        self.fit_pending = False
        self.layer_iters = []

    # -- opening ---------------------------------------------------------------

    def open(self, table, editable=True, technology=None, layer_properties=None):
        self.editable = editable
        self.manager = pya.Manager()
        view = pya.LayoutView(editable, self.manager)
        # Copy instances as references, never ask (the question is a Qt dialog).
        view.set_config("edit-hier-copy-mode", "0")
        # A click picks the instance, not a shape inside a cell of another view.
        view.set_config("edit-top-level-selection", "true")
        index = view.create_layout(editable)
        cv = view.cellview(index)
        doc = Document(cv.layout(), table).load()
        top = doc.top_cell()
        if top is not None:
            cv.cell = top
        if technology:
            cv.technology = technology
        if layer_properties:
            view.load_layer_props(layer_properties, True)
        view.add_missing_layers()
        self._style_marker(view)
        view.max_hier()
        if self.width and self.height:
            view.resize(self.width, self.height)
        view.zoom_fit()
        if self.view is not None:
            self.view._destroy() if hasattr(self.view, "_destroy") else None
        self.view = view
        self.doc = doc
        self.sent = {}
        self.depth = 0
        self.mark_saved()
        self.fit_pending = not (self.width and self.height)
        self.dirty_frame = True
        return {"top": top.name if top else "", "dbu": doc.layout.dbu,
                "missing": doc.missing(), "external": doc.external(), "notes": doc.notes}

    def _style_marker(self, view):
        """Red, hollow, thick: how a "cannot be found" cell looks."""
        it = view.begin_layers()
        while not it.at_end():
            node = it.current()
            if node.source.startswith(MARKER_LAYER):
                node.fill_color = 0xff0000
                node.frame_color = 0xff0000
                node.dither_pattern = 1  # hollow
                node.width = 2
                node.name = "cannot be found"
                view.set_layer_properties(it, node)
            it.next()

    def require(self):
        if self.view is None:
            raise RuntimeError("no layout is open")
        return self.view

    def cellview(self):
        return self.require().cellview(0)

    # -- display ---------------------------------------------------------------

    def resize(self, width, height):
        width, height = max(1, int(width)), max(1, int(height))
        if (width, height) == (self.width, self.height) and self.frame is not None:
            return
        self.width, self.height = width, height
        if self.view is not None:
            self.view.resize(width, height)
            if self.fit_pending:
                self.fit_pending = False
                self.view.zoom_fit()
        if self.display and self.frame_dir:
            old = self.frame
            self.serial += 1
            self.frame = Frame(self.frame_dir, self.serial, width, height)
            self.send(self.frame.message())
            if old is not None:
                old.close()
        self.dirty_frame = True

    def render(self):
        """Draw into the frame if something changed; True when a frame was sent."""
        if not (self.dirty_frame and self.frame and self.view):
            return False
        self.dirty_frame = False
        pixels = self.view.get_pixels_with_options(self.frame.width, self.frame.height)
        self.frame.write(pixels)
        self.send({"type": "damage", "serial": self.frame.serial, "x": 0, "y": 0,
                   "width": self.frame.width, "height": self.frame.height})
        return True

    def close(self):
        if self.frame is not None:
            self.frame.close()
            self.frame = None

    # -- input -----------------------------------------------------------------

    def input(self, event):
        """Feed one input event. Errors from KLayout become notices."""
        view = self.view
        if view is None:
            return
        kind = event.get("kind")
        mods = modifiers(event.get("modifiers"))
        x, y = float(event.get("x", 0.0)), float(event.get("y", 0.0))
        point = pya.DPoint(x, y)
        try:
            if kind == "move":
                buttons = 0
                for b in event.get("buttons") or []:
                    buttons |= BUTTONS.get(b, 0)
                view.send_mouse_move_event(point, buttons | mods)
                self.pointer = self._micron(x, y)
            elif kind in ("press", "release", "double_click"):
                b = BUTTONS.get(event.get("button"), LEFT) | mods
                if kind == "press":
                    view.send_mouse_press_event(point, b)
                elif kind == "release":
                    view.send_mouse_release_event(point, b)
                else:
                    view.send_mouse_double_clicked_event(point, b)
            elif kind == "wheel":
                view.send_wheel_event(int(event.get("delta", 0)),
                                      bool(event.get("horizontal")), point, mods)
            elif kind == "key":
                code = KEYS.get(event.get("key") or "")
                text = event.get("text") or ""
                if code is None and text:
                    code = ord(text[0].upper())
                if code is not None:
                    view.send_key_press_event(code, mods)
            elif kind == "enter":
                view.send_enter_event()
            elif kind == "leave":
                view.send_leave_event()
                self.pointer = (None, None)
        except Exception as error:
            self.notice("error", _message(error))
        self.dirty_frame = True

    def _micron(self, x, y):
        """Pixel position (y down) → µm."""
        if not self.height:
            return (None, None)
        p = self.view.viewport_trans().inverted() * pya.DPoint(x, self.height - 1 - y)
        return (round(p.x, 6), round(p.y, 6))

    def foreign_selection(self):
        """Selected objects that live inside external or missing cells."""
        if self.view is None or self.doc is None:
            return []
        return [s for s in self.view.each_object_selected()
                if owner(s) in self.doc.foreign]

    def drop_foreign_selection(self):
        """Unselect objects inside cells of other views (they would be edited but never
        saved) and cancel a move that may have started with them."""
        if not self.foreign_selection():
            return
        keep = [s for s in self.view.each_object_selected() if owner(s) not in self.doc.foreign]
        self.view.cancel()
        self.view.object_selection = keep
        self.notice("warning", "objects inside cells of other views cannot be edited here; "
                               "open that view to change them")
        self.dirty_frame = True

    def notice(self, level, text):
        self.send({"type": "event", "event": {"kind": "notice", "level": level, "text": text}})

    # -- state events ------------------------------------------------------------

    def report(self, full=True):
        """Send the events whose content changed. `full` = also layers, cells and
        selection (skipped after plain pointer motion)."""
        if self.view is None:
            return
        events = [self.status_event(), self.modified_event()]
        if full:
            events += [self.layers_event(), self.cells_event(), self.selection_event()]
        for event in events:
            if self.sent.get(event["kind"]) != event:
                self.sent[event["kind"]] = event
                self.send({"type": "event", "event": event})

    def status_event(self):
        x, y = self.pointer
        return {"kind": "status", "x": x, "y": y, "mode": self.view.mode_name(), "message": ""}

    def _changed(self):
        """True when KLayout saw a change since the last call (its dirty flag), which is
        then cleared. KLayout clears it only when saving through the view, so the view
        "saves" nothing (no layers, no cells) to a scratch file: 0.2 ms."""
        cv = self.view.cellview(0)
        if not cv.is_dirty():
            return False
        options = pya.SaveLayoutOptions()
        options.deselect_all_layers()
        options.clear_cells()
        options.format = "GDS2"
        scratch = os.path.join(self.frame_dir or tempfile.gettempdir(),
                               "klayout-%d.clean.gds" % os.getpid())
        try:
            self.view.save_as(0, scratch, options)
        finally:
            if os.path.exists(scratch):
                os.unlink(scratch)
        return True

    def track(self):
        """Count one step if the layout changed (after an input event or a call)."""
        if self.view is None or not self._changed():
            return
        if self.saved_depth is not None and self.saved_depth > self.depth:
            self.saved_depth = None  # changed after undoing past the save: no way back
        self.depth += 1

    def undone(self, step):
        """After undo (step -1) or redo (+1)."""
        self.depth += step
        self._changed()

    def forget_changes(self):
        """What happened so far is not an edit (loading, placing)."""
        if self.view is not None:
            self._changed()

    def is_dirty(self):
        return self.view is not None and self.depth != self.saved_depth

    def mark_saved(self):
        self.forget_changes()
        self.saved_depth = self.depth

    def modified_event(self):
        return {"kind": "modified", "dirty": self.is_dirty(),
                "can_undo": self.manager.has_undo(), "can_redo": self.manager.has_redo()}

    def layers(self):
        """The flattened layer list: [(iterator, node, depth)]."""
        out = []
        it = self.view.begin_layers()
        while not it.at_end():
            depth, p = 0, it.parent()
            while not p.is_null():
                depth += 1
                p = p.parent()
            out.append((it.dup(), it.current(), depth))
            it.next()
        return out

    def layers_event(self):
        current = self.view.current_layer
        layers = []
        self.layer_iters = []
        for index, (it, node, depth) in enumerate(self.layers()):
            self.layer_iters.append(it)
            source = node.source
            layers.append({
                "id": index,
                "name": node.name or source,
                "source": source,
                "color": color(node.eff_fill_color(True)),
                "frame_color": color(node.eff_frame_color(True)),
                "dither": node.eff_dither_pattern(True),
                "visible": bool(node.visible),
                "current": not current.is_null() and it == current,
                "depth": depth,
                "valid": bool(node.valid),
            })
        return {"kind": "layers", "layers": layers}

    def layer_iter(self, index):
        layers = self.layers()
        if not 0 <= int(index) < len(layers):
            raise ValueError("no layer %s" % index)
        return layers[int(index)]

    def cells_event(self):
        cv = self.cellview()
        ly = self.doc.layout
        top = self.doc.top_cell()
        path = [ly.cell(ci).name for ci in cv.path] if cv.is_valid() else []
        cells = []
        for c in ly.each_cell():
            info = self.doc.foreign.get(c.cell_index())
            cells.append({
                "name": c.name,
                "children": sorted({ly.cell(ci).name for ci in c.each_child_cell()}),
                "external": info["label"] if info and info["top"] and not info["missing"] else None,
                "missing": bool(info and info["missing"]),
                "file": info["file"] if info else None,
            })
        return {"kind": "cells", "top": top.name if top else "", "path": path, "cells": cells}

    def selection_event(self):
        view = self.view
        ly = self.doc.layout
        count = view.selection_size()
        box = view.selection_bbox()
        items = []
        for sel in view.each_object_selected():
            if len(items) >= MAX_SELECTION_ITEMS:
                break
            ci = owner(sel)
            cell = ly.cell(ci).name if ly.is_valid_cell_index(ci) else ""
            if sel.is_cell_inst():
                inst = sel.inst()
                items.append({"kind": "instance", "layer": "", "cell": cell,
                              "text": "%s %s" % (ly.cell(inst.cell_index).name, inst.dcplx_trans)})
            else:
                shape = sel.shape
                items.append({"kind": _shape_kind(shape), "cell": cell,
                              "layer": str(ly.get_info(sel.layer)),
                              "text": shape.text_string if shape.is_text() else str(shape.dbbox())})
        bbox = None if box.empty() else [box.left, box.bottom, box.right, box.top]
        return {"kind": "selection", "count": count, "bbox": bbox, "items": items}


def owner(sel):
    """Index of the cell holding a selected object. For an instance that is the cell
    placing it (`cell_index()` names the placed cell)."""
    if sel.is_cell_inst():
        path = sel.path
        return path[-2].inst().cell_index if len(path) > 1 else sel.top
    return sel.cell_index()


def _shape_kind(shape):
    for kind in ("box", "polygon", "path", "text", "edge", "point"):
        test = getattr(shape, "is_" + kind, None)
        if test is not None and test():
            return kind
    return "shape"


def _message(error):
    """KLayout's error text without the " in LayoutViewBase.send_…" tail."""
    text = str(error)
    cut = text.find(" in LayoutViewBase.")
    return text[:cut] if cut > 0 else text
