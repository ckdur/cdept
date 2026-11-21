# cdept-klayout tests: pins (plain python3; run through ./run.sh).
# SPDX-License-Identifier: GPL-3.0-or-later
"""Pins as CDePT objects (docs/LINKING.md §9): on a technology's pin and label layers,
or CDePT's only; tracked through edits, undo and save; Draw Pin; generated from a
schematic's pins; and selecting instances by paths through the hierarchy."""

import os
import shutil
import tempfile
import unittest

from client import CallError, Helper

# Metal1-like: 8/0 with its pin layer 8/2 and text layer 8/25; 5/0 a conductor without
# a pin or text layer (its pins are CDePT's only).
RULES = {"size": 0.1, "box": 0.5, "generate": "8/0",
         "layers": [{"drawing": "8/0", "pin": "8/2", "label": "8/25"}, {"drawing": "5/0"}]}

# top: a Metal1 wire and a 5/0 wire, an old label "W" on 8/25 (before pins were objects);
# sub: an instance bound to M1 (GDS property 61); deep: x1 = sub bound to "x1".
FIXTURES = r'''
import pya
ly = pya.Layout(); ly.dbu = 0.001
top = ly.create_cell("top")
top.shapes(ly.layer(8, 0)).insert(pya.Box(0, 0, 4000, 500))
top.shapes(ly.layer(5, 0)).insert(pya.Box(0, 2000, 4000, 2300))
t = pya.Text("W", pya.Trans(pya.Vector(3800, 250))); top.shapes(ly.layer(8, 25)).insert(t)
ly.write(FOLDER + "/top.gds")
ly = pya.Layout(); ly.dbu = 0.001
deep = ly.create_cell("deep")
sub = ly.create_cell("sub")
leaf = ly.create_cell("leaf")
leaf.shapes(ly.layer(1, 0)).insert(pya.Box(0, 0, 500, 500))
i = sub.insert(pya.CellInstArray(leaf.cell_index(), pya.Trans(1000, 0)))
i.set_property(61, "M1")
i = deep.insert(pya.CellInstArray(sub.cell_index(), pya.Trans(0, 5000)))
i.set_property(61, "x1")
ly.write(FOLDER + "/deep.gds")
"ok"
'''

READ_PINS = r'''
import pya
ly = pya.Layout(); ly.read(PATH)
c = ly.cell("top")
out = []
for li in ly.layer_indexes():
    info = ly.get_info(li)
    for s in c.shapes(li).each():
        v = s.property(62)
        if v is not None:
            out.append(("%d/%d" % (info.layer, info.datatype), str(v), s.is_text()))
sorted(out), sorted(str(ly.get_info(li)) for li in ly.layer_indexes())
'''


def table(folder, root, pins=None, rules=RULES):
    path = os.path.join(folder, root)
    entry = {"label": "t/%s/layout" % root.split(".")[0], "refs": {}}
    if pins:
        entry["pins"] = pins
    out = {"root": path, "files": {path: entry}}
    if rules is not None:
        out["pin_rules"] = rules
    return out


