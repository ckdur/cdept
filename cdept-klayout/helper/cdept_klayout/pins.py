# cdept-klayout: pins of a layout (docs/LINKING.md §9).
# SPDX-License-Identifier: GPL-3.0-or-later
"""Pins are CDePT objects: the layout's sidecar lists them (name, direction, net, shapes
on conductors, label). What a pin is made of in the stream depends on the technology,
which CDePT sends as `pin_rules` with the reference table:

  {"size": 0.1, "box": 0.5, "generate": "8/0",
   "layers": [{"drawing": "8/0", "pin": "8/2", "label": "8/25"}, {"drawing": "*", ...}]}

A pin on conductor D:
- with a pin layer P for D: its shape is on P, carrying the property PIN_PROPERTY (62)
  = the pin's name;
- without one: its shape is on the in-memory layer PIN_LAYER (never written, like the
  "cannot be found" markers), with PIN_PROPERTY and LAYER_PROPERTY (63) = "D";
- its name: a text with PIN_PROPERTY on D's label layer, else on PIN_LAYER (shown only).

So a pin is everything in a cell carrying PIN_PROPERTY: shapes move, stretch, delete
and undo like any other, and `collect` reads the pins back for the sidecar on save.
Shapes elsewhere that carry PIN_PROPERTY (tagged by other means) count as pin shapes
on their own layer.
"""

import pya

PIN_PROPERTY = 62
LAYER_PROPERTY = 63
PIN_LAYER = "CDEPT_PINS"
DIRECTIONS = ("input", "output", "inout")
_DIRECTION = {"i": "input", "in": "input", "ipin": "input", "input": "input",
              "o": "output", "out": "output", "opin": "output", "output": "output",
              "io": "inout", "iopin": "inout", "inout": "inout"}


def direction(value):
    """`input`, `output` or `inout` from xschem's i/o/io (ipin, opin, iopin) and the like."""
    out = _DIRECTION.get(str(value or "inout").strip().lower())
    if out is None:
        raise ValueError("direction %r is not one of %s" % (value, ", ".join(DIRECTIONS)))
    return out
# Gap between generated pins, µm.
GAP = 1.0


def _r(value):
    return round(float(value), 6)


def layer_name(info):
    return "%d/%d" % (info.layer, info.datatype)


def _info(text):
    return pya.LayerInfo.from_string(str(text))


class Rules:
    """The technology's `pin_rules`."""

    def __init__(self, rules):
        rules = rules or {}
        self.size = float(rules.get("size") or 0.1)
        self.box = float(rules.get("box") or 0.5)
        self.generate = rules.get("generate") or None
        self.layers = [l for l in (rules.get("layers") or []) if l.get("drawing")]
        self.known = bool(rules)

    def rule(self, drawing):
        """The rule for conductor `drawing` ("8/0"), or None."""
        for l in self.layers:
            if l["drawing"] == drawing:
                return l
        for l in self.layers:
            if l["drawing"] == "*":
                return l
        return None

    def pin_layers(self):
        """pin layer -> conductor."""
        return {l["pin"]: l["drawing"] for l in self.layers
                if l.get("pin") and l["drawing"] != "*"}

    def label_layers(self):
        return {l["label"] for l in self.layers if l.get("label")}

    def is_conductor(self, name):
        return any(l["drawing"] == name for l in self.layers)


