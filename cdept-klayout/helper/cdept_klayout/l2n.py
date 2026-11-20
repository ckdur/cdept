# cdept-klayout: the connectivity of a layout, from an LVS database.
# SPDX-License-Identifier: GPL-3.0-or-later
"""The layout side of a KLayout LVS database (.lvsdb, as a PDK's LVS deck writes it),
for CDePT's net comparison (libcdept::link::nets, docs/LINKING.md §8). The comparison
is CDePT's; this says what is connected in the layout, and where.

`Connectivity(lvsdb, top)` loads the database and flattens it (without changing it):
each device counts where it is placed, each net of the top circuit is a whole net, and
nets inside placed cells that do not leave them are nets of their own. The batch script
(`connectivity.py`) and the editor (`nets.py`) load the same database the same way and
so agree on the net ids.

`describe(stream)` gives the JSON CDePT reads (µm, top cell coordinates):
  {"top": name,
   "nets": [{"id", "name", "bbox"}],            name "" for a net without a label
   "devices": [{"name", "class", "bbox",
                "terminals": [{"name", "net", "point"}]}],
   "instances": [{"path", "cell", "bbox", "leaf"}],
   "labels": [{"text", "point", "layer", "net"}]}
`instances` are the instances bound to schematic instances (GDS property 61), with
their paths through bound cells (`x1/M1`), from the stream the database was made from.
`labels` are the texts of the top cell itself (not of the cells it places) with the
net each one names (null: on no shape of a net of that name). Net names alone do not
tell: a label inside a placed cell names a net of the top circuit too.
"""

import pya

NAME_PROPERTY = 61


def _box_um(box, dbu):
    if box is None or box.empty():
        return None
    return [round(box.left * dbu, 6), round(box.bottom * dbu, 6),
            round(box.right * dbu, 6), round(box.top * dbu, 6)]


def _name(obj):
    value = obj.expanded_name() if hasattr(obj, "expanded_name") else obj.name
    value = str(value or "")
    # KLayout's own names for nets without a label.
    return "" if value.startswith("$") else value


class Entry:
    """A net of the flattened layout: a net of some circuit, placed by `trans`."""

    def __init__(self, net, trans):
        self.net = net
        self.trans = trans