class Pins(unittest.TestCase):
    W, H = 600, 400

    def setUp(self):
        self.folder = tempfile.mkdtemp(prefix="cdk-")
        self.helper = Helper(self.folder, display=True)
        self.helper.call("python", code="FOLDER = %r\n" % self.folder + FIXTURES)
        self.helper.send({"type": "resize", "width": self.W, "height": self.H})

    def tearDown(self):
        self.helper.close()
        shutil.rmtree(self.folder, ignore_errors=True)

    def open(self, root="top.gds", pins=None, rules=RULES):
        self.helper.call("open", refs=table(self.folder, root, pins, rules))
        self.helper.call("zoom", what="box", box=[-2, -1, 6, 4])
        self.helper.settle()

    def py(self, code):
        return eval(self.helper.call("python", code=code))

    def read(self, path):
        return self.py("PATH = %r\n" % path + READ_PINS)

    def test_1_pin_layers_and_overlay_through_save(self):
        h = self.helper
        self.open()
        h.call("pins_add", name="A", direction="i", layer="8/0", box=[0, 0, 0.5, 0.5])
        h.call("pins_add", name="B", direction="output", layer="5/0", box=[0, 2, 0.3, 2.3])
        with self.assertRaises(CallError):
            h.call("pins_add", name="A", layer="8/0", box=[1, 0, 1.5, 0.5])
        pins = h.call("pins_list")["pins"]
        self.assertEqual(pins["A"]["shapes"], [{"layer": "8/0", "box": [0, 0, 0.5, 0.5]}])
        self.assertEqual(pins["A"]["direction"], "input")
        self.assertFalse(pins["A"]["overlay"])
        self.assertTrue(pins["A"]["label_on"])
        self.assertTrue(pins["B"]["overlay"])
        self.assertEqual(pins["B"]["shapes"][0]["layer"], "5/0")
        # Moving the pin's shape (an ordinary edit) moves the pin.
        h.call("python", code="t = pya.DTrans(pya.DVector(1, 0))\n"
               "c = doc.top_cell()\n"
               "li = layout.find_layer(8, 2)\n"
               "for s in list(c.shapes(li).each()): s.transform(t)\n"
               "'ok'")
        moved = h.call("pins_list")["pins"]["A"]
        self.assertEqual(moved["shapes"][0]["box"], [1, 0, 1.5, 0.5])
        self.assertFalse(moved["label_on"])
        h.call("pins_update", name="A", label="middle")
        self.assertTrue(h.call("pins_list")["pins"]["A"]["label_on"])
        # Save: the report lists both; the stream holds A (8/2, 8/25), not B nor the
        # in-memory layer.
        report = h.call("save")
        self.assertEqual(sorted(report["pins"]), ["A", "B"])
        self.assertEqual(report["pins"]["B"], {"direction": "output",
                                               "shapes": [{"layer": "5/0", "box": [0, 2, 0.3, 2.3]}],
                                               "label": [0.15, 2.15]})
        tagged, layers = self.read(os.path.join(self.folder, "top.gds"))
        self.assertEqual(tagged, [("8/2", "A", False), ("8/25", "A", True)])
        self.assertFalse(any("CDEPT" in l for l in layers))
        # Opened again with the sidecar's pins: B is made again, A not twice.
        self.open(pins=report["pins"])
        again = h.call("pins_list")["pins"]
        self.assertEqual(sorted(again), ["A", "B"])
        self.assertEqual(again["A"]["count"], 1)
        self.assertEqual(again["B"]["shapes"], report["pins"]["B"]["shapes"])
        self.assertEqual(again["B"]["direction"], "output")

    def test_2_undo_rename_remove_select(self):
        h = self.helper
        self.open()
        h.call("pins_add", name="A", layer="8/0", box=[0, 0, 0.5, 0.5])
        h.call("undo")
        self.assertEqual(h.call("pins_list")["pins"], {})
        h.call("redo")
        self.assertIn("A", h.call("pins_list")["pins"])
        h.call("pins_rename", name="A", new="IN")
        pins = h.call("pins_list")["pins"]
        self.assertEqual(sorted(pins), ["IN"])
        texts = self.py("[s.text_string for s in doc.top_cell().shapes(layout.find_layer(8, 25)).each()"
                        " if s.property(62) is not None]")
        self.assertEqual(texts, ["IN"])
        self.assertEqual(h.call("pins_select", names=["IN"], zoom=True)["found"], ["IN"])
        h.settle()
        self.assertEqual(h.last_event("selection")["count"], 2)  # the shape and its label
        h.call("pins_remove", name="IN")
        self.assertEqual(h.call("pins_list")["pins"], {})
        with self.assertRaises(CallError):
            h.call("pins_remove", name="IN")

    def test_3_from_selection_and_draw_pin(self):
        h = self.helper
        self.open()
        # Pin from the selected Metal1 wire: a copy on the pin layer.
        h.call("python", code="p = pya.ObjectInstPath(); p.top = doc.top_cell().cell_index()\n"
               "p.layer = layout.find_layer(8, 0)\n"
               "p.shape = list(doc.top_cell().shapes(p.layer).each())[0]\n"
               "view.object_selection = [p]\n'ok'")
        h.call("pins_add", name="WIRE", direction="inout", from_selection=True)
        pins = h.call("pins_list")["pins"]
        self.assertEqual(pins["WIRE"]["shapes"], [{"layer": "8/0", "box": [0, 0, 4, 0.5]}])
        self.assertEqual(self.py("doc.top_cell().shapes(layout.find_layer(8, 0)).size()"), 1)
        # Draw Pin: the next box drawn on the current layer becomes the pin.
        layer = next(l for l in h.last_event("layers")["layers"] if l["source"].startswith("8/0"))
        h.call("current_layer", id=layer["id"])
        self.assertTrue(h.call("pins_tool", name="D", direction="o")["armed"])
        to_pixel = lambda x, y: self.py(
            "p = view.viewport_trans() * pya.DPoint(%r, %r)\n(p.x, %d - 1 - p.y)" % (x, y, self.H))
        h.click(*to_pixel(2.0, 1.0))
        h.click(*to_pixel(2.6, 1.6))
        h.settle()
        pins = h.call("pins_list")["pins"]
        self.assertIn("D", pins)
        self.assertEqual(pins["D"]["direction"], "output")
        self.assertFalse(pins["D"]["overlay"])
        b = pins["D"]["shapes"][0]["box"]
        self.assertAlmostEqual(b[2] - b[0], 0.6, places=2)
        # The box drawn on Metal1 itself is gone (the pin is on 8/2).
        self.assertEqual(self.py("doc.top_cell().shapes(layout.find_layer(8, 0)).size()"), 1)
        self.assertEqual(self.py("view.mode_name()"), "select")
        # Two undo steps: the pin, then the box.
        h.call("undo")
        self.assertNotIn("D", h.call("pins_list")["pins"])

    def test_4_generate_takes_over_old_labels(self):
        h = self.helper
        self.open()
        done = h.call("link_apply", items=[], pins=[
            {"name": "W", "direction": "i"}, {"name": "Z", "direction": "o", "net": "Z"}])
        self.assertEqual(sorted(done["pins"]), ["W", "Z"])
        pins = h.call("pins_list")["pins"]
        # W: the old text, now the pin's label, with a square around it.
        self.assertEqual(pins["W"]["label"], [3.8, 0.25])
        self.assertEqual(pins["W"]["shapes"][0]["box"], [3.55, 0.0, 4.05, 0.5])
        self.assertTrue(pins["W"]["label_on"])
        # Z: a new square of Metal1, left of the cell.
        z = pins["Z"]["shapes"][0]
        self.assertEqual(z["layer"], "8/0")
        self.assertLess(z["box"][2], 0)
        self.assertEqual(pins["Z"]["net"], "Z")
        again = h.call("link_apply", items=[], pins=[{"name": "W"}, {"name": "Z"}])
        self.assertEqual(again["pins"], [])
        self.assertEqual(h.call("pins_list")["pins"]["Z"]["count"], 1)

    def test_5_without_rules_pins_are_cdept_only(self):
        h = self.helper
        self.open(rules=None)
        h.call("pins_add", name="A", layer="8/0", box=[0, 0, 0.5, 0.5])
        pins = h.call("pins_list")
        self.assertFalse(pins["rules"])
        self.assertTrue(pins["pins"]["A"]["overlay"])
        report = h.call("save")
        self.assertEqual(sorted(report["pins"]), ["A"])
        tagged, _ = self.read(os.path.join(self.folder, "top.gds"))
        self.assertEqual(tagged, [])

    def test_6_instances_by_path(self):
        h = self.helper
        refs = table(self.folder, "deep.gds")
        h.call("open", refs=refs)
        h.settle()
        found = h.call("link_select", names=["x1/M1", "x1", "x2/M1", "x1/M9"], zoom=True)
        self.assertEqual(found["found"], ["x1", "x1/M1"])
        self.assertEqual(h.call("link_selected")["names"], ["x1", "x1/M1"])
        boxes = self.py("from cdept_klayout import link\nlink.boxes(editor, ['x1/M1'])")
        self.assertEqual(boxes, [[1.0, 5.0, 1.5, 5.5]])


if __name__ == "__main__":
    unittest.main(verbosity=2)
