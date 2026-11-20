# cdept-klayout: properties of the selection and PCell parameters (docs/KLAYOUT.md §4).
# SPDX-License-Identifier: GPL-3.0-or-later
"""What CDePT's properties dialog and PCell forms show, and how their edits are made.

KLayout's own dialogs are Qt; CDePT draws its forms from what this module describes:
- `describe(editor)`: the selected objects (box, polygon, path, text, instance), in µm;
- `apply(editor, params)`: one edited object back, as one undoable step;
- `libraries()`, `pcell_form(...)`, `place_library(...)`: KLayout's libraries (PDK
  libraries registered on KLAYOUT_PATH, "Basic"), and placing their cells and PCells.
"""

import pya

from .editor import owner

# PCellParameterDeclaration types by name.
_TYPES = {
    "TypeInt": "int", "TypeDouble": "double", "TypeString": "string", "TypeBoolean": "bool",
    "TypeList": "list", "TypeLayer": "layer", "TypeShape": "shape", "TypeCallback": "callback",
    "TypeNone": "none",
}
TYPE_NAMES = {getattr(pya.PCellParameterDeclaration, k): v for k, v in _TYPES.items()
              if hasattr(pya.PCellParameterDeclaration, k)}

# Most objects described at once.
MAX_OBJECTS = 200


def _r(value):
    return round(float(value), 9)


def _point(p):
    return [_r(p.x), _r(p.y)]


# -- PCell values: KLayout's values <-> JSON ----------------------------------------


def to_json(value):
    """A PCell parameter value as JSON. Shapes (guiding shapes) are objects in µm:
    {"point": [x, y]}, {"box": [l, b, r, t]}, {"polygon": [[x, y], …]},
    {"path": [[x, y], …], "width": w}, {"edge": [[x, y], [x, y]]}."""
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (list, tuple)):
        return [to_json(v) for v in value]
    shape = shape_to_json(value)
    if shape is not None:
        return shape
    return str(value)  # LayerInfo ("1/0", "name (1/0)"), …


def shape_to_json(value):
    """A guiding shape value as JSON (see `to_json`), or None for other values."""
    if isinstance(value, (pya.DPoint, pya.Point)):
        return {"point": _point(value)}
    if isinstance(value, (pya.DBox, pya.Box)):
        return {"box": [_r(value.left), _r(value.bottom), _r(value.right), _r(value.top)]}
    if isinstance(value, (pya.DPolygon, pya.DSimplePolygon, pya.Polygon, pya.SimplePolygon)):
        return {"polygon": [_point(p) for p in value.each_point_hull()]}
    if isinstance(value, (pya.DPath, pya.Path)):
        return {"path": [_point(p) for p in value.each_point()], "width": _r(value.width)}
    if isinstance(value, (pya.DEdge, pya.Edge)):
        return {"edge": [_point(value.p1), _point(value.p2)]}
    return None


def shape_from_json(value, old=None):
    """The guiding shape of `value` (see `to_json`); `old` when it is not one."""
    if value is None:
        return None
    if not isinstance(value, dict):
        return old
    if "point" in value:
        x, y = (float(v) for v in value["point"])
        return pya.DPoint(x, y)
    if "box" in value:
        l, b, r, t = (float(v) for v in value["box"])
        return pya.DBox(min(l, r), min(b, t), max(l, r), max(b, t))
    if "polygon" in value:
        points = _pts(value["polygon"])
        if len(points) < 3:
            raise ValueError("a polygon needs at least three points")
        return pya.DPolygon(points)
    if "path" in value:
        points = _pts(value["path"])
        if not points:
            raise ValueError("a path needs at least one point")
        width = float(value.get("width", old.width if isinstance(old, pya.DPath) else 0))
        return pya.DPath(points, width)
    if "edge" in value:
        (x1, y1), (x2, y2) = value["edge"]
        return pya.DEdge(float(x1), float(y1), float(x2), float(y2))
    return old


def from_json(kind, value, old=None):
    """A JSON value as the PCell parameter of type `kind` wants it. Values CDePT
    cannot edit (callbacks) keep `old`."""
    if kind == "int":
        return int(round(float(value)))
    if kind == "double":
        return float(value)
    if kind == "string":
        return "" if value is None else str(value)
    if kind == "bool":
        return bool(value)
    if kind == "list":
        if isinstance(value, str):
            return [v.strip() for v in value.split(",") if v.strip()]
        return [str(v) for v in (value or [])]
    if kind == "layer":
        if value in (None, ""):
            return pya.LayerInfo()
        if isinstance(value, pya.LayerInfo):
            return value
        return pya.LayerInfo.from_string(str(value))
    if kind == "shape":
        return shape_from_json(value, old)
    return old


