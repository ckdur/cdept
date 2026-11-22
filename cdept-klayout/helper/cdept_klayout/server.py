# cdept-klayout: the protocol loop (docs/KLAYOUT.md §4).
# SPDX-License-Identifier: GPL-3.0-or-later
"""Connects to CDePT, answers calls, feeds input, sends frames and events."""

import ast
import json
import os
import select
import socket
import sys
import time
import traceback

import pya

from .document import RefTable
from . import properties
from . import link
from . import pins
from . import pcells
from .editor import Editor, _message
from .markers import Markers
from .nets import Nets

PROTOCOL_VERSION = 1
# How long one wait for requests lasts; KLayout's own events are processed between.
POLL = 0.01
# Pointer-only status updates are sent at most this often.
STATUS_INTERVAL = 0.03
# Seconds between looks at the project's PCell files (pcells.py).
PCELL_CHECK = 1.0


class Connection:
    """JSON lines over the Unix socket."""

    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(path)
        self.buffer = b""
        self.closed = False

    def send(self, message):
        data = (json.dumps(message, separators=(",", ":")) + "\n").encode()
        try:
            self.sock.sendall(data)
        except OSError:
            self.closed = True

    def receive(self, timeout):
        """Messages that arrived within `timeout` seconds (all that are there)."""
        ready, _, _ = select.select([self.sock], [], [], timeout)
        messages = []
        while ready:
            try:
                chunk = self.sock.recv(1 << 16)
            except OSError:
                chunk = b""
            if not chunk:
                self.closed = True
                break
            self.buffer += chunk
            ready, _, _ = select.select([self.sock], [], [], 0)
        while b"\n" in self.buffer:
            line, self.buffer = self.buffer.split(b"\n", 1)
            if line.strip():
                try:
                    messages.append(json.loads(line))
                except ValueError as error:
                    print("cdept-klayout: bad message: %s" % error, file=sys.stderr)
        return messages


class ModalGuard:
    """Closes any modal Qt dialog KLayout opens: nobody could answer it, and its
    event loop would block the helper. Needs KLayout's Qt bindings; without them
    (the pip module) there is no QApplication and no dialog either."""

    def __init__(self, notice):
        self.notice = notice
        self.timer = None
        if not hasattr(pya, "QTimer") or not hasattr(pya, "QApplication"):
            return
        self.timer = pya.QTimer()
        self.timer.interval = 50
        self.timer.timeout = self._check
        self.timer.start()

    def _check(self):
        widget = pya.QApplication.activeModalWidget()
        if widget is not None:
            title = widget.windowTitle
            widget.close()
            self.notice("warning", "KLayout opened a dialog CDePT cannot show (%s); "
                                   "it was cancelled" % (title or "untitled"))