class Connectivity:
    """The layout's nets, flattened into the top circuit.

    The hierarchy is walked from the top circuit: a net of a placed circuit that
    leaves it through a pin is the parent's net; one that does not is a net of its
    own, placed where its circuit is. Net ids: the top circuit's nets in their order,
    then those inner nets in the order of the walk (depth first)."""

    def __init__(self, lvsdb, top=None):
        self.lvs = pya.LayoutVsSchematic()
        self.lvs.read(lvsdb)
        netlist = self.lvs.netlist()
        circuit = netlist.circuit_by_name(top) if top else None
        if circuit is None:
            circuits = list(netlist.each_circuit_top_down())
            if not circuits:
                raise ValueError("%s holds no circuit" % lvsdb)
            circuit = circuits[0]
        self.circuit = circuit
        self.dbu = self.lvs.internal_layout().dbu
        self.layers = [self.lvs.layer_by_name(n) for n in self.lvs.layer_names()]
        self.entries = []
        # (devices with their terminals' ids and shapes) found by the walk
        self.devices = []
        self._walk(circuit, pya.ICplxTrans(), None, 0)
        self.nets = [e.net for e in self.entries]
        self._boxes = None

    def _walk(self, circuit, trans, outer, depth):
        """`outer`: cluster id → net id for the nets of `circuit` that are a parent's
        net (None at the top)."""
        ids = {}
        for net in circuit.each_net():
            known = outer.get(net.cluster_id) if outer else None
            if known is None:
                known = len(self.entries)
                self.entries.append(Entry(net, trans))
            ids[net.cluster_id] = known
        for device in circuit.each_device():
            self.devices.append((device, trans, ids))
        if depth > 64:
            return
        for sub in circuit.each_subcircuit():
            child = sub.circuit_ref()
            mapping = {}
            for pin in child.each_pin():
                inner = child.net_for_pin(pin.id())
                parent = sub.net_for_pin(pin.id())
                if inner is not None and parent is not None:
                    mapping[inner.cluster_id] = ids.get(parent.cluster_id)
            self._walk(child, trans * sub.trans.to_itrans(self.dbu), mapping, depth + 1)

    # -- geometry ---------------------------------------------------------------

    def shapes(self, net_or_id):
        """The shapes of a net (an id, or a net of the top circuit): [(layer number,
        Region in database units, top cell coordinates)]."""
        if isinstance(net_or_id, int):
            entry = self.entries[net_or_id]
        else:
            entry = next((e for e in self.entries if e.net is net_or_id), None) or \
                Entry(net_or_id, pya.ICplxTrans())
        out = []
        for i, region in enumerate(self.layers):
            shapes = self.lvs.shapes_of_net(entry.net, region, True)
            if not shapes.is_empty():
                if not entry.trans.is_unity():
                    shapes = shapes.transformed(entry.trans)
                out.append((i, shapes))
        return out

    def net_box(self, index):
        box = pya.Box()
        for _, region in self.shapes(index):
            box += region.bbox()
        return box

    def boxes(self):
        if self._boxes is None:
            self._boxes = [self.net_box(i) for i in range(len(self.entries))]
        return self._boxes

    def net_at(self, point):
        """The id of the net with a shape at `point` (µm), or None. When shapes of
        several nets are there, the one on the layer listed last (upper layers come
        later in the PDK decks) wins."""
        p = pya.Point(int(round(point[0] / self.dbu)), int(round(point[1] / self.dbu)))
        probe = pya.Region(pya.Box(p.x - 1, p.y - 1, p.x + 1, p.y + 1))
        best = None
        for index, box in enumerate(self.boxes()):
            if box.empty() or not box.enlarged(1, 1).contains(p):
                continue
            for layer, region in self.shapes(index):
                if not region.interacting(probe).is_empty():
                    if best is None or layer > best[0]:
                        best = (layer, index)
        return None if best is None else best[1]

    # -- description ------------------------------------------------------------

    def describe(self, stream=None):
        nets = [{"id": i, "name": _name(net), "bbox": _box_um(box, self.dbu)}
                for i, (net, box) in enumerate(zip(self.nets, self.boxes()))]
        devices = [self._device(d, trans, ids) for d, trans, ids in self.devices]
        out = {"top": self.circuit.name, "nets": nets, "devices": devices}
        if stream:
            out["instances"] = bound_instances(stream, self.circuit.name)
            out["labels"] = self.labels(stream)
        return out

    def labels(self, stream):
        """The texts of the top cell of `stream`, each with the net it names."""
        ly = pya.Layout()
        ly.read(stream)
        cell = ly.cell(self.circuit.name)
        out = []
        if cell is None:
            return out
        seen = set()
        for index in ly.layer_indexes():
            for shape in cell.shapes(index).each(pya.Shapes.STexts):
                text = shape.text_string
                p = shape.text_dpos
                key = (text, round(p.x, 6), round(p.y, 6))
                if key in seen:
                    continue
                seen.add(key)
                info = ly.get_info(index)
                out.append({"text": text, "point": [round(p.x, 6), round(p.y, 6)],
                            "layer": "%d/%d" % (info.layer, info.datatype),
                            "net": self._named_at(text, (p.x, p.y))})
        return out

    def _named_at(self, name, point):
        """The net called `name` with a shape at `point` (µm)."""
        p = pya.Point(int(round(point[0] / self.dbu)), int(round(point[1] / self.dbu)))
        probe = pya.Region(pya.Box(p.x - 1, p.y - 1, p.x + 1, p.y + 1))
        for i, net in enumerate(self.nets):
            if _name(net) != name:
                continue
            for _, region in self.shapes(i):
                if not region.interacting(probe).is_empty():
                    return i
        return None

    def _device(self, device, trans, ids):
        terminals = []
        box = pya.Box()
        for definition in device.device_class().terminal_definitions():
            net = device.net_for_terminal(definition.id())
            point = None
            if net is not None:
                for ref in net.each_terminal():
                    if ref.device().id() == device.id() and \
                            ref.terminal_id() == definition.id():
                        tbox = pya.Box()
                        for region in self.lvs.shapes_of_terminal(ref, trans).values():
                            if not region.is_empty():
                                tbox += region.bbox()
                        if not tbox.empty():
                            c = tbox.center()
                            point = [round(c.x * self.dbu, 6), round(c.y * self.dbu, 6)]
                            box += tbox
                        break
            terminals.append({"name": definition.name,
                              "net": ids.get(net.cluster_id) if net is not None else None,
                              "point": point})
        return {"name": str(device.expanded_name()), "class": device.device_class().name,
                "bbox": _box_um(box, self.dbu), "terminals": terminals}


def bound_instances(stream, top):
    """The instances bound to schematic instances (GDS property 61), through the bound
    cells below `top`: [{path, cell, bbox (µm, top cell), leaf}]. `leaf`: the cell is
    a library cell or PCell, or holds no bound instances itself."""
    ly = pya.Layout()
    ly.read(stream)
    cell = ly.cell(top)
    if cell is None:
        raise ValueError("%s has no cell %s" % (stream, top))
    out = []

    def walk(cell, prefix, trans):
        for inst in cell.each_inst():
            value = inst.property(NAME_PROPERTY)
            if value is None:
                continue
            path = prefix + str(value)
            child = inst.cell
            nested = not child.is_proxy() and any(
                i.property(NAME_PROPERTY) is not None for i in child.each_inst())
            box = inst.bbox().transformed(trans)
            out.append({"path": path, "cell": child.name,
                        "bbox": _box_um(box, ly.dbu), "leaf": not nested})
            if nested and not inst.is_regular_array():
                walk(child, path + "/", trans * inst.cplx_trans)

    walk(cell, "", pya.ICplxTrans())
    return out