def describe_parameters(decl):
    """The parameter declarations of a PCell: what the form shows."""
    out = []
    for p in decl.get_parameters():
        kind = TYPE_NAMES.get(p.type, "none")
        description = p.description or p.name
        group = ""
        if "\t" in description:  # KLayout's "Group<TAB>Label"
            group, description = description.split("\t", 1)
        choices = []
        try:
            for label, value in zip(p.choice_descriptions(), p.choice_values()):
                choices.append({"label": label, "value": to_json(value)})
        except Exception:
            pass
        out.append({
            "name": p.name, "type": kind, "description": description, "group": group,
            "unit": p.unit or "", "hidden": bool(p.hidden), "readonly": bool(p.readonly),
            "default": to_json(p.default), "choices": choices,
        })
    return out


def _decl_types(decl):
    return {p.name: TYPE_NAMES.get(p.type, "none") for p in decl.get_parameters()}


def convert(decl, values, old):
    """`values` (JSON, by name) on top of `old` (KLayout values, by name)."""
    types = _decl_types(decl)
    out = dict(old)
    for name, value in (values or {}).items():
        if name not in types:
            raise ValueError("%s has no parameter %s" % (decl.name(), name))
        out[name] = from_json(types[name], value, old.get(name))
    return out


def coerce(decl, layout, values):
    """`values` (by name) as the PCell makes them consistent (its
    `coerce_parameters`: e.g. CIRCLE's radius follows its handle)."""
    if type(decl) is getattr(pya, "PCellDeclaration_Native", ()):
        # A C++ PCell: KLayout's binding runs the base class's coercion (none), so
        # Basic's own rules are ported (basic.py); other C++ PCells stay as they are.
        from .basic import coerce as basic
        return basic(decl.name(), values) or dict(values)
    names = [p.name for p in decl.get_parameters()]
    try:
        out = decl.coerce_parameters(layout, [values.get(n) for n in names])
    except Exception:
        return dict(values)
    return dict(zip(names, out))


def changed_parameters(decl, layout, current, changes):
    """The parameters of an instance after the form's `changes` (JSON, by name): the
    current ones made consistent first (hidden parameters such as CIRCLE's last radius
    tell what changed), then the changes, then consistent again."""
    base = coerce(decl, layout, current)
    return coerce(decl, layout, convert(decl, changes, base))


def _form(library, decl, values):
    return {"library": library, "name": decl.name(), "params": describe_parameters(decl),
            "values": {k: to_json(v) for k, v in values.items()}}


# -- PCell parameter strings (KLayout's edit-inst-pcell-parameters) ---------------


def _quoted(text):
    return "'" + text.replace("\\", "\\\\").replace("'", "\\'") + "'"


def _word(text):
    ok = text and all(c.isalnum() or c in "_.$" for c in text)
    return text if ok else _quoted(text)


def parsable(value):
    """tl::Variant::to_parsable_string for the values a PCell parameter has."""
    if value is None:
        return "nil"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return "#%d" % value
    if isinstance(value, float):
        return "##%s" % repr(value)
    if isinstance(value, str):
        return _quoted(value)
    if isinstance(value, (list, tuple)):
        return "(" + ",".join(parsable(v) for v in value) + ")"
    if isinstance(value, pya.LayerInfo):
        return "[layer:%s]" % value
    return "[%s:%s]" % (type(value).__name__.lower(), value)


def parameter_string(values):
    """What KLayout's instance mode reads from `edit-inst-pcell-parameters`."""
    return "!" + "".join("%s:%s;" % (_word(k), parsable(v)) for k, v in sorted(values.items()))


# -- KLayout's libraries ----------------------------------------------------------


def libraries():
    """KLayout's libraries (not CDePT's): name, description, cells and PCells."""
    from .pcells import libraries as registered
    project_libraries = set(registered())
    out = []
    for lib in _all_libraries():
        name = lib.name()
        ly = lib.layout()
        cells = [{"name": n, "pcell": True} for n in sorted(ly.pcell_names())]
        cells += [{"name": c.name, "pcell": False}
                  for c in sorted(ly.top_cells(), key=lambda c: c.name)
                  if not c.is_pcell_variant() and not c.is_proxy()]
        out.append({"name": name, "description": lib.description or "",
                    "technologies": list(lib.technologies()), "cells": cells,
                    "project": name in project_libraries})
    return out