class Server:
    def __init__(self, connection, frame_dir, display):
        self.conn = connection
        self.app = pya.Application.instance()
        self.editor = Editor(self.send, frame_dir=frame_dir, display=display)
        self.running = True
        self.python_globals = {"pya": pya, "server": self}
        self.last_status = 0.0
        self.guard = ModalGuard(self.editor.notice)
        self.markers = Markers()
        self.nets = Nets()
        self.last_pcell_check = time.monotonic()

    def send(self, message):
        self.conn.send(message)

    # -- main loop -------------------------------------------------------------

    def run(self):
        self.send({"type": "ready", "protocol": PROTOCOL_VERSION,
                   "klayout": self.app.version() if self.app else "KLayout"})
        while self.running and not self.conn.closed:
            messages = self.conn.receive(POLL)
            full = False
            for message in messages:
                full |= self.handle(message)
                if not self.running:
                    break
            self.process_events()
            self.pcells_tick()
            self.live_tick()
            self.editor.render()
            now = time.monotonic()
            if full or (messages and now - self.last_status >= STATUS_INTERVAL):
                self.last_status = now
                try:
                    self.editor.report(full=full)
                except Exception as error:
                    print("cdept-klayout: report: %s" % error, file=sys.stderr)
        self.editor.close()

    # -- PCells of the project (pcells.py) ------------------------------------------

    def pcells_tick(self):
        """Now and then: register PCell files that changed again (KLayout rebuilds
        their instances, which is not an edit), and show what went wrong in them."""
        now = time.monotonic()
        if now - self.last_pcell_check >= PCELL_CHECK:
            self.last_pcell_check = now
            self.refresh_pcells()
        for level, text in pcells.take_messages():
            self.editor.notice(level, text)

    def refresh_pcells(self):
        try:
            changed = pcells.refresh()
        except Exception as error:
            self.editor.notice("error", "PCells: %s" % _message(error))
            return False
        if changed and self.editor.view is not None:
            self.editor.forget_changes()
            self.editor.dirty_frame = True
        return changed

    def m_refresh_pcells(self, p):
        if "pcells" in p:
            pcells.want(p["pcells"])
        changed = self.refresh_pcells()
        return {"changed": changed, "libraries": pcells.libraries()}

    def process_events(self):
        if self.app is not None and hasattr(self.app, "process_events"):
            self.app.process_events()
        view = self.editor.view
        if view is not None and hasattr(view, "timer"):
            view.timer()  # the Qt-less pip module redraws from here

    def handle(self, message):
        """Handle one request. True when the full state should be reported."""
        kind = message.get("type")
        if kind == "input":
            event = message.get("input") or {}
            if self.editor.probe_selection and event.get("kind") not in (
                    "move", "wheel", "enter", "leave", "release"):
                self.editor.drop_probe_selection()
            self.editor.input(event)
            full = event.get("kind") not in ("move", "wheel", "enter", "leave")
            if full:
                self.editor.track()
                self.editor.show_pin_layer()
                self.editor.drop_foreign_selection()
            return full
        if kind == "resize":
            self.editor.resize(message.get("width", 1), message.get("height", 1))
            return False
        if kind == "shutdown":
            self.send({"type": "reply", "id": message.get("id", 0), "ok": True, "value": None})
            self.running = False
            return False
        if kind == "call":
            self.call(message.get("id", 0), message.get("method", ""), message.get("params"))
            return True
        print("cdept-klayout: unknown message type %r" % kind, file=sys.stderr)
        return False

    def call(self, id, method, params):
        handler = getattr(self, "m_" + method, None) if method.isidentifier() else None
        if handler is None:
            self.send({"type": "reply", "id": id, "ok": False,
                       "error": "unknown method %s" % method})
            return
        try:
            value = handler(params or {})
            self.editor.show_pin_layer()
            if method not in ("undo", "redo", "save", "open", "place"):
                self.editor.track()
            if method != "open":
                self.editor.drop_foreign_selection()
            self.send({"type": "reply", "id": id, "ok": True, "value": value})
        except Exception as error:
            if os.environ.get("CDEPT_KLAYOUT_DEBUG"):
                traceback.print_exc()
            self.send({"type": "reply", "id": id, "ok": False, "error": _message(error)})
        self.editor.dirty_frame = True

    # -- methods (docs/KLAYOUT.md §4) --------------------------------------------

    def m_open(self, p):
        table = RefTable(p.get("refs"))
        if not table.root:
            raise ValueError("open: the reference table has no root")
        value = self.editor.open(table, editable=p.get("editable", True),
                                 technology=p.get("technology"),
                                 layer_properties=p.get("layer_properties"))
        self.editor.doc.locked = {os.path.normpath(f) for f in p.get("locked") or []}
        return value

    def m_save(self, p):
        doc = self._doc()
        in_place = not p.get("file") or os.path.normpath(p["file"]) == doc.root
        # External cells edited in place go back to their files with the root.
        report = doc.save(p.get("file"), also=self.editor.unsaved if in_place else ())
        if in_place:
            self.editor.mark_saved()
            self.editor.report(full=False)  # "modified" goes out before the reply
        self._notes(doc)
        return report

    def m_export(self, p):
        if p.get("refs"):
            doc, _ = self._temporary(p["refs"])
        else:
            doc = self._doc()
        out = p.get("out")
        if not out:
            raise ValueError("export: no 'out' file")
        return doc.export(out)

    def m_render(self, p):
        out = p.get("out")
        if not out:
            raise ValueError("render: no 'out' file")
        width, height = int(p.get("width", 400)), int(p.get("height", 300))
        if p.get("refs"):
            _, view = self._temporary(p["refs"], p.get("technology"), p.get("layer_properties"))
        else:
            view = self.editor.require()
        if p.get("refs"):
            # One cell of a file that holds several (a standard-cell GDS).
            cell = p.get("cell")
            if cell:
                ly = view.cellview(0).layout()
                if not ly.has_cell(cell):
                    raise ValueError("render: no cell %s" % cell)
                view.select_cell_path([ly.cell(cell).cell_index()], 0)
                view.max_hier()
            view.resize(width, height)
            view.zoom_fit()
        view.get_pixels_with_options(width, height).write_png(out)
        return {"file": out, "width": width, "height": height}

    def _temporary(self, refs, technology=None, layer_properties=None):
        """A document and view of their own (previews, exports), not shown."""
        from .document import Document
        view = pya.LayoutView(False)
        index = view.create_layout(False)
        cv = view.cellview(index)
        if technology:
            cv.technology = technology
        table = RefTable(refs)
        pcells.sync(table.pcells)
        doc = Document(cv.layout(), table).load()
        top = doc.top_cell()
        if top is not None:
            cv.cell = top
        if layer_properties:
            view.load_layer_props(layer_properties, True)
        view.add_missing_layers()
        self.editor._style_marker(view)
        view.max_hier()
        return doc, view

    def m_mode(self, p):
        view = self._view()
        name = p.get("name", "select")
        if name not in view.mode_names():
            raise ValueError("unknown mode %s (known: %s)" % (name, " ".join(view.mode_names())))
        view.switch_mode(name)

    def m_zoom(self, p):
        view = self._view()
        what = p.get("what", "fit")
        if what == "fit":
            view.zoom_fit()
        elif what == "in":
            view.zoom_in()
        elif what == "out":
            view.zoom_out()
        elif what == "box":
            l, b, r, t = p["box"]
            view.zoom_box(pya.DBox(l, b, r, t))
        elif what == "selection":
            view.zoom_fit_sel()
        else:
            raise ValueError("zoom: unknown %s" % what)

    def m_markers(self, p):
        """DRC/LVS highlights (markers.py); empty `shapes` clears them."""
        return self.markers.set(self._view(), p)

    def m_undo(self, p):
        view = self._view()
        view.cancel()
        if self.editor.manager.has_undo():
            self.editor.manager.undo()
            self.editor.undone(-1)

    def m_redo(self, p):
        view = self._view()
        view.cancel()
        if self.editor.can_redo():
            self.editor.manager.redo()
            self.editor.undone(+1)

    def _refuse_foreign(self, what):
        if self.editor.foreign_selection():
            text = ("cannot %s objects inside cells of other views; descend into the "
                    "instance or open that view" % what)
            self.editor.notice("warning", text)
            raise RuntimeError(text)

    def m_delete(self, p):
        self._refuse_foreign("delete")
        self._view().call_menu("cm_delete")

    def m_copy(self, p):
        self._view().call_menu("cm_copy")

    def m_cut(self, p):
        self._refuse_foreign("cut")
        self._view().call_menu("cm_cut")

    def m_paste(self, p):
        self._view().call_menu("cm_paste")

    def m_select_all(self, p):
        self._view().call_menu("cm_select_all")

    def m_clear_selection(self, p):
        self._view().clear_selection()

    def m_layer_visible(self, p):
        it, node, _ = self.editor.layer_iter(p["id"])
        node.visible = bool(p.get("visible", True))
        self._view().set_layer_properties(it, node)

    def m_current_layer(self, p):
        it, _, _ = self.editor.layer_iter(p["id"])
        self._view().current_layer = it

    def m_add_layer(self, p):
        """Add a layer to the layout and to the layer list, with KLayout's next default
        colour and pattern (as add_missing_layers gives them)."""
        view = self._view()
        layer, datatype = int(p["layer"]), int(p.get("datatype", 0))
        name = p.get("name") or ""
        info = pya.LayerInfo(layer, datatype, name) if name else pya.LayerInfo(layer, datatype)
        self._doc().layout.layer(info)
        view.add_missing_layers()
        source = "%d/%d@1" % (layer, datatype)
        for index, (it, node, _) in enumerate(self.editor.layers()):
            if node.source.split(" ")[-1] == source or node.source == source:
                if name and node.name != name:
                    node.name = name
                    view.set_layer_properties(it, node)
                return {"id": index}
        return {"id": None}

    def m_show_cell(self, p):
        view = self._view()
        ly = self._doc().layout
        path = []
        for name in p.get("path") or []:
            if not ly.has_cell(name):
                raise ValueError("no cell %s" % name)
            path.append(ly.cell(name).cell_index())
        if not path:
            raise ValueError("show_cell: empty path")
        view.select_cell_path(path, 0)
        ctx = self.editor.context()
        if ctx is not None and not ctx["writable"]:
            self.editor.notice("warning", "this cell belongs to another view and cannot be "
                                          "changed here: " + ctx["reason"])

    # -- editing in place (docs/KLAYOUT.md §3.4) ---------------------------------

    def m_descend(self, p):
        return self.editor.descend()

    def m_ascend(self, p):
        return self.editor.ascend(all_levels=bool(p.get("all")))

    def m_locks(self, p):
        self.editor.set_locked(p.get("files"))

    def m_levels(self, p):
        view = self._view()
        view.min_hier_levels = int(p.get("min", 0))
        view.max_hier_levels = int(p.get("max", 99))

    def m_place(self, p):
        doc = self._doc()
        table = RefTable(p.get("refs"))
        if not table.root:
            raise ValueError("place: the reference table has no root")
        doc.table.merge(table)
        pcells.sync(doc.table.pcells)
        cell = doc.place(table.root, p.get("name"))
        self.editor.forget_changes()  # loading is not an edit
        self._notes(doc)
        view = self._view()
        view.set_config("edit-inst-lib-name", "")
        view.set_config("edit-inst-cell-name", cell.name)
        view.set_config("edit-inst-pcell-parameters", "")
        view.switch_mode("instance")
        return {"name": cell.name}

    # -- properties and PCells (properties.py) ------------------------------------

    def m_properties(self, p):
        self._doc()
        return properties.describe(self.editor)

    def m_set_properties(self, p):
        self._doc()
        return properties.apply(self.editor, p)

    def m_libraries(self, p):
        return properties.libraries()

    def m_pcell_form(self, p):
        return properties.pcell_form(self.editor, p["library"], p["cell"])

    def m_place_library(self, p):
        self._doc()
        return properties.place_library(self.editor, p["library"], p["cell"], p.get("values"))

    def m_replace_cell(self, p):
        doc = self._doc()
        if p.get("refs"):  # another layout view of the project
            table = RefTable(p["refs"])
            doc.table.merge(table)
            pcells.sync(doc.table.pcells)
            cell = doc.place(table.root, p.get("name"))
            self.editor.forget_changes()  # loading is not an edit
            self._notes(doc)
            p = dict(p, target=cell.cell_index())
        return properties.replace_cell(self.editor, p)

    # -- linking with a schematic (link.py, docs/LINKING.md) ------------------------

    def m_link_apply(self, p):
        return link.apply(self.editor, p)

    def m_link_instances(self, p):
        return link.instances(self.editor)

    def m_link_select(self, p):
        return link.select(self.editor, p.get("names"), zoom=bool(p.get("zoom")))

    def m_link_selected(self, p):
        return link.selected(self.editor)

    def m_link_highlight(self, p):
        """Markers around the instances of these names (empty: clear)."""
        boxes = link.boxes(self.editor, p.get("names"))
        shapes = [{"kind": "box", "box": b} for b in boxes]
        return self.markers.set(self._view(), {"shapes": shapes, "color": p.get("color"),
                                               "zoom": bool(p.get("zoom"))})

    # -- pins (pins.py, docs/LINKING.md §9) -------------------------------------------

    def m_pins_list(self, p):
        return pins.listing(self.editor)

    def m_pins_add(self, p):
        return pins.add(self.editor, p)

    def m_pins_remove(self, p):
        return pins.remove(self.editor, p)

    def m_pins_rename(self, p):
        return pins.rename(self.editor, p)

    def m_pins_update(self, p):
        return pins.update(self.editor, p)

    def m_pins_select(self, p):
        return pins.select(self.editor, p)

    def m_pins_tool(self, p):
        return self.editor.pin_tool.arm(p)

    def m_pins_style(self, p):
        return pins.style(self.editor, p)

    # -- nets (nets.py, docs/LINKING.md §8) -------------------------------------------

    def m_nets_load(self, p):
        """Load the LVS database of the last extraction: {lvsdb, top?}."""
        return self.nets.load(p)

    def m_nets_highlight(self, p):
        """Draw the shapes of nets: {nets: [id], color?, zoom?}; empty clears."""
        return self.nets.show(self._view(), p)

    def m_nets_flylines(self, p):
        """Draw flylines: {lines: [[x1, y1, x2, y2]], color?}; empty clears."""
        return self.nets.lines(self._view(), p)

    def m_nets_probe(self, p):
        """The net at {point: [x, y]} (µm) or {pixel: [x, y]}: {net, name, point}."""
        return self.nets.probe(self.editor, p)

    def m_nets_clear(self, p):
        self.nets.clear()

    def m_nets_live(self, p):
        """Live connectivity (live.py): {connectivity?: the extraction's JSON (set up and
        calibrate now), rules?, on?, delay_ms?} → its state. With `update`, trace now
        and answer the result."""
        status = self.nets.live.configure(self.editor, p)
        if p.get("update") and self.nets.live.seeds and self.nets.live.conn is not None:
            return self.nets.live.update(self.editor)
        return status

    def live_tick(self):
        """Trace the terminals again when the layout has changed and is quiet."""
        live = self.nets.live
        try:
            if live.due(self.editor):
                self.send({"type": "event", "event": live.update(self.editor)})
        except Exception as error:
            live.on = False
            self.editor.notice("warning", "Live flylines stopped: %s" % _message(error))

    def m_config(self, p):
        self._view().set_config(p["name"], str(p.get("value", "")))

    def m_get_config(self, p):
        return self._view().get_config(p["name"])

    def m_info(self, p):
        doc = self.editor.doc
        top = doc.top_cell() if doc else None
        cv = self.editor.cellview() if self.editor.view else None
        return {"klayout": self.app.version() if self.app else "",
                "technology": cv.technology if cv else "",
                "file": doc.root if doc else None,
                "top": top.name if top else None,
                "cells": doc.layout.cells() if doc else 0,
                "dbu": doc.layout.dbu if doc else None}

    def m_python(self, p):
        """Run Python code; the value of a last expression is returned as repr."""
        code = p.get("code", "")
        g = self.python_globals
        g.update(view=self.editor.view, doc=self.editor.doc, editor=self.editor,
                 layout=self.editor.doc.layout if self.editor.doc else None)
        tree = ast.parse(code, "<cdept>", "exec")
        last = None
        if tree.body and isinstance(tree.body[-1], ast.Expr):
            last = ast.Expression(tree.body.pop().value)
        exec(compile(tree, "<cdept>", "exec"), g)
        if last is not None:
            return repr(eval(compile(last, "<cdept>", "eval"), g))
        return None

    # -- helpers ---------------------------------------------------------------

    def _view(self):
        return self.editor.require()

    def _doc(self):
        if self.editor.doc is None:
            raise RuntimeError("no layout is open")
        return self.editor.doc

    def _notes(self, doc):
        for note in doc.notes:
            self.editor.notice("warning", note)
        doc.notes = []


def run(options):
    """Entry point; `options` are the script's globals (KLayout's `-rd` values)."""
    path = options.get("connect")
    if not path:
        print("cdept-klayout: start with -rd connect=<socket>", file=sys.stderr)
        return 2
    conn = Connection(path)
    display = str(options.get("display", "0")) not in ("", "0", "false")
    frame_dir = options.get("frame_dir") or None
    try:
        server = Server(conn, frame_dir, display)
    except Exception as error:
        conn.send({"type": "failed", "error": "cannot start: %s" % error})
        return 1
    server.run()
    return 0
