# cdept-klayout: one layout with its references filled in (docs/KLAYOUT.md §3).
# SPDX-License-Identifier: GPL-3.0-or-later
"""A layout file and the cells it places from other views.

The root file is read as it is. Every ghost cell in it (placed, not defined) is
looked up in the reference table CDePT sent: found -> the other file's top cell is
copied into the ghost cell ("external"); not found -> the ghost cell gets a red box
with an X and "<label> cannot be found" on a layer that is never written
("missing"). The same happens, recursively, to the ghost cells of external files.

Saving writes the root's own cells only: external and missing cells become ghost
cells again, cells that came with external files are left out. Cells of KLayout
libraries (PDK PCells, the project's PCell views, docs/KLAYOUT.md §3.6) stay library
cells wherever they are placed: KLayout writes them with their geometry and a note of
where they come from, and reads them back as live library cells. External cells edited
in place (descended into) are written back to their own files the same way: each file
gets its own cells, with the cells of other files as ghosts under the names that file
uses for them.
"""

import os

import pya

from .pins import Pins

# Layer of the "cannot be found" markers. It has no GDS number, lives only in memory
# and is removed before anything is written.
MARKER_LAYER = "CDEPT_MISSING"
# Size of a marker when no bounding box is known, in µm.
DEFAULT_BBOX = [0.0, 0.0, 2.0, 2.0]


class RefTable:
    """The reference table of docs/KLAYOUT.md §3.3."""

    def __init__(self, table):
        table = table or {}
        self.root = table.get("root") or ""
        self.files = {}
        for name, entry in (table.get("files") or {}).items():
            self.files[os.path.normpath(name)] = entry or {}
        # The project's PCell views: {CDePT library: {cell: file}} (pcells.py).
        self.pcells = table.get("pcells") or {}
        # How pins are drawn in this technology (pins.py); None: no rules.
        self.pin_rules = table.get("pin_rules")

    def entry(self, path):
        return self.files.get(os.path.normpath(path), {})

    def label(self, path):
        return self.entry(path).get("label") or os.path.basename(path)

    def read_only(self, path):
        """True when CDePT says the file must not be written (read-only library)."""
        return bool(self.entry(path).get("read_only"))

    def pins(self, path):
        """The pins the sidecar of file `path` lists: {name: record}."""
        return self.entry(path).get("pins") or {}

    def ref(self, path, name):
        """The table's entry for ghost `name` of file `path`, or None."""
        return (self.entry(path).get("refs") or {}).get(name)

    def merge(self, other):
        """Add the files of another table (after `place`)."""
        for name, entry in other.files.items():
            self.files.setdefault(name, entry)
        for library, cells in other.pcells.items():
            self.pcells.setdefault(library, {}).update(cells)
        if self.pin_rules is None:
            self.pin_rules = other.pin_rules


def _bbox(value):
    if isinstance(value, (list, tuple)) and len(value) == 4:
        try:
            b = [float(v) for v in value]
        except (TypeError, ValueError):
            return None
        if b[0] <= b[2] and b[1] <= b[3]:
            return b
    return None