def _all_libraries():
    """Every registered library, by id: `library_by_name(name)` misses libraries bound
    to a technology (PDK libraries such as IHP's SG13_dev are)."""
    return [lib for lib in (pya.Library.library_by_id(i) for i in pya.Library.library_ids())
            if lib is not None]


def _library(name):
    lib = pya.Library.library_by_name(name)
    if lib is None:
        lib = next((l for l in _all_libraries() if l.name() == name), None)
    if lib is None:
        raise ValueError("there is no KLayout library %s" % name)
    return lib


def _current_layer(editor):
    it = editor.view.current_layer
    if it.is_null():
        return None
    node = it.current()
    if not node.has_source_name and node.source_layer < 0:
        return None
    ly = editor.doc.layout
    li = node.layer_index()
    if li >= 0 and ly.is_valid_layer(li):
        return ly.get_info(li)
    return None


def pcell_form(editor, library, cell):
    """The form of a PCell of a library, with its defaults (a layer parameter without a
    default gets the current layer)."""
    lib = _library(library)
    decl = lib.layout().pcell_declaration(cell)
    if decl is None:
        raise ValueError("%s has no PCell %s" % (library, cell))
    values = {}
    current = _current_layer(editor) if editor.view is not None else None
    for p in decl.get_parameters():
        value = p.default
        if TYPE_NAMES.get(p.type) == "layer" and value is None and current is not None:
            value = current
        values[p.name] = value
    return _form(library, decl, values)


def place_library(editor, library, cell, values=None):
    """Instance mode with a cell or PCell of a KLayout library."""
    lib = _library(library)
    view = editor.require()
    ly = lib.layout()
    decl = ly.pcell_declaration(cell)
    if decl is not None:
        defaults = {p.name: p.default for p in decl.get_parameters()}
        params = parameter_string(coerce(decl, editor.doc.layout, convert(decl, values, defaults)))
    elif ly.has_cell(cell):
        params = ""
    else:
        raise ValueError("%s has no cell %s" % (library, cell))
    view.set_config("edit-inst-lib-name", library)
    view.set_config("edit-inst-cell-name", cell)
    view.set_config("edit-inst-pcell-parameters", params)
    view.switch_mode("instance")
    return {"name": cell, "pcell": decl is not None}


# -- the selection ------------------------------------------------------------------


def _key(sel, ly):
    """What identifies an object between `describe` and `apply`."""
    if sel.is_cell_inst():
        inst = sel.inst()
        return "inst %d %s %s" % (owner(sel), ly.cell(inst.cell_index).name, inst.cell_inst)
    return "shape %d %d %s" % (sel.cell_index(), sel.layer, sel.shape)


def _selection(editor):
    return list(editor.view.each_object_selected())[:MAX_OBJECTS]


def describe(editor):
    """The selected objects, each as the properties dialog shows it."""
    view = editor.require()
    doc = editor.doc
    ly = doc.layout
    out = []
    for index, sel in enumerate(_selection(editor)):
        ci = owner(sel)
        foreign = doc.foreign.get(ci)
        item = {"index": index, "key": _key(sel, ly),
                "parent": ly.cell(ci).name if ly.is_valid_cell_index(ci) else "",
                "editable": bool(editor.editable and not foreign), "reason": ""}
        if not editor.editable:
            item["reason"] = "the layout is read-only"
        elif foreign:
            item["reason"] = "inside %s, a cell of another view" % foreign["label"]
        if sel.is_cell_inst():
            item.update(_describe_instance(doc, sel.inst()))
        else:
            item["layer"] = str(ly.get_info(sel.layer))
            item.update(_describe_shape(sel.shape))
        out.append(item)
    return {"objects": out, "count": view.selection_size(),
            "layers": [str(ly.get_info(li)) for li in ly.layer_indexes()
                       if li != doc.marker]}


