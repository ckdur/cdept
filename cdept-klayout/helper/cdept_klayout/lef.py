# cdept-klayout: LEF abstracts, read-only (docs/KLAYOUT.md §9).
# SPDX-License-Identifier: GPL-3.0-or-later
"""A LEF file shown as a layout.

The LEF stays the view's file: it is read with KLayout's LEF/DEF reader into the
layout in memory, and nothing is written. The technology LEF (layers, units) is read
first. Two ways to number the layers:

* the technology's own LEF/DEF reader setting, when it names a layer map (IHP
  `sg13g2.map`): layers get the PDK's GDS numbers, so the PDK's layer properties
  (colours) apply;
* otherwise a generic mapping by KLayout's defaults: layers numbered in the order the
  technology LEF and the file name them, pins / labels / obstructions on datatype
  2 / 1 / 3 of their layer, the macro boundary on layer 1. Colours come from the
  technology's layer properties by layer name when it has them (`li1.drawing`), else
  from a fixed palette.

Pins, obstructions, labels and the macro boundary are separate layers either way, so
the layers panel can switch each of them.
"""

import os
import re

import pya

from .document import Document

READ_ONLY = "LEF abstract: read-only"

# Colours (0xRRGGBB) for layers the technology gives none for, by order of appearance.
PALETTE = [0x4f81bd, 0xc0504d, 0x9bbb59, 0x8064a2, 0x4bacc6, 0xf79646,
           0x2c4d75, 0x772c2a, 0x5f7530, 0x4d3b62, 0x276a7c, 0xb65708]
OUTLINE_COLOR = 0xe0e0e0

# KLayout's built-in dither pattern indexes.
SOLID, HOLLOW = 0, 1
HATCHED = 5


def is_lef(path):
    return str(path).lower().endswith(".lef")


class LefDocument(Document):
    """A LEF file read as a layout. Read-only: it has no sidecar, no references, no
    pins of its own, and can neither be saved nor exported."""

    abstract = True

    def __init__(self, layout, table, technology=None, layer_properties=None):
        super().__init__(layout, table)
        self.technology = technology
        self.layer_properties = layer_properties
        # "technology": the technology's layer map; "generic": KLayout's defaults.
        self.mapping = "generic"

    def load(self):
        options, self.mapping, self.layer_properties = reader_options(
            self.technology, self.table.lef.get("tech_lef"), self.layer_properties)
        self.layout.read(self.root, options)
        if self.mapping == "generic":
            self.notes.append("LEF: layers numbered by the order the LEF names them "
                              "(the technology has no LEF/DEF layer map)")
        return self

    def realize_pins(self):
        pass

    def save(self, file=None, also=()):
        raise RuntimeError(READ_ONLY)

    def export(self, out):
        raise RuntimeError(READ_ONLY)


def open_document(layout, table, technology=None, layer_properties=None):
    """The document of reference table `table`: a LEF abstract or a layout file."""
    if table.lef is not None:
        return LefDocument(layout, table, technology, layer_properties)
    return Document(layout, table)


# -- reading ---------------------------------------------------------------------


def find_technology(technology):
    """The technology `technology` names (a name or a .lyt file), or None."""
    if not technology:
        return None
    try:
        if os.path.isfile(technology):
            tech = pya.Technology()
            tech.load(technology)
            return tech
        return pya.Technology.technology_by_name(technology)
    except Exception:
        return None


def technology_layer_properties(tech):
    """The technology's .lyp file, when it names one that exists."""
    if tech is None:
        return None
    name = tech.layer_properties_file
    if not name:
        return None
    for path in (name, os.path.join(tech.base_path() or "", name)):
        if os.path.isfile(path):
            return path
    return None


def reader_options(technology, tech_lef, layer_properties):
    """(LoadLayoutOptions, "technology" | "generic", the .lyp to colour with)."""
    tech = find_technology(technology)
    mapping = "generic"
    config = pya.LEFDEFReaderConfiguration()
    options = pya.LoadLayoutOptions()
    if tech is not None:
        tech_options = tech.load_layout_options
        tech_config = tech_options.lefdef_config
        map_file = tech_config.map_file
        if map_file:
            if not os.path.isabs(map_file):
                map_file = os.path.join(tech.base_path() or "", map_file)
            if os.path.isfile(map_file):
                options, config = tech_options, tech_config
                config.map_file = map_file
                mapping = "technology"
    config.lef_files = [tech_lef] if tech_lef and os.path.isfile(tech_lef) else []
    config.produce_pins = True
    config.produce_lef_pins = True
    config.produce_obstructions = True
    config.produce_labels = True
    config.produce_lef_labels = True
    config.produce_cell_outlines = True
    config.create_other_layers = True
    options.lefdef_config = config
    return options, mapping, layer_properties or technology_layer_properties(tech)


# -- naming and colours --------------------------------------------------------------


