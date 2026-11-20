# cdept-klayout: the layout side of linking a schematic with a layout.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Instances of the top cell that stand for schematic instances (docs/LINKING.md).

A layout instance belongs to a schematic instance through the GDS property NAME_PROPERTY
(61) holding the instance name. CDePT works out what every schematic instance should
be (`items`, see `apply`); this module places, changes and finds the instances.

- `instances(editor)`: the bound instances of the top cell, by name;
- `apply(editor, params)`: make the top cell match `items` (one undoable step);
- `select(editor, names, zoom)`: select the instances of these names.
"""

import os

import pya

from . import properties

NAME_PROPERTY = 61
# Room between generated instances and rows, in µm.
GAP = 2.0
# A row of generated instances is wrapped at this width, in µm.
ROW_WIDTH = 60.0


def _name(inst):
    value = inst.property(NAME_PROPERTY)
    return None if value is None or value == "" else str(value)


def _r(value):
    return round(float(value), 6)


def _box(box):
    return [_r(box.left), _r(box.bottom), _r(box.right), _r(box.top)]


def _top(editor):
    doc = editor.doc
    if doc is None:
        raise RuntimeError("no layout is open")
    top = doc.top_cell()
    if top is None:
        raise RuntimeError("the layout has no top cell")
    return doc, top


def _describe(doc, inst):
    """What a bound instance is: cell, library, PCell parameters, bounding box."""
    info = {"bbox": _box(inst.dbbox()), "count": 1, "pcell": False, "params": {},
            "library": "", "cell": inst.cell.name}
    if inst.is_pcell():
        decl = inst.pcell_declaration()
        lib = inst.cell.library()
        info.update(pcell=True, cell=decl.name(), library=lib.name() if lib else "",
                    params={k: properties.to_json(v)
                            for k, v in inst.pcell_parameters_by_name().items()})
    else:
        lib = inst.cell.library()
        if lib is not None:
            info["library"] = lib.name()
            info["cell"] = inst.cell.basic_name()
        foreign = doc.foreign.get(inst.cell_index)
        if foreign:
            info["file"] = foreign.get("file")
            info["label"] = foreign.get("label")
    return info


def _bound(top):
    """Name -> instances of the top cell bound to it; and the unbound ones."""
    bound, unbound = {}, []
    for inst in top.each_inst():
        name = _name(inst)
        if name is None:
            unbound.append(inst)
        else:
            bound.setdefault(name, []).append(inst)
    return bound, unbound


def instances(editor):
    doc, top = _top(editor)
    bound, unbound = _bound(top)
    out = {}
    for name, insts in bound.items():
        info = _describe(doc, insts[0])
        info["count"] = len(insts)
        out[name] = info
    return {"instances": out, "unbound": len(unbound)}


# -- making the top cell match the schematic -------------------------------------------


def _pcell_variant(ly, item):
    """Cell index in `ly` for a PCell (or fixed cell) of a KLayout library."""
    lib = properties._library(item["library"])
    lib_ly = lib.layout()
    decl = lib_ly.pcell_declaration(item["cell"])
    if decl is not None:
        defaults = {p.name: p.default for p in decl.get_parameters()}
        values = properties.convert(decl, item.get("params") or {}, defaults)
        return ly.add_pcell_variant(lib, lib_ly.pcell_id(item["cell"]), values)
    if lib_ly.has_cell(item["cell"]):
        return ly.add_lib_cell(lib, lib_ly.cell(item["cell"]).cell_index())
    raise ValueError("%s has no cell or PCell %s" % (item["library"], item["cell"]))


def _cell_for(doc, item):
    """Cell index for an item: a PCell variant, or a layout view loaded as an external
    cell (like `place`)."""
    if item.get("kind") == "cell":
        from .document import RefTable
        table = RefTable(item.get("refs") or {"root": item["file"], "files": {}})
        doc.table.merge(table)
        return doc.place(table.root or item["file"], item.get("cell")).cell_index()
    return _pcell_variant(doc.layout, item)


def _same_target(doc, inst, item):
    if item.get("kind") == "cell":
        foreign = doc.foreign.get(inst.cell_index) or {}
        return os.path.normpath(foreign.get("file") or "") == os.path.normpath(item["file"])
    if inst.is_pcell():
        lib = inst.cell.library()
        return (inst.pcell_declaration().name() == item["cell"]
                and lib is not None and lib.name() == item["library"])
    lib = inst.cell.library()
    return lib is not None and lib.name() == item["library"] and \
        inst.cell.basic_name() == item["cell"]


def _changed_params(inst, item):
    """The parameters of `item` that differ from the instance's, converted."""
    decl = inst.pcell_declaration()
    current = inst.pcell_parameters_by_name()
    wanted = properties.convert(decl, item.get("params") or {}, current)
    return {k: v for k, v in wanted.items() if properties.to_json(current.get(k))
            != properties.to_json(v)}