class Pins:
    """The pins of a document: rules, what is not geometry (direction, net), the
    in-memory layer."""

    def __init__(self, doc, rules=None):
        self.doc = doc
        self.rules = Rules(rules)
        # file -> name -> {"direction", "net"}
        self.info = {}
        self._overlay = None
        self.fresh = False

    # -- layers ------------------------------------------------------------------

    def overlay(self):
        """PIN_LAYER, made when first needed (`fresh` tells the editor to show it)."""
        if self._overlay is None:
            self._overlay = self.doc.layout.layer(pya.LayerInfo(PIN_LAYER))
            self.fresh = True
        return self._overlay

    def overlay_index(self):
        return self._overlay

    def _layer(self, name):
        return self.doc.layout.layer(_info(name))

    # -- reading -------------------------------------------------------------------

    def collect(self, ci, file=None):
        """The pins of cell `ci`: {name: {direction, net, shapes, label, count, overlay}}.
        `count`: how many pin shapes carry the name (a copy makes two); `overlay`: some
        shape is CDePT's only (no pin layer)."""
        ly = self.doc.layout
        cell = ly.cell(ci)
        file = file or self.doc.root
        pins = {}
        pin_layers = self.rules.pin_layers()
        label_layers = self.rules.label_layers()
        overlay = self._overlay

        def pin(name):
            if name not in pins:
                known = (self.info.get(file) or {}).get(name) or {}
                pins[name] = {"direction": known.get("direction") or "inout",
                              "net": known.get("net"), "shapes": [], "label": None,
                              "count": 0, "overlay": False}
            return pins[name]

        for li in ly.layer_indexes():
            info = ly.get_info(li)
            here = layer_name(info) if overlay != li else None
            for shape in cell.shapes(li).each():
                value = shape.property(PIN_PROPERTY)
                if value is None or str(value) == "":
                    continue
                p = pin(str(value))
                if shape.is_text():
                    point = shape.text_dpos
                    # A label on a label layer wins over the one shown on PIN_LAYER.
                    if p["label"] is None or here in label_layers:
                        p["label"] = [_r(point.x), _r(point.y)]
                    continue
                if li == overlay:
                    drawing = shape.property(LAYER_PROPERTY)
                    if drawing is None:
                        continue
                    drawing = str(drawing)
                    p["overlay"] = True
                elif here in pin_layers:
                    drawing = pin_layers[here]
                else:
                    drawing = here
                p["shapes"].append(_shape_json(shape, drawing))
                p["count"] += 1
        return pins

    def records(self, ci, file=None):
        """`collect` as the sidecar keeps it (docs/LINKING.md §9)."""
        out = {}
        for name, p in self.collect(ci, file).items():
            if not p["shapes"]:
                continue
            record = {"direction": p["direction"], "shapes": p["shapes"]}
            if p["net"]:
                record["net"] = p["net"]
            if p["label"] is not None:
                record["label"] = p["label"]
            out[name] = record
        return out

    # -- making --------------------------------------------------------------------

    def realize(self, ci, file, records):
        """Make the pins of the sidecar `records` ({name: record}) in cell `ci` where the
        cell has no shape of that pin yet (the stream wins: it is what was edited).
        Remembers direction and net. Returns the names made."""
        have = self.collect(ci, file)
        made = []
        for name, record in sorted((records or {}).items()):
            self.remember(file, name, record.get("direction"), record.get("net"))
            if name in have and have[name]["shapes"]:
                continue
            for shape in record.get("shapes") or []:
                self.add_shape(ci, name, shape)
            point = record.get("label") or _middle(record.get("shapes") or [])
            if point is not None and record.get("shapes") and not (
                    name in have and have[name]["label"] is not None):
                self.add_label(ci, name, record["shapes"][0]["layer"], point)
            made.append(name)
        return made

    def remember(self, file, name, direction_=None, net=None):
        known = self.info.setdefault(file, {}).setdefault(name, {})
        if direction_:
            known["direction"] = direction(direction_)
        if net is not None:
            known["net"] = net or None

    def add_shape(self, ci, name, shape):
        """One shape of pin `name` ({layer, box | polygon}, µm) on conductor
        shape["layer"]: on its pin layer, else on PIN_LAYER."""
        ly = self.doc.layout
        drawing = shape["layer"]
        geometry = _geometry(shape)
        rule = self.rules.rule(drawing)
        if rule is not None and rule.get("pin"):
            new = ly.cell(ci).shapes(self._layer(rule["pin"])).insert(geometry)
        else:
            new = ly.cell(ci).shapes(self.overlay()).insert(geometry)
            new.set_property(LAYER_PROPERTY, drawing)
        new.set_property(PIN_PROPERTY, name)
        return new

    def add_label(self, ci, name, drawing, point):
        """The name of pin `name` as a text: on the conductor's label layer, else shown
        on PIN_LAYER only."""
        ly = self.doc.layout
        rule = self.rules.rule(drawing)
        li = self._layer(rule["label"]) if rule is not None and rule.get("label") else self.overlay()
        text = pya.DText(name, pya.DTrans(pya.DVector(float(point[0]), float(point[1]))))
        text.size = self.rules.size
        new = ly.cell(ci).shapes(li).insert(text)
        new.set_property(PIN_PROPERTY, name)
        return new

    def shapes_of(self, ci, name):
        """Every shape of cell `ci` that belongs to pin `name`: [(layer index, shape)]."""
        ly = self.doc.layout
        out = []
        for li in ly.layer_indexes():
            for shape in ly.cell(ci).shapes(li).each():
                value = shape.property(PIN_PROPERTY)
                if value is not None and str(value) == name:
                    out.append((li, shape))
        return out

    def remove(self, ci, name):
        shapes = self.shapes_of(ci, name)
        for _, shape in shapes:
            shape.delete()
        return len(shapes)

    def rename(self, ci, file, name, new):
        if not new or new == name:
            raise ValueError("give the pin a new name")
        if self.shapes_of(ci, new):
            raise ValueError("there is a pin %s already" % new)
        shapes = self.shapes_of(ci, name)
        if not shapes:
            raise ValueError("there is no pin %s" % name)
        for _, shape in shapes:
            shape.set_property(PIN_PROPERTY, new)
            if shape.is_text() and shape.text_string == name:
                shape.text_string = new
        known = (self.info.get(file) or {}).pop(name, None)
        if known:
            self.info.setdefault(file, {})[new] = known
        return len(shapes)