def classify(name):
    """(base layer, purpose) of a layer name the LEF/DEF reader made: `Metal1.PIN`,
    `li1.OBS`, `Metal1.PIN/LEFPIN`, `Metal1.NET/PIN/SPNET/...`, `OUTLINE`.
    purpose: pin, obs, label, routing, fill, outline or the word itself (lower case)."""
    name = (name or "").strip().strip("'")
    if not name:
        return None, None
    if name.upper() == "OUTLINE":
        return "", "outline"
    base, _, rest = name.partition(".")
    tokens = {t.strip().upper() for t in rest.split("/") if t.strip() and t != "..."}
    if not tokens:
        return base, "drawing"
    if "LEFPIN" in tokens:
        return base, "pin"
    if tokens & {"LEFOBS", "OBS", "BLOCKAGE", "BLOCKAGES"}:
        return base, "obs"
    if "LABEL" in tokens:
        return base, "label"
    if "NET" in tokens:
        return base, "routing"
    if "PIN" in tokens:
        return base, "pin"
    if "FILL" in tokens:
        return base, "fill"
    return base, rest.split("/")[0].lower()


def display_name(base, purpose):
    if purpose == "outline":
        return "macro boundary"
    if purpose == "routing":
        return base
    return "%s %s" % (base, {"obs": "obstruction"}.get(purpose, purpose))


def _lyp_colours(path):
    """{layer name without its ' - n/m' tail: (fill, frame)} of a .lyp file."""
    colours = {}
    if not path or not os.path.isfile(path):
        return colours
    try:
        view = pya.LayoutView()
        view.load_layer_props(path)
        it = view.begin_layers()
        while not it.at_end():
            node = it.current()
            if node.name and not node.has_children():
                colours.setdefault(re.sub(r"\s+-\s+\d+/\d+$", "", node.name),
                                   (node.fill_color, node.frame_color))
            it.next()
    except Exception:
        pass
    return colours


def _key(source):
    """`layer/datatype` of a node's source (`8/2@1`, or `Metal1.PIN 2/2@1` for a layer
    that has a name)."""
    found = re.search(r"(\d+)/(\d+)(@|$)", source)
    return "%s/%s" % (found.group(1), found.group(2)) if found else source


def style(view, layout, mapping, layer_properties):
    """Names, colours and fill patterns of the layers of a LEF abstract: from the
    technology's layer properties where it has them, else generated."""
    from_lyp = set()
    if mapping == "technology" and layer_properties and os.path.isfile(layer_properties):
        view.load_layer_props(layer_properties, True)
        view.remove_unused_layers()
        it = view.begin_layers()
        while not it.at_end():
            from_lyp.add(_key(it.current().source))
            it.next()
    view.add_missing_layers()
    _drop_empty(view, layout)
    named = _lyp_colours(layer_properties) if mapping != "technology" else {}
    # The reader's name of each layer, by layer/datatype.
    info = {}
    for li in layout.layer_indexes():
        i = layout.get_info(li)
        info["%d/%d" % (i.layer, i.datatype)] = i.name
    order = {}
    it = view.begin_layers()
    while not it.at_end():
        node = it.current()
        key = _key(node.source)
        base, purpose = classify(info.get(key, ""))
        if purpose is not None:
            if purpose == "outline":
                node.fill_color = node.frame_color = OUTLINE_COLOR
                node.dither_pattern = HOLLOW
                node.width = 2
            else:
                if key not in from_lyp:
                    fill, frame = _colour(base, named, order)
                    node.fill_color, node.frame_color = fill, frame
                    node.dither_pattern = SOLID
                if purpose == "obs":
                    # Not the pattern the technology gives its pins.
                    node.dither_pattern = HOLLOW if key in from_lyp else HATCHED
                elif purpose == "label":
                    node.dither_pattern = HOLLOW
            node.name = display_name(base, purpose)
            view.set_layer_properties(it, node)
        it.next()


def _drop_empty(view, layout):
    """Take layers nothing is drawn on out of the layers panel (a technology's layer
    properties list hundreds; the LEF reader declares layers for all purposes)."""
    used = set()
    for li in layout.layer_indexes():
        if any(not cell.shapes(li).is_empty() for cell in layout.each_cell()):
            info = layout.get_info(li)
            used.add("%d/%d" % (info.layer, info.datatype))
    empty = []
    it = view.begin_layers()
    while not it.at_end():
        if _key(it.current().source) not in used:
            empty.append(it.current().source)
        it.next()
    for source in empty:
        it = view.begin_layers()
        while not it.at_end():
            if it.current().source == source:
                view.delete_layer(it)
                break
            it.next()


def _colour(base, named, order):
    """(fill, frame) for layers of `base`: the technology's colour of its drawing layer,
    else the palette's next colour (the same for all layers of a base)."""
    for key in (base + ".drawing", base):
        if key in named:
            return named[key]
    if base not in order:
        order[base] = PALETTE[len(order) % len(PALETTE)]
    return order[base], order[base]