def _describe_shape(shape):
    if shape.is_box():
        b = shape.dbox
        return {"kind": "box", "bbox": [_r(b.left), _r(b.bottom), _r(b.right), _r(b.top)]}
    if shape.is_path():
        p = shape.dpath
        return {"kind": "path", "width": _r(p.width), "points": [_point(q) for q in p.each_point()],
                "begin_ext": _r(p.bgn_ext), "end_ext": _r(p.end_ext), "round": bool(p.round)}
    if shape.is_text():
        t = shape.dtext
        tr = t.trans
        return {"kind": "text", "string": t.string, "size": _r(t.size),
                "x": _r(tr.disp.x), "y": _r(tr.disp.y), "rotation": int(tr.angle),
                "mirror": bool(tr.is_mirror()), "halign": int(t.halign), "valign": int(t.valign)}
    if shape.is_polygon() or shape.is_simple_polygon():
        p = shape.dpolygon
        return {"kind": "polygon", "points": [_point(q) for q in p.each_point_hull()],
                "holes": p.holes(), "offset": [0.0, 0.0]}
    return {"kind": "other", "text": str(shape)}


def _describe_instance(doc, inst):
    ly = doc.layout
    cell = ly.cell(inst.cell_index)
    t = inst.dcplx_trans
    info = doc.foreign.get(inst.cell_index)
    lib = cell.library() if cell.is_library_cell() else None
    out = {"kind": "instance", "cell": cell.name, "layer": None,
           "library": lib.name() if lib else None,
           "external": info["label"] if info and not info["missing"] else None,
           "missing": bool(info and info["missing"]),
           "x": _r(t.disp.x), "y": _r(t.disp.y), "angle": _r(t.angle),
           "mirror": bool(t.is_mirror()), "mag": _r(t.mag), "array": None, "pcell": None}
    if lib is not None:
        out["cell"] = cell.basic_name() if hasattr(cell, "basic_name") else cell.name
    if inst.is_regular_array():
        out["array"] = {"a": _point(inst.da), "b": _point(inst.db), "na": inst.na, "nb": inst.nb}
    if inst.is_pcell():
        decl = inst.pcell_declaration()
        out["cell"] = decl.name()
        out["pcell"] = _form(lib.name() if lib else "", decl, inst.pcell_parameters_by_name())
    return out


def apply(editor, params):
    """Change one selected object: `params` = {index, key, object: {…as described…},
    pcell: {name: value} (changed parameters only)}. One undoable step; the selection
    keeps the object. Returns what `describe` returns now."""
    view = editor.require()
    ly = editor.doc.layout
    selection, index, sel = _chosen(editor, params)
    obj = params.get("object") or {}
    # Everything that can be wrong is found before anything changes: KLayout's Manager
    # cannot take back half a transaction.
    if sel.is_cell_inst():
        change = _instance_change(ly, sel, obj, params.get("pcell") or {})
    else:
        change = _shape_change(ly, sel, obj)
    manager = editor.manager
    manager.transaction("Properties")
    try:
        new = change()
    finally:
        manager.commit()
    selection[index] = new
    view.object_selection = selection
    return describe(editor)


def _chosen(editor, params):
    """(selection, index, object) of the object `params` names ({index, key}), after
    checking it may be changed."""
    view = editor.require()
    doc = editor.doc
    index = int(params.get("index", -1))
    selection = list(view.each_object_selected())
    if not 0 <= index < len(selection) or _key(selection[index], doc.layout) != params.get("key"):
        raise RuntimeError("the selection changed; look at the properties again")
    sel = selection[index]
    if not editor.editable:
        raise RuntimeError("the layout is read-only")
    if doc.foreign.get(owner(sel)):
        raise RuntimeError("this object is inside a cell of another view; open that view")
    return selection, index, sel


def _instance_path(sel, inst):
    """The selection path of `sel` with its last element made `inst` (after a change
    that may have given the instance another cell)."""
    new = pya.ObjectInstPath()
    new.cv_index = sel.cv_index
    new.top = sel.top
    for element in list(sel.path)[:-1]:
        new.append_path(element)
    new.append_path(pya.InstElement(inst))
    return new


def replace_cell(editor, params):
    """Make the selected instance place another cell, keeping its transformation:
    `params` = {index, key} and either {library, cell, values} (a cell or PCell of a
    KLayout library) or {target} (a cell index here, e.g. another view just loaded).
    One undoable step. Returns what `describe` returns now."""
    view = editor.require()
    ly = editor.doc.layout
    selection, index, sel = _chosen(editor, params)
    if not sel.is_cell_inst():
        raise ValueError("only an instance can place another cell")
    target = params.get("target")
    if target is not None:
        target = int(target)
        if not ly.is_valid_cell_index(target):
            raise ValueError("there is no such cell")

        def make():
            return target
    else:
        lib = _library(params["library"])
        name = params["cell"]
        decl = lib.layout().pcell_declaration(name)
        if decl is not None:
            defaults = {p.name: p.default for p in decl.get_parameters()}
            values = coerce(decl, ly, convert(decl, params.get("values"), defaults))
            pid = lib.layout().pcell_id(name)

            def make():
                return ly.add_pcell_variant(lib, pid, values)
        elif lib.layout().has_cell(name):
            lci = lib.layout().cell(name).cell_index()

            def make():
                return ly.add_lib_cell(lib, lci)
        else:
            raise ValueError("%s has no cell %s" % (lib.name(), name))
    inst = sel.inst()
    manager = editor.manager
    manager.transaction("Replace cell")
    try:
        inst.cell_index = make()
    finally:
        manager.commit()
    selection[index] = _instance_path(sel, inst)
    view.object_selection = selection
    return describe(editor)