class _Rows:
    """Where new instances go: rows to the right of what the cell has, one row per
    kind of device (wrapped at ROW_WIDTH), stacked upwards."""

    def __init__(self, top):
        box = top.dbbox()
        self.left = 0.0 if box.empty() else box.right + GAP
        self.y = 0.0 if box.empty() else box.bottom
        self.rows = {}  # key -> {"x", "y", "h"}

    def place(self, key, bbox):
        row = self.rows.get(key)
        if row is None or (row["x"] > self.left
                           and row["x"] + bbox.width() > self.left + ROW_WIDTH):
            row = {"x": self.left, "y": self.y, "h": 0.0}
            self.rows[key] = row
        # The cell's origin goes where its box's lower left corner lands.
        at = pya.DVector(row["x"] - bbox.left, row["y"] - bbox.bottom)
        row["x"] += bbox.width() + GAP
        row["h"] = max(row["h"], bbox.height())
        self.y = max(self.y, row["y"] + row["h"] + GAP)
        return at


def apply(editor, params):
    """Make the top cell match the schematic. params:
      items: [{name, symbol, kind: "pcell", library, cell, params} |
              {name, symbol, kind: "cell", library, cell, view, file, refs}]
      pins: [{name, direction}], pin_layer: "8/25" or null, pin_size: µm,
      remove: delete instances bound to names not in items.
    Instances of the right cell that are bound to nothing are taken over ("bound")
    before new ones are placed. One undoable step. Returns what was added, bound,
    updated, replaced, removed."""
    doc, top = _top(editor)
    if not editor.editable:
        raise RuntimeError("the layout is read-only")
    ly = doc.layout
    items = params.get("items") or []
    report = {"added": [], "bound": [], "updated": [], "replaced": [], "unchanged": [], "removed": [],
              "extra": [], "duplicates": [], "pins": [], "failed": []}
    bound, unbound = _bound(top)
    rows = _Rows(top)
    manager = editor.manager
    manager.transaction("Generate from schematic")
    try:
        new_items = []
        for item in items:
            name = item["name"]
            insts = bound.get(name) or []
            if len(insts) > 1:
                report["duplicates"].append(name)
            try:
                if not insts:
                    new_items.append(item)
                    continue
                inst = insts[0]
                if _same_target(doc, inst, item):
                    changes = _changed_params(inst, item) if inst.is_pcell() else {}
                    if changes:
                        inst.change_pcell_parameters(changes)
                        report["updated"].append(name)
                    else:
                        report["unchanged"].append(name)
                    continue
                ci = _cell_for(doc, item)
                trans = inst.dcplx_trans
                inst.delete()
                new = top.insert(pya.DCellInstArray(ci, trans))
                new.set_property(NAME_PROPERTY, name)
                report["replaced"].append(name)
            except Exception as error:
                report["failed"].append({"name": name, "error": str(error)})
        # New instances, by kind, in rows.
        key = lambda item: (item.get("kind") or "", item.get("library") or "",
                            item.get("cell") or "", item["name"])
        # An instance of the right cell that is bound to nothing yet is taken over
        # (a layout drawn before the link), left to right, bottom to top.
        free = sorted(unbound, key=lambda i: (i.dbbox().left, i.dbbox().bottom))
        for item in sorted(new_items, key=lambda item: item["name"]):
            match = next((i for i in free if _same_target(doc, i, item)), None)
            if match is None:
                continue
            free.remove(match)
            if match.is_pcell() and _changed_params(match, item):
                match.change_pcell_parameters(_changed_params(match, item))
            match.set_property(NAME_PROPERTY, item["name"])
            report["bound"].append(item["name"])
        new_items = [item for item in new_items if item["name"] not in report["bound"]]
        for item in sorted(new_items, key=key):
            try:
                ci = _cell_for(doc, item)
                bbox = ly.cell(ci).dbbox()
                if bbox.empty():
                    bbox = pya.DBox(0, 0, 0, 0)
                at = rows.place(key(item)[:3], bbox)
                new = top.insert(pya.DCellInstArray(ci, pya.DTrans(at)))
                new.set_property(NAME_PROPERTY, item["name"])
                report["added"].append(item["name"])
            except Exception as error:
                report["failed"].append({"name": item["name"], "error": str(error)})
        wanted = {item["name"] for item in items}
        for name, insts in sorted(bound.items()):
            if name in wanted:
                continue
            if params.get("remove"):
                for inst in insts:
                    inst.delete()
                report["removed"].append(name)
            else:
                report["extra"].append(name)
        report["pins"] = _pins(ly, top, params)
    finally:
        manager.commit()
    return report


