# cdept-klayout: live connectivity between the terminals of the last extraction.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Flylines that follow the edits (docs/LINKING.md §8), without a new extraction.

Update Connectivity extracts the layout with the PDK's LVS deck: devices, their
terminals, the nets. After that, wiring changes far more often than devices do, so
between extractions this module keeps the devices as extracted and traces the
**wiring** on the live layout with KLayout's net tracer (`pya.NetTracer`), using the
technology's net tracer connectivity (the `<connectivity>` of the `.lyt`: which layers
connect through which vias):

- every device terminal and every pin label of the extraction is a *seed*: a point
  and the conductor layer it starts on (the first that has a shape there);
- *calibration* (right after the extraction, the layout as extracted): seeds traced
  into one net must have been on one net of the extraction; a seed that disagrees with
  most of its net (a MOS bulk, whose terminal sits in the channel, traces as the gate)
  is not traced but kept on its extracted net (connections through wells, which the
  net tracer does not know);
- after each change (debounced), the seeds are traced again: their nets now. A seed of
  a device inside a top-level instance moves with that instance; devices of instances
  deleted since are gone; instances added since are listed (their devices wait for
  the next extraction).

The result goes to CDePT as a `nets_live` event; CDePT compares it with the schematic
again (libcdept::link::nets::Connectivity::with_live) and sends the new flylines.
"""

import json
import re
import time

import pya

NAME_PROPERTY = 61
LAYER_SPEC = re.compile(r"^\s*(\d+)\s*/\s*(\d+)\s*$")
TOKEN = re.compile(r"[A-Za-z_][A-Za-z0-9_.]*|\d+/\d+")


def tech_connectivity(technology):
    """The first net tracer connectivity of a technology, or None."""
    if not technology:
        return None
    tech = pya.Technology.technology_by_name(technology)
    if tech is None:
        return None
    component = tech.component("connectivity")
    if component is None:
        return None
    for stack in component.each():
        if any(True for _ in stack.each_connection()):
            return stack
    return None


def make_connectivity(rules):
    """A connectivity from {connections: [[a, via, b] | [a, b]], symbols: {name: expr}}
    (tests, technologies without one)."""
    conn = pya.NetTracerConnectivity()
    for name, expression in (rules.get("symbols") or {}).items():
        conn.symbol(name, expression)
    for c in rules.get("connections") or []:
        if len(c) == 3:
            conn.connection(c[0], c[1], c[2])
        else:
            conn.connection(c[0], c[1])
    return conn


def conductors(conn):
    """The conductor layers of a connectivity, in order (vias left out), each as the
    original layer a trace may start on: [(name, (layer, datatype))]."""
    symbols = {s.symbol(): s.expression() for s in conn.each_symbol()}

    def first_layer(name, depth=0):
        if depth > 16:
            return None
        m = LAYER_SPEC.match(name)
        if m:
            return int(m.group(1)), int(m.group(2))
        expr = symbols.get(name)
        if expr is None:
            return None
        tokens = TOKEN.findall(expr)
        return first_layer(tokens[0], depth + 1) if tokens else None

    out, seen = [], set()
    for c in conn.each_connection():
        for name in (c.layer_a(), c.layer_b()):
            if name in seen:
                continue
            seen.add(name)
            spec = first_layer(name)
            if spec is not None:
                out.append((name, spec))
    return out


class Seed:
    """Where a trace starts: a device terminal or a pin label of the extraction."""

    __slots__ = ("device", "terminal", "label", "point", "layer", "net", "owner",
                 "traceable", "number", "elayer", "pin")

    def __init__(self, point, net, device=None, terminal=None, label=None, number=None):
        self.device = device
        self.terminal = terminal
        self.label = label
        self.point = point          # µm, top cell, as extracted
        self.layer = None           # layer index to start on
        self.net = net              # extracted net id
        self.owner = None           # name of the top-level bound instance holding it
        self.traceable = False
        self.number = number        # layer number of a label ("8/25" → 8)
        self.elayer = None          # the tracer's layer of the shape at the point
        self.pin = None             # a pin's label: the pin's name (it moves with it)


class Trace:
    """One traced net: its key, box (µm), and its shapes: (tracer layer, Polygon in
    database units, top cell)."""

    def __init__(self, key, box, items, dbu):
        self.key = key
        self.box = box
        self.items = items
        self.dbu = dbu
        self._polygons = None

    @property
    def polygons(self):
        """The shapes in µm, for drawing."""
        if self._polygons is None:
            self._polygons = [poly.to_dtype(self.dbu) for _, poly in self.items]
        return self._polygons

    def layer_at(self, point):
        """The tracer layer of a shape of this net at `point` (a Point, dbu)."""
        for layer, poly in self.items:
            if poly.bbox().contains(point) and poly.inside(point):
                return layer
        return None

    def holds(self, layer, point):
        """A shape of this net on tracer layer `layer` is at `point` (dbu)."""
        for lay, poly in self.items:
            if lay == layer and poly.bbox().contains(point) and poly.inside(point):
                return True
        return False


class Live:
    def __init__(self):
        self.on = True
        self.delay = 0.3
        # An update slower than this turns live updates off (they block the editor).
        self.limit = 0.25
        self.conn = None
        self.reason = ""
        self.seeds = []
        self.base = None            # the extraction's JSON (devices, labels, nets)
        self.owners = {}            # top-level instance name → DCplxTrans at setup
        self.revision = None        # revision the last result is of
        self.seen = None            # revision seen last, and since when
        self.seen_at = 0.0
        self.result = None          # last event sent
        self.traces = []            # Trace per live net id (None: kept as extracted)
        self.extracted = {}         # live net id → extracted net id (kept nets)
        self.timing = {}

    # -- setup ------------------------------------------------------------------

    def configure(self, editor, p):
        if "on" in p:
            self.on = bool(p["on"])
        if "delay_ms" in p:
            self.delay = max(0.0, float(p["delay_ms"]) / 1000.0)
        if "limit_ms" in p:
            self.limit = max(0.001, float(p["limit_ms"]) / 1000.0)
        if p.get("connectivity"):
            self.setup(editor, p)
        return self.status()

    def status(self):
        return {"available": self.conn is not None and bool(self.seeds),
                "reason": self.reason, "on": self.on, "delay_ms": int(self.delay * 1000),
                "limit_ms": int(self.limit * 1000),
                "seeds": len(self.seeds),
                "traced": sum(1 for s in self.seeds if s.traceable),
                "timing": self.timing}

    def clear(self):
        self.seeds = []
        self.base = None
        self.result = None
        self.traces = []
        self.revision = None

    def setup(self, editor, p):
        """Seeds from the extraction's JSON; calibrate on the layout as it is now."""
        self.clear()
        started = time.monotonic()
        with open(p["connectivity"]) as f:
            self.base = json.load(f)
        layout = editor.doc.layout if editor.doc is not None else None
        if layout is None:
            self.reason = "no layout is open"
            return
        if p.get("rules"):
            self.conn = make_connectivity(p["rules"])
        else:
            self.conn = tech_connectivity(layout.technology_name)
        if self.conn is None:
            self.reason = ("the technology %r has no net tracer connectivity: flylines "
                           "follow Update Connectivity only" % (layout.technology_name or ""))
            return
        self.reason = ""
        self.layers = []
        for name, (l, d) in conductors(self.conn):
            index = layout.find_layer(pya.LayerInfo(l, d))
            if index is not None:
                self.layers.append((name, l, index))
        for i, device in enumerate(self.base.get("devices") or []):
            for k, t in enumerate(device.get("terminals") or []):
                if t.get("point") is not None:
                    self.seeds.append(Seed(tuple(t["point"]), t.get("net"), device=i,
                                           terminal=k))
        for i, label in enumerate(self.base.get("labels") or []):
            number = None
            m = LAYER_SPEC.match(str(label.get("layer") or ""))
            if m:
                number = int(m.group(1))
            seed = Seed(tuple(label["point"]), label.get("net"), label=i, number=number)
            seed.pin = label.get("pin")
            self.seeds.append(seed)
        top = self._top(editor)
        self.owners = {}
        instances = self._instances(top)
        for name, inst in instances.items():
            self.owners[name] = inst.dcplx_trans
        boxes = [(name, inst.dbbox()) for name, inst in instances.items()]
        for seed in self.seeds:
            if seed.device is None:
                continue
            box = (self.base["devices"][seed.device].get("bbox"))
            point = pya.DPoint(*seed.point) if box is None else \
                pya.DPoint((box[0] + box[2]) / 2, (box[1] + box[3]) / 2)
            inside = [(b.area(), n) for n, b in boxes if b.contains(point)]
            seed.owner = min(inside)[1] if inside else None
        # Calibration: which seeds trace into nets that agree with the extraction.
        dbu = layout.dbu
        for seed in self.seeds:
            seed.layer = self._start_layer(layout, top, seed, dbu)
        items = [(seed, seed.point) for seed in self.seeds if seed.layer is not None]
        traces, found = self._group(layout, top, items)
        groups = {}
        for seed, _ in items:
            net = found.get(id(seed))
            if net is None:
                seed.layer = None
                continue
            groups.setdefault(net, []).append(seed)
        for members in groups.values():
            counts = {}
            for s in members:
                counts[s.net] = counts.get(s.net, 0) + 1
            best = max(counts.values())
            winners = [n for n, c in counts.items() if c == best]
            majority = winners[0] if len(winners) == 1 and winners[0] is not None else None
            for s in members:
                # A seed on no net of the extraction (a label beside the wire) agrees
                # with any net.
                s.traceable = majority is not None and s.net in (majority, None)
        self.revision = None
        self.seen = None
        self.timing["setup_ms"] = round((time.monotonic() - started) * 1000, 2)

    @staticmethod
    def _pins_now(editor, top):
        """Pin name -> where its label is now (the middle of its first shape)."""
        pins = getattr(editor.doc, "pins", None)
        if pins is None or top is None:
            return None
        out = {}
        for name, p in pins.collect(top.cell_index()).items():
            point = p["label"]
            if point is None and p["shapes"]:
                from .pins import _middle
                point = _middle(p["shapes"])
            if point is not None:
                out[name] = point
        return out

    @staticmethod
    def _top(editor):
        return editor.doc.top_cell() if editor.doc is not None else None

    @staticmethod
    def _instances(top):
        out = {}
        if top is None:
            return out
        for inst in top.each_inst():
            value = inst.property(NAME_PROPERTY)
            if value is not None:
                out[str(value)] = inst
        return out

    def _start_layer(self, layout, top, seed, dbu):
        """The conductor layer a trace from this seed starts on: a label's own layer
        number first, then the first conductor with a shape at the point."""
        order = list(self.layers)
        if seed.number is not None:
            order.sort(key=lambda entry: entry[1] != seed.number)
        p = pya.Point(int(round(seed.point[0] / dbu)), int(round(seed.point[1] / dbu)))
        probe = pya.Box(p.x - 1, p.y - 1, p.x + 1, p.y + 1)
        for _, _, index in order:
            it = top.begin_shapes_rec_touching(index, probe)
            if not it.at_end():
                return index
        return None

    def _trace(self, layout, top, point, layer):
        dbu = layout.dbu
        tracer = pya.NetTracer()
        p = pya.Point(int(round(point[0] / dbu)), int(round(point[1] / dbu)))
        try:
            tracer.trace(self.conn, layout, top, p, layer)
        except Exception:
            return None
        key = None
        box = pya.Box()
        items = []
        for el in tracer.each_element():
            b = el.bbox()
            k = (el.layer(), b.left, b.bottom, b.right, b.top)
            if key is None or k < key:
                key = k
            box += b
            shape = el.shape()
            if shape is not None and (shape.is_polygon() or shape.is_box() or shape.is_path()):
                items.append((el.layer(), shape.polygon.transformed(el.trans())))
        if key is None:
            return None
        return Trace(key, box.to_dtype(dbu), items, dbu)

    def _group(self, layout, top, items):
        """Trace `items` [(seed, point µm)]: the traces, and seed id → index of its
        trace (None: nothing traced). A seed whose point lies on a shape of a net
        traced already (on its own tracer layer) joins it without a trace of its own."""
        dbu = layout.dbu
        traces, by_key, result = [], {}, {}
        located = [(seed, point, pya.Point(int(round(point[0] / dbu)),
                                           int(round(point[1] / dbu))))
                   for seed, point in items]
        count = 0
        for seed, point, at in located:
            if id(seed) in result:
                continue
            trace = self._trace(layout, top, point, seed.layer)
            count += 1
            if trace is None:
                result[id(seed)] = None
                continue
            net = by_key.get(trace.key)
            if net is None:
                net = len(traces)
                by_key[trace.key] = net
                traces.append(trace)
            result[id(seed)] = net
            if seed.elayer is None:
                seed.elayer = trace.layer_at(at)
            box = trace.box
            for other, opoint, oat in located:
                if id(other) in result or other.elayer is None:
                    continue
                if not (box.left <= opoint[0] <= box.right and
                        box.bottom <= opoint[1] <= box.top):
                    continue
                if trace.holds(other.elayer, oat):
                    result[id(other)] = net
        self.timing["traces"] = count
        return traces, result

    # -- updates ------------------------------------------------------------------

    def due(self, editor):
        """True when the layout changed since the last result and has been quiet for
        the delay."""
        if not self.on or not self.seeds or self.conn is None or editor.view is None:
            return False
        revision = editor.revision
        if revision != self.seen:
            self.seen = revision
            self.seen_at = time.monotonic()
            return False
        return revision != self.revision and time.monotonic() - self.seen_at >= self.delay

    def update(self, editor):
        """Trace the seeds on the layout as it is now; the event for CDePT."""
        started = time.monotonic()
        layout = editor.doc.layout
        top = self._top(editor)
        dbu = layout.dbu
        instances = self._instances(top)
        moved = {}
        deltas = {}
        for name, before in self.owners.items():
            inst = instances.get(name)
            if inst is None:
                continue
            now = inst.dcplx_trans
            if str(now) != str(before):
                delta = now * before.inverted()
                deltas[name] = delta
                m = pya.DCplxTrans(delta)
                u = m * pya.DPoint(1, 0) - m * pya.DPoint(0, 0)
                v = m * pya.DPoint(0, 1) - m * pya.DPoint(0, 0)
                o = m * pya.DPoint(0, 0)
                moved[name] = [round(u.x, 9), round(v.x, 9), round(u.y, 9), round(v.y, 9),
                               round(o.x, 6), round(o.y, 6)]
        gone = sorted(n for n in self.owners if n not in instances)
        added = sorted(n for n in instances if n not in self.owners)
        # Trace the traceable seeds (a seed inside a net traced already joins it).
        def now(seed):
            if seed.owner in deltas:
                q = deltas[seed.owner] * pya.DPoint(*seed.point)
                return (q.x, q.y)
            return seed.point

        # Pins (pins.py) are where they are now: moved, the trace starts there.
        pins_now = self._pins_now(editor, top)
        for seed in self.seeds:
            if seed.pin is None or pins_now is None:
                continue
            point = pins_now.get(seed.pin)
            if point is None:
                seed.traceable = False
                seed.layer = None
                continue
            if tuple(point) != tuple(seed.point):
                seed.point = tuple(point)
                seed.layer = None  # looked for again below
        # Labels that were on no shape: perhaps a wire is under them now.
        for seed in self.seeds:
            if seed.label is not None and seed.layer is None:
                seed.layer = self._start_layer(layout, top, seed, dbu)
                seed.traceable = seed.layer is not None
        items = [(seed, now(seed)) for seed in self.seeds
                 if seed.traceable and seed.owner not in gone]
        traces, found = self._group(layout, top, items)
        live_of = {id(seed): (found.get(id(seed)), point) for seed, point in items}
        traced = self.timing.get("traces", 0)
        # Seeds kept as extracted join the live net of a traced seed of their
        # extracted net, else a net of their own (the extracted one).
        extracted_to_live = {}
        for seed in self.seeds:
            entry = live_of.get(id(seed))
            if entry is not None and entry[0] is not None and seed.net is not None:
                extracted_to_live.setdefault(seed.net, entry[0])
        kept = {}
        for seed in self.seeds:
            if seed.owner in gone or seed.traceable:
                continue
            point = now(seed)
            if seed.net is None:
                live_of[id(seed)] = (None, point)
                continue
            net = extracted_to_live.get(seed.net)
            if net is None:
                net = kept.get(seed.net)
                if net is None:
                    net = len(traces)
                    kept[seed.net] = net
                    traces.append(None)
            live_of[id(seed)] = (net, point)
        self.traces = traces
        self.extracted = {v: k for k, v in kept.items()}
        # The nets: names and boxes from the extraction.
        base_nets = self.base.get("nets") or []
        names = {}
        for seed in self.seeds:
            entry = live_of.get(id(seed))
            if entry and entry[0] is not None and seed.net is not None \
                    and 0 <= seed.net < len(base_nets):
                names.setdefault(entry[0], {}).setdefault(base_nets[seed.net].get("name") or "", 0)
                names[entry[0]][base_nets[seed.net].get("name") or ""] += 1
        nets = []
        for i, trace in enumerate(traces):
            counts = names.get(i, {})
            named = sorted((c, n) for n, c in counts.items() if n)
            name = named[-1][1] if named else ""
            if trace is not None:
                b = trace.box
                bbox = [round(b.left, 6), round(b.bottom, 6), round(b.right, 6), round(b.top, 6)]
            else:
                base = base_nets[self.extracted[i]] if self.extracted.get(i) is not None \
                    and self.extracted[i] < len(base_nets) else {}
                bbox = base.get("bbox")
            nets.append({"id": i, "name": name, "bbox": bbox})
        devices = []
        for i, device in enumerate(self.base.get("devices") or []):
            terms = []
            owner = None
            for k, t in enumerate(device.get("terminals") or []):
                seed = self._seed_of(i, k)
                if seed is not None:
                    owner = seed.owner
                entry = live_of.get(id(seed)) if seed is not None else None
                if entry is None:
                    terms.append({"net": None, "point": t.get("point")})
                else:
                    terms.append({"net": entry[0],
                                  "point": [round(entry[1][0], 6), round(entry[1][1], 6)]})
            if owner in gone:
                continue
            bbox = device.get("bbox")
            if bbox is not None and owner in deltas:
                box = deltas[owner] * pya.DBox(*bbox)
                bbox = [round(box.left, 6), round(box.bottom, 6), round(box.right, 6),
                        round(box.top, 6)]
            devices.append({"index": i, "bbox": bbox, "terminals": terms})
        labels = []
        for i, _ in enumerate(self.base.get("labels") or []):
            seed = self._label_seed(i)
            entry = live_of.get(id(seed)) if seed is not None else None
            labels.append(entry[0] if entry else None)
        self.revision = editor.revision
        ms = round((time.monotonic() - started) * 1000, 2)
        self.timing["update_ms"] = ms
        self.timing["traces"] = traced
        stopped = ""
        if self.on and ms > self.limit * 1000:
            self.on = False
            stopped = ("live flylines are off: tracing took %.0f ms (more than %d ms); "
                       "Update Connectivity draws them" % (ms, int(self.limit * 1000)))
        self.result = {"kind": "nets_live", "revision": editor.revision, "ms": ms,
                       "nets": nets, "devices": devices, "moved": moved,
                       "labels": labels, "added": added, "gone": gone,
                       "untraced": sum(1 for s in self.seeds if not s.traceable),
                       "stopped": stopped}
        return self.result

    def _seed_of(self, device, terminal):
        index = getattr(self, "_seed_index", None)
        if index is None or index[0] is not self.seeds:
            table = {}
            labels = {}
            for s in self.seeds:
                if s.device is not None:
                    table[(s.device, s.terminal)] = s
                else:
                    labels[s.label] = s
            self._seed_index = index = (self.seeds, table, labels)
        return index[1].get((device, terminal))

    def _label_seed(self, label):
        self._seed_of(-1, -1)
        return self._seed_index[2].get(label)

    # -- drawing ------------------------------------------------------------------

    def polygons(self, net):
        """The polygons (µm) of live net `net`; None when it is kept as extracted
        (`extracted_id` says which)."""
        if not 0 <= net < len(self.traces):
            return []
        trace = self.traces[net]
        return None if trace is None else trace.polygons

    def extracted_id(self, net):
        return self.extracted.get(net)

    def probe(self, layout, top, point):
        """The live net with a shape at `point` (µm): the topmost conductor first."""
        if not self.traces or self.conn is None:
            return None
        dbu = layout.dbu
        p = pya.Point(int(round(point[0] / dbu)), int(round(point[1] / dbu)))
        probe = pya.Box(p.x - 1, p.y - 1, p.x + 1, p.y + 1)
        keys = {t.key: i for i, t in enumerate(self.traces) if t is not None}
        for _, _, index in reversed(self.layers):
            if top.begin_shapes_rec_touching(index, probe).at_end():
                continue
            trace = self._trace(layout, top, point, index)
            if trace is None:
                continue
            return keys.get(trace.key, -1), trace
        return None