def _layer_info(text):
    info = pya.LayerInfo.from_string(str(text))
    if info.layer < 0 and not info.name:
        raise ValueError("%s is not a layer" % text)
    return info


def _pts(points):
    return [pya.DPoint(float(x), float(y)) for x, y in points]


def _shape_change(ly, sel, obj):
    """A function that makes the change to a shape and returns its new selection path."""
    shape = sel.shape
    kind = obj.get("kind")
    if kind == "box" and shape.is_box():
        l, b, r, t = (float(v) for v in obj["bbox"])
        box = pya.DBox(min(l, r), min(b, t), max(l, r), max(b, t))

        def edit():
            shape.dbox = box
    elif kind == "path" and shape.is_path():
        points = _pts(obj.get("points") or [])
        if not points:
            raise ValueError("a path needs at least one point")
        width = float(obj["width"])
        if width < 0:
            raise ValueError("the width of a path cannot be negative")
        path = pya.DPath(points, width, float(obj.get("begin_ext", 0)),
                         float(obj.get("end_ext", 0)), bool(obj.get("round")))

        def edit():
            shape.dpath = path
    elif kind == "text" and shape.is_text():
        tr = pya.DTrans(int(obj.get("rotation", 0)) % 4, bool(obj.get("mirror")),
                        float(obj["x"]), float(obj["y"]))
        text = pya.DText(str(obj.get("string", "")), tr)
        text.size = float(obj.get("size", 0))
        text.halign = int(obj.get("halign", -1))
        text.valign = int(obj.get("valign", -1))

        def edit():
            shape.dtext = text
    elif kind == "polygon" and (shape.is_polygon() or shape.is_simple_polygon()):
        dx, dy = (float(v) for v in obj.get("offset") or [0, 0])

        def edit():
            if dx or dy:
                shape.transform(pya.DTrans(pya.DVector(dx, dy)))
    elif kind == "other":
        def edit():
            pass
    else:
        raise ValueError("the object is not a %s" % kind)
    layer = obj.get("layer")
    info = _layer_info(layer) if layer and str(ly.get_info(sel.layer)) != layer else None

    def change():
        edit()
        new = sel.dup()
        if info is not None:
            li = ly.find_layer(info)
            li = li if li is not None else ly.layer(info)
            shape.layer = li
            new.layer = li
        new.shape = shape
        return new
    return change


def _instance_change(ly, sel, obj, pcell):
    """A function that changes an instance and returns its new selection path."""
    inst = sel.inst()
    mag = float(obj.get("mag", 1.0))
    if mag <= 0:
        raise ValueError("the magnification must be positive")
    trans = pya.DCplxTrans(mag, float(obj.get("angle", 0.0)), bool(obj.get("mirror")),
                           float(obj["x"]), float(obj["y"]))
    array = obj.get("array")
    if array:
        na, nb = int(array["na"]), int(array["nb"])
        if na < 1 or nb < 1:
            raise ValueError("an array needs at least one column and one row")
        a, b = pya.DVector(*map(float, array["a"])), pya.DVector(*map(float, array["b"]))
        cell_inst = pya.DCellInstArray(inst.cell_index, trans, a, b, na, nb)
    else:
        cell_inst = pya.DCellInstArray(inst.cell_index, trans)
    values = None
    if pcell:
        if not inst.is_pcell():
            raise ValueError("%s is not a PCell" % inst.cell.name)
        values = changed_parameters(inst.pcell_declaration(), ly, inst.pcell_parameters_by_name(),
                                    pcell)

    def change():
        if str(inst.dcell_inst) != str(cell_inst):
            inst.dcell_inst = cell_inst
        if values is not None:
            inst.change_pcell_parameters(values)
        return _instance_path(sel, inst)
    return change
