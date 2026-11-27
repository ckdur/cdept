# What a tutorial of CDePT asks about the layout as it is now, unsaved edits included
# (cdept-gui/src/views/layout/guide.rs). Only the cell shown at the top is looked at.
# SPDX-License-Identifier: GPL-3.0-or-later
"""The facts of a tutorial about the layout:

  layout.at:<cell>:<x>,<y>[:<param>=<value>,...]
      an instance of <cell> (a PCell by its name, whatever its variant) sits at x, y (µm),
      with these PCell parameters
  layout.covers:<layer>/<datatype>:<l>,<b>,<r>,<t>
      the cell's own shapes on the layer (boxes, polygons, paths: not what placed cells
      bring) cover the whole box
  layout.pin:<name>:<layer>/<datatype>:<l>,<b>,<r>,<t>
      pin <name> has a shape inside the box, wholly on metal of that layer (the cell's
      own or a placed cell's)
"""

import pya

from . import pins

# How far (µm) an instance may be from the place asked for.
NEAR = 0.0011


def _numbers(text):
    return [float(v) for v in text.split(",")]


def _layer(layout, text):
    layer, datatype = (int(v) for v in text.split("/"))
    return layout.find_layer(pya.LayerInfo(layer, datatype))


def _box(layout, numbers):
    return pya.Region(pya.DBox(*numbers).to_itype(layout.dbu))


def _same(value, wanted):
    if isinstance(value, bool):
        return str(value).lower() == wanted.lower()
    if isinstance(value, (int, float)):
        try:
            return abs(value - float(wanted)) < 1e-9
        except ValueError:
            return False
    return str(value) == wanted


def _at(layout, top, rest):
    cell, place = rest[0], _numbers(rest[1])
    wanted = [p.split("=", 1) for p in rest[2].split(",")] if len(rest) > 2 else []
    for inst in top.each_inst():
        child = layout.cell(inst.cell_index)
        name = child.basic_name() if child.is_pcell_variant() else child.name
        if name.split("$")[0] != cell:
            continue
        at = inst.dcplx_trans.disp
        if abs(at.x - place[0]) > NEAR or abs(at.y - place[1]) > NEAR:
            continue
        values = inst.pcell_parameters_by_name() if inst.is_pcell() else {}
        if all(_same(values.get(k), v) for k, v in wanted):
            return True
    return False


def _covers(layout, top, rest):
    li = _layer(layout, rest[0])
    if li is None:
        return False
    drawn = pya.Region(top.shapes(li))  # texts are left out by Region
    return (_box(layout, _numbers(rest[1])) - drawn).is_empty()


def _pin(editor, layout, top, rest):
    name, layer_text, area = rest[0], rest[1], _numbers(rest[2])
    pin = pins.listing(editor)["pins"].get(name)
    li = _layer(layout, layer_text)
    if pin is None or li is None:
        return False
    metal = pya.Region(top.begin_shapes_rec(li))
    allowed = _box(layout, area)
    for shape in pin.get("shapes") or []:
        if shape.get("box") is None:
            continue
        box = _box(layout, shape["box"])
        if (box - allowed).is_empty() and (box - metal).is_empty():
            return True
    return False


def probe(editor, queries):
    """One true or false per query; false for a query this helper does not know."""
    layout = editor.doc.layout
    top = editor.doc.top_cell()
    answers = []
    for query in queries:
        kind, _, text = query.partition(":")
        rest = text.split(":")
        try:
            if top is None:
                answers.append(False)
            elif kind == "layout.at":
                answers.append(_at(layout, top, rest))
            elif kind == "layout.covers":
                answers.append(_covers(layout, top, rest))
            elif kind == "layout.pin":
                answers.append(_pin(editor, layout, top, rest))
            else:
                answers.append(False)
        except (ValueError, IndexError):
            answers.append(False)
    return answers