def _geometry(shape):
    if shape.get("box") is not None:
        b = [float(v) for v in shape["box"]]
        return pya.DBox(b[0], b[1], b[2], b[3])
    points = [pya.DPoint(float(x), float(y)) for x, y in shape.get("polygon") or []]
    if len(points) < 3:
        raise ValueError("a pin shape needs a box or a polygon of 3 points or more")
    return pya.DPolygon(points)


def _shape_json(shape, drawing):
    if shape.is_box():
        b = shape.dbox
        return {"layer": drawing, "box": [_r(b.left), _r(b.bottom), _r(b.right), _r(b.top)]}
    poly = shape.dpolygon if not shape.is_path() else shape.dpath.polygon()
    if poly.is_box():
        b = poly.bbox()
        return {"layer": drawing, "box": [_r(b.left), _r(b.bottom), _r(b.right), _r(b.top)]}
    return {"layer": drawing, "polygon": [[_r(p.x), _r(p.y)] for p in poly.each_point_hull()]}


def _middle(shapes):
    for shape in shapes:
        try:
            b = _geometry(shape).bbox()
        except ValueError:
            continue
        c = b.center()
        return [_r(c.x), _r(c.y)]
    return None


# -- the editor's side ----------------------------------------------------------------


def _top(editor):
    doc = editor.doc
    if doc is None:
        raise RuntimeError("no layout is open")
    top = doc.top_cell()
    if top is None:
        raise RuntimeError("the layout has no top cell")
    return doc, top


def _editable(editor):
    if not editor.editable:
        raise RuntimeError("the layout is read-only")


def listing(editor):
    """{pins: {name: {direction, net, shapes, label, count, overlay, label_on}}, rules}"""
    doc, top = _top(editor)
    pins = doc.pins.collect(top.cell_index())
    for p in pins.values():
        p["label_on"] = _label_on(p)
    return {"pins": pins, "rules": doc.pins.rules.known,
            "generate": doc.pins.rules.generate}


def _label_on(p):
    if p["label"] is None:
        return False
    x, y = p["label"]
    for s in p["shapes"]:
        b = _geometry(s).bbox()
        if b.left - 1e-9 <= x <= b.right + 1e-9 and b.bottom - 1e-9 <= y <= b.top + 1e-9:
            return True
    return False