class Document:
    """The layout `layout` built from a reference table.

    `foreign` maps the index of every cell that did not come from the root file to
    what it is: {"file", "label", "missing", "bbox", "top", "reason", "name"}. `top` is
    true for the cell standing for a whole file (what a ghost name refers to); `name` is
    the cell's name inside its file (the copy may have been renamed here).
    `ghost_names[file][cell index]` is the name `file` uses for a cell of another file.
    """

    def __init__(self, layout, table):
        self.layout = layout
        self.table = table
        self.root = os.path.normpath(table.root)
        self.foreign = {}
        self.by_file = {}
        self.ghost_names = {}
        self.dbu_of = {}  # database unit of each external file
        # Files another tab is editing (docs/KLAYOUT.md §3.4): not to be changed here.
        self.locked = set()
        self._layers = {}
        self.marker = None
        self.notes = []
        # Pins (pins.py, docs/LINKING.md §9).
        self.pins = Pins(self, table.pin_rules)

    # -- loading ---------------------------------------------------------------

    def load(self):
        """Read the root file and fill all its ghost cells."""
        self.layout.read(self.root)
        for ci in [c.cell_index() for c in self.layout.each_cell() if c.is_ghost_cell()]:
            self._fill(ci, self.root, [self.root])
        self.realize_pins()
        return self

    def realize_pins(self):
        """The pins of the sidecars (root and external files) that the stream does not
        hold (no pin layer in this technology, or never saved), made in the cells."""
        top = self.top_cell()
        if top is not None:
            self.pins.realize(top.cell_index(), self.root, self.table.pins(self.root))
        for ci, info in sorted(self.foreign.items()):
            if info["top"] and not info["missing"] and info.get("file"):
                self.pins.realize(ci, info["file"], self.table.pins(info["file"]))

    def top_cell(self):
        """The root's own top cell (the first one when there are several)."""
        tops = [c for c in self.layout.top_cells() if c.cell_index() not in self.foreign]
        if not tops:
            return None
        base = os.path.basename(self.root).split(".")[0]
        for c in tops:
            if c.name == base:
                return c
        return tops[0]

    def missing(self):
        return sorted(self.layout.cell(ci).name for ci, f in self.foreign.items() if f["missing"])

    def external(self):
        return sorted(
            self.layout.cell(ci).name
            for ci, f in self.foreign.items()
            if f["top"] and not f["missing"]
        )

    def place(self, path, name=None):
        """Load file `path` (in the table) as an external cell; returns its cell."""
        path = os.path.normpath(path)
        if path in self.by_file:
            return self.layout.cell(self.by_file[path])
        cell = self.layout.create_cell(self._unique(name or self._file_top_name(path)))
        ci = cell.cell_index()
        self._import(ci, path, self.table.label(path), [self.root, path])
        return self.layout.cell(ci)

    def _file_top_name(self, path):
        return os.path.basename(path).split(".")[0]

    def _fill(self, ci, from_file, stack):
        """Give ghost cell `ci` (named as in file `from_file`) its content."""
        name = self.layout.cell(ci).name
        ref = self.table.ref(from_file, name)
        if ref is None:
            self._mark_missing(ci, name, None, "not in the reference table")
            return ci
        label = ref.get("label") or name
        path = ref.get("file")
        if not path:
            self._mark_missing(ci, label, ref.get("bbox"), ref.get("reason") or "not found")
            return ci
        path = os.path.normpath(path)
        # A file being read (one placing this one): placing it again would be a cycle.
        if path in stack:
            self._mark_missing(ci, label, ref.get("bbox"), "it contains itself")
            return ci
        if path in self.by_file:
            target = self.by_file[path]
            self._redirect(ci, target)
            return target
        try:
            self._import(ci, path, label, stack + [path])
        except Exception as error:  # unreadable file and the like
            self._mark_missing(ci, label, ref.get("bbox"), str(error))
        return ci

    def _import(self, ci, path, label, stack):
        """Copy the top cell of file `path` (and what it places) into cell `ci`."""
        src = pya.Layout()
        src.read(path)
        tops = [c for c in src.top_cells() if not c.is_ghost_cell()]
        if not tops:
            raise RuntimeError("%s has no cell" % path)
        base = self._file_top_name(path)
        top = next((c for c in tops if c.name == base), tops[0])
        if len(tops) > 1:
            self.notes.append("%s has %d top cells, using %s" % (path, len(tops), top.name))

        self.by_file[path] = ci
        self.foreign[ci] = {"file": path, "label": label, "missing": False,
                            "bbox": None, "top": True, "reason": "", "name": top.name}
        self.dbu_of[path] = src.dbu
        cell = self.layout.cell(ci)
        cell.ghost_cell = False

        # A target cell for every cell below the top.
        mapping = {top.cell_index(): ci}
        owned = {top.cell_index()}  # cells whose content comes from this file
        for sci in top.called_cells():
            sc = src.cell(sci)
            if sc.is_ghost_cell():
                mapping[sci] = self._nested(sc.name, path, stack)
                self.ghost_names.setdefault(path, {})[mapping[sci]] = sc.name
            elif sc.is_proxy() and self._library_cell(sc) is not None:
                # A library cell or PCell: the same library cell here, still live.
                mapping[sci] = self._library_cell(sc)
            else:
                new = self.layout.create_cell(self._unique(sc.name))
                mapping[sci] = new.cell_index()
                owned.add(sci)
                self.foreign[new.cell_index()] = {"file": path, "label": label, "missing": False,
                                                  "bbox": None, "top": False, "reason": "",
                                                  "name": sc.name}

        scale = pya.ICplxTrans(src.dbu / self.layout.dbu)
        layers = {li: self._layer(src.get_info(li)) for li in src.layer_indexes()}
        for sci in owned:
            sc = src.cell(sci)
            target = self.layout.cell(mapping[sci])
            for li, tl in layers.items():
                shapes = sc.shapes(li)
                if not shapes.is_empty():
                    target.shapes(tl).insert(shapes, scale)
            for inst in sc.each_inst():
                array = inst.dcell_inst  # in µm: independent of the database unit
                array.cell_index = mapping[inst.cell_index]
                if inst.prop_id:
                    target.insert(array, inst.prop_id)
                else:
                    target.insert(array)
        self.foreign[ci]["bbox"] = self._dbbox(ci)

    def _library_cell(self, sc):
        """The index of the cell here that is library cell `sc` of another layout (a
        PCell variant with the same parameters), or None when `sc` is not linked to a
        library that is there (a PCell of a library not registered: copied as is)."""
        lib = sc.library() if sc.is_library_cell() else None
        if lib is None:
            return None
        try:
            if sc.is_pcell_variant():
                decl = sc.pcell_declaration()
                pid = lib.layout().pcell_id(decl.name())
                return self.layout.add_pcell_variant(lib, pid, sc.pcell_parameters_by_name())
            return self.layout.add_lib_cell(lib, sc.library_cell_index())
        except Exception as error:
            self.notes.append("%s: %s" % (sc.name, error))
            return None

    def _nested(self, name, path, stack):
        """The target cell for ghost `name` of external file `path`."""
        ref = self.table.ref(path, name)
        target = os.path.normpath(ref["file"]) if ref and ref.get("file") else None
        if target in self.by_file and target not in stack:
            return self.by_file[target]
        cell = self.layout.create_cell(self._unique(name))
        return self._fill(cell.cell_index(), path, stack)

    def _redirect(self, ci, target):
        """Make every instance of cell `ci` place `target` instead, then delete `ci`."""
        cell = self.layout.cell(ci)
        for parent in list(cell.each_parent_inst()):
            parent.child_inst().cell_index = target
        self.layout.delete_cell(ci)

    def _mark_missing(self, ci, label, bbox, reason):
        """Draw the "cannot be found" marker into cell `ci`."""
        b = _bbox(bbox) or list(DEFAULT_BBOX)
        cell = self.layout.cell(ci)
        cell.clear()
        cell.ghost_cell = False
        shapes = cell.shapes(self.marker_layer())
        l, bo, r, t = b
        shapes.insert(pya.DBox(l, bo, r, t))
        shapes.insert(pya.DPath([pya.DPoint(l, bo), pya.DPoint(r, t)], 0))
        shapes.insert(pya.DPath([pya.DPoint(l, t), pya.DPoint(r, bo)], 0))
        text = pya.DText("%s cannot be found" % label, pya.DTrans(pya.DVector(l, t)))
        text.size = max((t - bo) / 10.0, self.layout.dbu)
        shapes.insert(text)
        self.foreign[ci] = {"file": None, "label": label, "missing": True,
                            "bbox": b, "top": True, "reason": reason or "",
                            "name": self.layout.cell(ci).name}

    def marker_layer(self):
        if self.marker is None:
            self.marker = self.layout.layer(pya.LayerInfo(MARKER_LAYER))
        return self.marker

    def _layer(self, info):
        key = str(info)
        if key not in self._layers:
            self._layers[key] = self.layout.layer(info)
        return self._layers[key]

    def _unique(self, name):
        if not self.layout.has_cell(name):
            return name
        n = 1
        while self.layout.has_cell("%s$%d" % (name, n)):
            n += 1
        return "%s$%d" % (name, n)

    def _dbbox(self, ci):
        box = self.layout.cell(ci).dbbox()
        if box.empty():
            return None
        return [box.left, box.bottom, box.right, box.top]

    # -- editing in place ------------------------------------------------------

    def file_of(self, ci):
        """The file cell `ci` belongs to: the root, an external file, or None (missing)."""
        info = self.foreign.get(ci)
        if info is None:
            return self.root
        return None if info["missing"] else info["file"]

    def why_not_writable(self, path):
        """None when `path` may be changed here, else the reason (a sentence)."""
        if path is None:
            return "a cell that cannot be found cannot be edited"
        label = self.table.label(path)
        if path in self.locked:
            return "%s has unsaved changes in another tab" % label
        if path != self.root and self.table.read_only(path):
            return "%s belongs to a read-only library" % label
        return None

    # -- writing ---------------------------------------------------------------

    def _without(self, ghost_foreign):
        """A copy of the layout ready to write: missing cells (and all foreign cells
        when `ghost_foreign`) are ghosts again, cells nothing uses are gone, the
        marker layer is removed. Returns (layout, {ghost cell index: info})."""
        # A copy without the undo manager: changing a managed layout outside a
        # transaction would wipe the editor's undo list.
        out = pya.Layout()
        out.assign(self.layout)
        ghosts = {}
        for ci, info in self.foreign.items():
            if info["missing"] or ghost_foreign:
                out.cell(ci).clear()
        # Keep what the root's own cells reach (library cells too: KLayout writes them
        # with their geometry and their library link).
        own = [ci for ci in range(out.cells()) if out.is_valid_cell_index(ci)
               and ci not in self.foreign and not out.cell(ci).is_proxy()]
        keep = set(own)
        for ci in own:
            keep.update(out.cell(ci).called_cells())
        drop = [ci for ci in range(out.cells()) if out.is_valid_cell_index(ci)
                and ci not in keep and (ci in self.foreign or out.cell(ci).is_proxy())]
        if drop:
            out.delete_cells(drop)
        for ci, info in self.foreign.items():
            if ci in keep and (info["missing"] or ghost_foreign):
                out.cell(ci).ghost_cell = True
                ghosts[ci] = info
        if self.marker is not None:
            out.delete_layer(self.marker)
        if self.pins.overlay_index() is not None:
            out.delete_layer(self.pins.overlay_index())
        return out, ghosts

    def save(self, path=None, also=()):
        """Write the root's own cells to `path` (default: the root file), and each
        external file of `also` back to itself. Returns the SaveReport of
        docs/KLAYOUT.md §4; the reports of `also` are in its "also"."""
        report = self._save_root(path)
        report["also"] = [self.save_external(f) for f in sorted(also)
                          if f in self.by_file and f != self.root]
        return report

    def save_external(self, path):
        """Write the cells that came from external file `path` back to it: its top cell
        under its own name, its own cells, the cells of other files as ghosts."""
        top = self.by_file[path]
        out = pya.Layout()
        out.assign(self.layout)

        def own(ci):
            info = self.foreign.get(ci)
            return info is not None and not info["missing"] and info["file"] == path
        def library_cell(ci):
            return out.cell(ci).is_proxy()
        # Every cell that is not this file's is emptied first, so that what it places
        # is not reached from here. Library cells stay: they are written as they are.
        for ci in range(out.cells()):
            if out.is_valid_cell_index(ci) and not own(ci) and not library_cell(ci):
                out.cell(ci).clear()
        keep = {top} | set(out.cell(top).called_cells())
        drop = [ci for ci in range(out.cells())
                if out.is_valid_cell_index(ci) and ci not in keep]
        names = {}
        ghosts = {}
        for ci in keep:
            if own(ci):
                names[ci] = self.foreign[ci]["name"]
                continue
            if library_cell(ci):
                continue
            # Another file's cell (or the root's, or a missing one): a reference.
            out.cell(ci).ghost_cell = True
            names[ci] = self.ghost_names.get(path, {}).get(ci) or self.layout.cell(ci).name
            ghosts[ci] = self.foreign.get(ci)
        if drop:
            out.delete_cells(drop)
        # Two rounds, so that no name is taken while renaming.
        for ci in names:
            out.rename_cell(ci, "cdept$tmp$%d" % ci)
        for ci, name in names.items():
            out.rename_cell(ci, name)
        if self.marker is not None:
            out.delete_layer(self.marker)
        if self.pins.overlay_index() is not None:
            out.delete_layer(self.pins.overlay_index())
        dbu = self.dbu_of.get(path)
        if dbu is not None and abs(dbu - out.dbu) > 1e-12:
            self.notes.append("%s was written with database unit %g (it had %g)"
                              % (path, out.dbu, dbu))
        _write(out, path)
        references = {}
        for ci, info in ghosts.items():
            if info is None:  # the root's own cell, placed by an external one
                references[names[ci]] = {"file": self.root, "missing": False,
                                         "bbox": self._dbbox(ci)}
                continue
            references[names[ci]] = {
                "file": info["file"] if info["top"] and not info["missing"] else None,
                "missing": info["missing"] or not info["top"],
                "bbox": info["bbox"] if info["missing"] else (self._dbbox(ci) or info["bbox"]),
            }
        self.foreign[top]["bbox"] = self._dbbox(top)
        return {"file": path, "top": self.foreign[top]["name"], "references": references,
                "pins": self.pins.records(top, path)}

    def _save_root(self, path=None):
        path = os.path.normpath(path or self.root)
        out, ghosts = self._without(ghost_foreign=True)
        _write(out, path)
        references = {}
        for ci, info in ghosts.items():
            name = self.layout.cell(ci).name
            if not info["top"]:
                self.notes.append("%s is a cell inside %s, not a view; saved as a reference "
                                  "that will not be found" % (name, info["file"]))
            references[name] = {
                "file": info["file"] if info["top"] else None,
                "missing": info["missing"] or not info["top"],
                "bbox": info["bbox"] if info["missing"] else (self._dbbox(ci) or info["bbox"]),
            }
        top = self.top_cell()
        report = {"file": path, "top": top.name if top else "", "references": references}
        if top is not None:
            report["pins"] = self.pins.records(top.cell_index(), self.root)
        return report

    def export(self, path):
        """Write a self-contained file: every reference filled in, missing ones left as
        ghost cells. Returns {file, missing: [names]}."""
        out, ghosts = self._without(ghost_foreign=False)
        # For other tools (DRC, LVS, tape-out): plain geometry, no library links.
        _write(out, path, context=False)
        top = self.top_cell()
        return {"file": os.path.normpath(path),
                "missing": sorted(self.layout.cell(ci).name for ci in ghosts),
                # The pins as they are now (the net check, docs/LINKING.md §9).
                "pins": self.pins.records(top.cell_index(), self.root) if top else {}}


def _write(layout, path, context=True):
    """Write through a temporary file so a failure leaves the old file intact.
    `context`: with KLayout's notes on library cells (read back as live PCells)."""
    folder, name = os.path.split(path)
    # Keep the suffix: KLayout picks the format (and compression) from it.
    tmp = os.path.join(folder, ".%s.tmp-%d.%s" % (name.split(".")[0], os.getpid(), name.split(".", 1)[-1]))
    try:
        if context:
            layout.write(tmp)
        else:
            options = pya.SaveLayoutOptions()
            options.set_format_from_filename(tmp)
            options.write_context_info = False
            layout.write(tmp, options)
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)