def _pins(ly, top, params):
    """Labels for the schematic's pins that the top cell does not have yet, in a column
    left of the cell."""
    layer = params.get("pin_layer")
    pins = params.get("pins") or []
    if not layer or not pins:
        return []
    info = pya.LayerInfo.from_string(str(layer))
    li = ly.layer(info)
    have = {s.text_string for s in top.shapes(li).each() if s.is_text()}
    size = float(params.get("pin_size") or 0.1)
    box = top.dbbox()
    x = (0.0 if box.empty() else box.left) - 2 * GAP
    y = 0.0 if box.empty() else box.top
    added = []
    for pin in pins:
        name = pin.get("name") or ""
        if not name or name in have:
            continue
        text = pya.DText(name, pya.DTrans(pya.DVector(x, y)))
        text.size = size
        top.shapes(li).insert(text)
        have.add(name)
        added.append(name)
        y -= max(4 * size, 1.0)
    return added


# -- selection ---------------------------------------------------------------------


def select(editor, names, zoom=False):
    """Select the instances of the top cell bound to `names` (others are unselected).
    Returns the names found."""
    doc, top = _top(editor)
    view = editor.require()
    wanted = set(names or [])
    paths, found, box = [], [], pya.DBox()
    for inst in top.each_inst():
        name = _name(inst)
        if name not in wanted:
            continue
        path = pya.ObjectInstPath()
        path.top = top.cell_index()
        path.append_path(pya.InstElement.new(inst))
        paths.append(path)
        found.append(name)
        box += inst.dbbox()
    view.object_selection = paths
    if zoom and not box.empty():
        margin = max(1.0, 0.25 * max(box.width(), box.height()))
        view.zoom_box(box.enlarged(margin, margin))
    return {"found": sorted(set(found))}


def selected(editor):
    """Names bound to the selected instances of the top cell."""
    doc, top = _top(editor)
    view = editor.require()
    names = set()
    for sel in view.each_object_selected():
        if not sel.is_cell_inst():
            continue
        path = list(sel.each_inst())
        if not path:
            continue
        inst = path[0].inst()
        name = _name(inst)
        if name is not None:
            names.add(name)
    return {"names": sorted(names)}


def boxes(editor, names):
    """Bounding boxes (µm) of the instances of the top cell bound to `names`."""
    doc, top = _top(editor)
    wanted = set(names or [])
    return [_box(inst.dbbox()) for inst in top.each_inst() if _name(inst) in wanted]