def _transaction(editor, title, work):
    editor.manager.transaction(title)
    try:
        return work()
    finally:
        editor.manager.commit()


def add(editor, p):
    """A new pin: {name, direction?, net?, from_selection?} with {layer, box | polygon}
    or the selected shapes (each on a conductor or a pin layer). One undo step."""
    doc, top = _top(editor)
    _editable(editor)
    name = str(p.get("name") or "").strip()
    if not name:
        raise ValueError("a pin needs a name")
    ci = top.cell_index()
    if doc.pins.shapes_of(ci, name):
        raise ValueError("there is a pin %s already" % name)
    doc.pins.remember(doc.root, name, p.get("direction") or "inout", p.get("net"))
    if p.get("from_selection"):
        shapes = _selected_shapes(editor, top)
        if not shapes:
            raise ValueError("select a box or polygon of the cell on a conductor first")
    else:
        if not p.get("layer"):
            raise ValueError("a pin needs a layer")
        shapes = [("new", {"layer": p["layer"], "box": p.get("box"),
                           "polygon": p.get("polygon")})]

    def work():
        first = None
        pin_layers = doc.pins.rules.pin_layers()
        for what, shape in shapes:
            if what == "tag":
                # A shape on a pin layer already: it becomes the pin's.
                layer, sel_shape = shape
                sel_shape.set_property(PIN_PROPERTY, name)
                drawing = pin_layers[layer]
                json = _shape_json(sel_shape, drawing)
            else:
                json = shape
                doc.pins.add_shape(ci, name, json)
            if first is None:
                first = json
        point = p.get("label") or _middle([first])
        doc.pins.add_label(ci, name, first["layer"], point)
        return first

    _transaction(editor, "Add pin", work)
    editor.view.clear_object_selection()
    return listing(editor)


def _selected_shapes(editor, top):
    """The selected shapes of the top cell: [("tag", (pin layer, shape)) | ("new", json)]."""
    doc = editor.doc
    ly = doc.layout
    pin_layers = doc.pins.rules.pin_layers()
    out = []
    for sel in editor.view.each_object_selected():
        if sel.is_cell_inst() or sel.shape is None or list(sel.each_inst()):
            continue
        shape = sel.shape
        if shape.is_text() or sel.layer == doc.pins.overlay_index():
            continue
        layer = layer_name(ly.get_info(sel.layer))
        if layer in pin_layers:
            out.append(("tag", (layer, shape)))
        else:
            out.append(("new", _shape_json(shape, layer)))
    return out


def remove(editor, p):
    doc, top = _top(editor)
    _editable(editor)
    name = p.get("name") or ""
    count = _transaction(editor, "Delete pin",
                         lambda: doc.pins.remove(top.cell_index(), name))
    if not count:
        raise ValueError("there is no pin %s" % name)
    (doc.pins.info.get(doc.root) or {}).pop(name, None)
    return listing(editor)


def rename(editor, p):
    doc, top = _top(editor)
    _editable(editor)
    _transaction(editor, "Rename pin",
                 lambda: doc.pins.rename(top.cell_index(), doc.root, p.get("name") or "",
                                         str(p.get("new") or "").strip()))
    return listing(editor)


def update(editor, p):
    """Change a pin: {name, direction?, net?, shapes?: [{layer, box|polygon}], label?:
    [x, y] or "middle"}. New shapes replace the pin's shapes (its label stays unless
    given). One undo step."""
    doc, top = _top(editor)
    _editable(editor)
    name = p.get("name") or ""
    ci = top.cell_index()
    have = doc.pins.collect(ci).get(name)
    if have is None:
        raise ValueError("there is no pin %s" % name)
    doc.pins.remember(doc.root, name, p.get("direction"), p.get("net"))

    def work():
        if p.get("shapes") is not None:
            for li, shape in doc.pins.shapes_of(ci, name):
                if not shape.is_text():
                    shape.delete()
            for shape in p["shapes"]:
                doc.pins.add_shape(ci, name, shape)
        label = p.get("label")
        if label is not None:
            shapes = p.get("shapes") or have["shapes"]
            point = _middle(shapes) if label == "middle" else label
            texts = [s for _, s in doc.pins.shapes_of(ci, name) if s.is_text()]
            if texts:
                for text in texts:
                    t = text.dtext
                    t.trans = pya.DTrans(t.trans.rot, t.trans.is_mirror(),
                                         pya.DVector(float(point[0]), float(point[1])))
                    text.dtext = t
            elif shapes:
                doc.pins.add_label(ci, name, shapes[0]["layer"], point)

    if p.get("shapes") is not None or p.get("label") is not None:
        _transaction(editor, "Change pin", work)
    return listing(editor)


