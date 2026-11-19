# cdept-klayout: summary of an LVS database for CDePT.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Reads a KLayout LVS database (.lvsdb) and writes what CDePT shows as JSON.

Run in batch mode: klayout -b -r lvsdb.py -rd lvsdb=<file.lvsdb> -rd out=<file.json>

The JSON: {"matched": bool, "circuits": [{"layout", "schematic", "status",
"nets": [...], "devices": [...], "pins": [...], "subcircuits": [...]}]} where every
list holds the pairs that do not match, or match with a warning: {"layout",
"schematic", "status"} with the names on either side ("" when that side has none).
Circuits that match have empty lists. Layout nets also carry "bbox": [l, b, r, t] in µm when they have shapes.
"""

import json

import pya

# Statuses that count as a match; pairs with them are not listed.
GOOD = ("Match", "Skipped")
# Listed, but the netlists still match (e.g. a device parameter differs a little).
WARNING = "MatchWithWarning"


def _status(value):
    text = str(value)
    return text.rsplit(".", 1)[-1] if "." in text else text


def _name(obj, attr="name"):
    if obj is None:
        return ""
    for getter in ("expanded_name", attr):
        if hasattr(obj, getter):
            value = getattr(obj, getter)
            value = value() if callable(value) else value
            if value:
                return str(value)
    return ""


def _net_bbox(lvs, net):
    """The box around the shapes of a layout net, in µm, or None."""
    if net is None:
        return None
    try:
        dbu = lvs.internal_layout().dbu
        box = pya.Box()
        for name in lvs.layer_names():
            region = lvs.shapes_of_net(net, lvs.layer_by_name(name), True)
            if not region.is_empty():
                box += region.bbox()
        if box.empty():
            return None
        return [box.left * dbu, box.bottom * dbu, box.right * dbu, box.top * dbu]
    except Exception:
        return None


def summarize(path):
    lvs = pya.LayoutVsSchematic()
    lvs.read(path)
    xref = lvs.xref()
    if xref is None:
        raise ValueError("%s has no cross reference (was LVS run with compare?)" % path)
    circuits = []
    matched = True
    for pair in xref.each_circuit_pair():
        status = _status(pair.status())
        entry = {"layout": _name(pair.first()), "schematic": _name(pair.second()),
                 "status": status, "nets": [], "devices": [], "pins": [],
                 "subcircuits": []}
        if status not in GOOD and status != WARNING:
            matched = False
        for key, each in (("nets", xref.each_net_pair), ("devices", xref.each_device_pair),
                          ("pins", xref.each_pin_pair),
                          ("subcircuits", xref.each_subcircuit_pair)):
            for item in each(pair):
                item_status = _status(item.status())
                if item_status in GOOD:
                    continue
                if item_status != WARNING:
                    matched = False
                record = {"layout": _name(item.first()), "schematic": _name(item.second()),
                          "status": item_status}
                if key == "nets":
                    bbox = _net_bbox(lvs, item.first())
                    if bbox:
                        record["bbox"] = bbox
                entry[key].append(record)
        circuits.append(entry)
    if not circuits:
        matched = False
    return {"matched": matched, "circuits": circuits}


def main():
    source = globals().get("lvsdb")
    out = globals().get("out")
    if not source or not out:
        raise ValueError("usage: klayout -b -r lvsdb.py -rd lvsdb=<file> -rd out=<file.json>")
    with open(out, "w") as f:
        json.dump(summarize(source), f, indent=1)


main()