def select(editor, p):
    """Select the shapes of these pins and (zoom) show them. Returns the names found."""
    doc, top = _top(editor)
    view = editor.require()
    paths, found, box = [], [], pya.DBox()
    ci = top.cell_index()
    dbu = doc.layout.dbu
    for name in p.get("names") or []:
        shapes = doc.pins.shapes_of(ci, name)
        if shapes:
            found.append(name)
        for li, shape in shapes:
            path = pya.ObjectInstPath()
            path.top = ci
            path.cv_index = 0
            path.layer = li
            path.shape = shape
            paths.append(path)
            box += shape.bbox().to_dtype(dbu)
    view.object_selection = paths
    if p.get("zoom") and not box.empty():
        margin = max(1.0, 0.5 * max(box.width(), box.height()))
        view.zoom_box(box.enlarged(margin, margin))
    return {"found": found}


def generate(doc, top, wanted, legacy_layer=None):
    """Pins for the schematic's pins `wanted` ([{name, direction, net?}]) that the top
    cell has no pin for: a pin of the same name made of a text from before pins were
    objects (a label without PIN_PROPERTY) is taken over; else a square of the rules'
    size on the `generate` conductor, in a column left of the cell. Returns the names
    made or taken over."""
    ci = top.cell_index()
    ly = doc.layout
    pins = doc.pins
    have = pins.collect(ci)
    made = []
    rules = pins.rules
    label_to_drawing = {l["label"]: l["drawing"] for l in rules.layers if l.get("label")}
    if legacy_layer and not rules.layers:
        # A device map of before (labels only): keep making labels there.
        label_to_drawing = {legacy_layer: None}
    texts = {}
    for li in ly.layer_indexes():
        name = layer_name(ly.get_info(li))
        if name not in label_to_drawing:
            continue
        for shape in top.shapes(li).each(pya.Shapes.STexts):
            if shape.property(PIN_PROPERTY) is None:
                texts.setdefault(shape.text_string, (shape, name))
    box = top.dbbox()
    x = (0.0 if box.empty() else box.left) - 2 * GAP - rules.box
    y = (0.0 if box.empty() else box.top) - rules.box
    half = rules.box / 2.0
    for want in wanted:
        name = want.get("name") or ""
        if not name:
            continue
        pins.remember(doc.root, name, want.get("direction") or "inout", want.get("net"))
        if name in have and have[name]["shapes"]:
            continue
        old = texts.get(name)
        if old is not None:
            shape, label_layer = old
            drawing = label_to_drawing.get(label_layer) or rules.generate
            point = shape.text_dpos
            if drawing is None:
                continue  # labels only, and this one is there
            shape.set_property(PIN_PROPERTY, name)
            pins.add_shape(ci, name, {"layer": drawing,
                                      "box": [point.x - half, point.y - half,
                                              point.x + half, point.y + half]})
            made.append(name)
            continue
        if rules.generate is None:
            if legacy_layer:
                text = pya.DText(name, pya.DTrans(pya.DVector(x, y)))
                text.size = rules.size
                top.shapes(ly.layer(_info(legacy_layer))).insert(text)
                made.append(name)
                y -= max(4 * rules.size, 1.0)
            continue
        shape = {"layer": rules.generate, "box": [x, y, x + rules.box, y + rules.box]}
        pins.add_shape(ci, name, shape)
        pins.add_label(ci, name, rules.generate, [x + half, y + half])
        made.append(name)
        y -= rules.box + GAP
    return made


class Tool:
    """Draw Pin: the next box drawn in box mode becomes pin `name` (the box itself is
    removed: the pin goes on the conductor's pin layer, or is CDePT's)."""

    def __init__(self, editor):
        self.editor = editor
        self.armed = None  # {name, direction, net, layer index, before: set}

    def arm(self, p):
        editor = self.editor
        doc, top = _top(editor)
        _editable(editor)
        name = str(p.get("name") or "").strip()
        if not p.get("on", True):
            self.armed = None
            return {"armed": False}
        if not name:
            raise ValueError("a pin needs a name")
        if doc.pins.shapes_of(top.cell_index(), name):
            raise ValueError("there is a pin %s already" % name)
        view = editor.require()
        it = view.current_layer
        if it.is_null() or it.current().layer_index() < 0:
            raise ValueError("choose the conductor (layer) to draw the pin on first")
        li = it.current().layer_index()
        if li == doc.pins.overlay_index():
            raise ValueError("choose a conductor, not the pins layer")
        self.armed = {"name": name, "direction": p.get("direction") or "inout",
                      "net": p.get("net"), "layer": li, "cell": top.cell_index(),
                      "before": self._boxes(top.cell_index(), li)}
        view.switch_mode("box")
        return {"armed": True, "layer": layer_name(doc.layout.get_info(li))}

    def _boxes(self, ci, li):
        cell = self.editor.doc.layout.cell(ci)
        return {s.box for s in cell.shapes(li).each(pya.Shapes.SBoxes)}

    def changed(self):
        """After a change: a new box on the armed layer is made the pin. True when it
        changed the layout."""
        a = self.armed
        editor = self.editor
        if a is None or editor.doc is None:
            return False
        ly = editor.doc.layout
        if not ly.is_valid_cell_index(a["cell"]):
            self.armed = None
            return False
        cell = ly.cell(a["cell"])
        new = [s for s in cell.shapes(a["layer"]).each(pya.Shapes.SBoxes)
               if s.box not in a["before"]]
        if not new:
            return False
        self.armed = None
        shape = new[0]
        layer = layer_name(ly.get_info(a["layer"]))
        pin_layers = editor.doc.pins.rules.pin_layers()
        b = shape.dbox
        json = {"layer": pin_layers.get(layer, layer),
                "box": [_r(b.left), _r(b.bottom), _r(b.right), _r(b.top)]}
        pins = editor.doc.pins
        pins.remember(editor.doc.root, a["name"], a["direction"], a["net"])

        def work():
            if layer in pin_layers:
                shape.set_property(PIN_PROPERTY, a["name"])
            else:
                shape.delete()
                pins.add_shape(a["cell"], a["name"], json)
            pins.add_label(a["cell"], a["name"], json["layer"], _middle([json]))

        _transaction(editor, "Draw pin", work)
        editor.view.switch_mode("select")
        editor.notice("info", "pin %s placed" % a["name"])
        return True


DEFAULT_STYLE = {"color": "#ff8000"}


def style(editor, p):
    """How PIN_LAYER (pins that are CDePT's only, and their names) is shown:
    {color: "#rrggbb", visible?}. Kept for the next layout opened."""
    editor.pin_style = dict(editor.pin_style or DEFAULT_STYLE, **{k: v for k, v in p.items() if v is not None})
    style_view(editor.require(), editor.pin_style)
    return {}


def style_view(view, p):
    it = view.begin_layers()
    while not it.at_end():
        node = it.current()
        if node.source.startswith(PIN_LAYER):
            color = p.get("color")
            if color:
                value = int(str(color).lstrip("#"), 16)
                node.fill_color = value
                node.frame_color = value
            node.dither_pattern = 1  # hollow
            node.width = 2
            node.name = "pins (CDePT)"
            if p.get("visible") is not None:
                node.visible = bool(p["visible"])
            view.set_layer_properties(it, node)
        it.next()
    return {}
