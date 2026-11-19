# cdept-klayout tests (plain python3; run through ./run.sh).
# SPDX-License-Identifier: GPL-3.0-or-later
"""The helper against the protocol of docs/KLAYOUT.md §4."""

import os
import shutil
import tempfile
import time
import unittest

from client import CallError, Helper

# Writes the fixtures: inv (two boxes), buf (two inv and one res, all ghosts), and
# loop (places itself).
FIXTURES = r'''
import pya
def ghost(ly, name):
    c = ly.create_cell(name); c.ghost_cell = True; return c
ly = pya.Layout(); ly.dbu = 0.001
inv = ly.create_cell("inv")
inv.shapes(ly.layer(1, 0)).insert(pya.Box(0, 0, 1000, 3000))
inv.shapes(ly.layer(2, 0)).insert(pya.Box(200, 200, 800, 2800))
ly.write(FOLDER + "/inv.gds")
ly = pya.Layout(); ly.dbu = 0.001
buf = ly.create_cell("buf")
buf.shapes(ly.layer(3, 0)).insert(pya.Box(-500, -500, 12000, -100))
g = ghost(ly, "inv")
buf.insert(pya.CellInstArray(g.cell_index(), pya.Trans(0, 0)))
buf.insert(pya.CellInstArray(g.cell_index(), pya.Trans(5000, 0)))
r = ghost(ly, "res")
buf.insert(pya.CellInstArray(r.cell_index(), pya.Trans(10000, 0)))
ly.write(FOLDER + "/buf.gds")
ly = pya.Layout(); ly.dbu = 0.001
loop = ly.create_cell("loop"); loop.shapes(ly.layer(1, 0)).insert(pya.Box(0, 0, 100, 100))
l2 = ghost(ly, "loop2"); loop.insert(pya.CellInstArray(l2.cell_index(), pya.Trans(200, 0)))
ly.write(FOLDER + "/loop.gds")
ly = pya.Layout(); ly.dbu = 0.001
loop2 = ly.create_cell("loop2"); loop2.shapes(ly.layer(1, 0)).insert(pya.Box(0, 0, 50, 50))
l1 = ghost(ly, "loop"); loop2.insert(pya.CellInstArray(l1.cell_index(), pya.Trans(0, 200)))
ly.write(FOLDER + "/loop2.gds")
"ok"
'''

# Reads a file back: {cell name: [ghost?, number of shapes, [child names]]}.
READBACK = r'''
import pya
ly = pya.Layout(); ly.read(PATH)
{c.name: [c.is_ghost_cell(), sum(c.shapes(li).size() for li in ly.layer_indexes()),
          sorted(ly.cell(i).name for i in c.each_child_cell())] for c in ly.each_cell()}
'''


def table(folder, root="buf.gds"):
    inv = os.path.join(folder, "inv.gds")
    return {
        "root": os.path.join(folder, root),
        "files": {
            os.path.join(folder, "buf.gds"): {"label": "logic/buf/layout", "refs": {
                "inv": {"file": inv, "label": "logic/inv/layout"},
                "res": {"file": None, "label": "logic/res/layout", "bbox": [0, 0, 1, 1],
                        "reason": "cell res has no layout view"},
            }},
            inv: {"label": "logic/inv/layout", "refs": {}},
            os.path.join(folder, "loop.gds"): {"label": "t/loop/layout", "refs": {
                "loop2": {"file": os.path.join(folder, "loop2.gds"), "label": "t/loop2/layout"}}},
            os.path.join(folder, "loop2.gds"): {"label": "t/loop2/layout", "refs": {
                "loop": {"file": os.path.join(folder, "loop.gds"), "label": "t/loop/layout"}}},
        },
    }


class Headless(unittest.TestCase):
    """Without a display: queries, open, save, export, render."""

    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="cdk-")
        cls.helper = Helper(cls.folder)
        cls.helper.call("python", code="FOLDER = %r\n" % cls.folder + FIXTURES)

    @classmethod
    def tearDownClass(cls):
        cls.helper.close()
        shutil.rmtree(cls.folder, ignore_errors=True)

    def readback(self, path):
        return eval(self.helper.call("python", code="PATH = %r\n" % path + READBACK))

    def test_ready_and_info(self):
        self.assertEqual(self.helper.ready["type"], "ready", self.helper.ready)
        self.assertEqual(self.helper.ready["protocol"], 1)
        self.assertIn("0.30", self.helper.ready["klayout"])
        info = self.helper.call("info")
        self.assertIn("KLayout", info["klayout"])

    def test_python(self):
        self.assertEqual(self.helper.call("python", code="x = 20\nx + 1"), "21")
        with self.assertRaises(CallError):
            self.helper.call("python", code="1/0")

    def test_unknown_method(self):
        with self.assertRaises(CallError) as e:
            self.helper.call("frobnicate")
        self.assertIn("unknown method", str(e.exception))

    def test_open_save_export(self):
        value = self.helper.call("open", refs=table(self.folder))
        self.assertEqual(value["top"], "buf")
        self.assertEqual(value["external"], ["inv"])
        self.assertEqual(value["missing"], ["res"])
        # The missing cell carries the marker: box, two diagonals and the text.
        marker = self.helper.call("python", code=(
            "c = layout.cell('res')\n"
            "[s.text_string for li in layout.layer_indexes() for s in c.shapes(li).each() "
            "if s.is_text()] + [c.shapes(li).size() for li in layout.layer_indexes()]"))
        self.assertIn("logic/res/layout cannot be found", marker)
        self.assertIn(4, eval(marker))

        out = os.path.join(self.folder, "buf-saved.gds")
        report = self.helper.call("save", file=out)
        self.assertEqual(report["top"], "buf")
        refs = report["references"]
        self.assertEqual(sorted(refs), ["inv", "res"])
        self.assertEqual(refs["inv"]["file"], os.path.join(self.folder, "inv.gds"))
        self.assertFalse(refs["inv"]["missing"])
        self.assertEqual(refs["inv"]["bbox"], [0.0, 0.0, 1.0, 3.0])
        self.assertTrue(refs["res"]["missing"])
        self.assertEqual(refs["res"]["bbox"], [0.0, 0.0, 1.0, 1.0])
        cells = self.readback(out)
        self.assertEqual(cells["buf"], [False, 1, ["inv", "res"]])
        self.assertEqual(cells["inv"][:2], [True, 0])  # a reference, not a definition
        self.assertEqual(cells["res"][:2], [True, 0])
        self.assertEqual(set(cells), {"buf", "inv", "res"})

        out = os.path.join(self.folder, "buf-flat.gds")
        value = self.helper.call("export", out=out)
        self.assertEqual(value["missing"], ["res"])
        cells = self.readback(out)
        self.assertEqual(cells["inv"][:2], [False, 2])
        self.assertTrue(cells["res"][0])

    def test_cycle_is_missing(self):
        value = self.helper.call("open", refs=table(self.folder, "loop.gds"))
        self.assertEqual(value["top"], "loop")
        self.assertEqual(value["external"], ["loop2"])
        self.assertEqual(len(value["missing"]), 1)

    def test_clash_with_own_cell(self):
        # inv placed into a file that has a cell of its own named "inv": the external
        # copy is renamed in memory, the file's own cell keeps its name.
        path = os.path.join(self.folder, "clash.gds")
        self.helper.call("python", code=(
            "import pya\nly = pya.Layout()\ntop = ly.create_cell('clash')\n"
            "own = ly.create_cell('inv'); own.shapes(ly.layer(9, 0)).insert(pya.Box(0, 0, 9, 9))\n"
            "top.insert(pya.CellInstArray(own.cell_index(), pya.Trans()))\n"
            "ly.write(%r)" % path))
        t = table(self.folder)
        t["root"] = path
        t["files"][path] = {"label": "t/clash/layout", "refs": {}}
        self.helper.call("open", refs=t)
        placed = self.helper.call("place", refs=dict(t, root=os.path.join(self.folder, "inv.gds")))
        self.assertNotEqual(placed["name"], "inv")
        report = self.helper.call("save", file=os.path.join(self.folder, "clash-saved.gds"))
        self.assertEqual(report["references"], {})  # placed but not used: nothing kept

    def test_render(self):
        out = os.path.join(self.folder, "buf.png")
        value = self.helper.call("render", refs=table(self.folder), width=320, height=200, out=out)
        self.assertEqual((value["width"], value["height"]), (320, 200))
        with open(out, "rb") as f:
            self.assertEqual(f.read(8), b"\x89PNG\r\n\x1a\n")


class Display(unittest.TestCase):
    """With a display: frames, input, editing, undo, events."""

    W, H = 640, 400

    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="cdk-")
        cls.helper = Helper(cls.folder, display=True)
        cls.helper.call("python", code="FOLDER = %r\n" % cls.folder + FIXTURES)
        cls.helper.send({"type": "resize", "width": cls.W, "height": cls.H})
        cls.helper.call("open", refs=table(cls.folder))
        cls.helper.call("zoom", what="box", box=[-1, -4, 15, 6])
        cls.helper.settle()

    @classmethod
    def tearDownClass(cls):
        cls.helper.close()
        shutil.rmtree(cls.folder, ignore_errors=True)

    def shapes(self, layer):
        return int(self.helper.call("python", code=(
            "c = layout.cell('buf')\n"
            "li = layout.find_layer(%d, 0)\nc.shapes(li).size() if li is not None else 0" % layer)))

    def test_1_frame(self):
        h = self.helper
        self.assertTrue(h.frames, "no frame message")
        frame = h.frames[-1]
        self.assertEqual((frame["width"], frame["height"], frame["stride"], frame["format"]),
                         (self.W, self.H, self.W * 4, "bgra"))
        self.assertTrue(h.damage)
        data = h.pixels(frame)
        self.assertEqual(len(data), self.W * self.H * 4)
        self.assertGreater(len({data[i:i + 4] for i in range(0, len(data), 4 * 97)}), 2)

    def test_2_events(self):
        h = self.helper
        layers = h.last_event("layers")["layers"]
        sources = [l["source"] for l in layers]
        self.assertTrue(any(s.startswith("1/0") for s in sources), sources)
        self.assertTrue(all(l["color"].startswith("#") for l in layers))
        cells = {c["name"]: c for c in h.last_event("cells")["cells"]}
        self.assertEqual(h.last_event("cells")["top"], "buf")
        self.assertEqual(cells["inv"]["external"], "logic/inv/layout")
        self.assertTrue(cells["res"]["missing"])
        self.assertEqual(cells["buf"]["children"], ["inv", "res"])
        self.assertFalse(h.last_event("modified")["dirty"])

    def test_3_box_needs_layer(self):
        h = self.helper
        h.call("mode", name="box")
        n = len(h.events)
        h.click(100, 100)
        h.click(200, 150)
        h.settle()
        notices = [e for e in h.events[n:] if e["kind"] == "notice"]
        self.assertTrue(notices, "no notice")
        self.assertIn("layer", notices[0]["text"])

    def test_4_box_and_undo(self):
        h = self.helper
        layers = h.last_event("layers")["layers"]
        layer = next(l for l in layers if l["source"].startswith("3/0"))
        h.call("current_layer", id=layer["id"])
        h.settle()
        current = [l["source"] for l in h.last_event("layers")["layers"] if l["current"]]
        self.assertEqual(current, [layer["source"]])
        h.call("mode", name="box")
        before = self.shapes(3)
        h.click(300, 100)
        h.input(kind="move", x=350, y=120, buttons=[])
        h.settle()
        status = h.last_event("status")
        self.assertEqual(status["mode"], "box")
        self.assertIsNotNone(status["x"])
        h.click(400, 150)
        h.settle()
        self.assertEqual(self.shapes(3), before + 1)
        self.assertTrue(h.last_event("modified")["dirty"])
        self.assertTrue(h.last_event("modified")["can_undo"])
        h.call("undo")
        h.settle()
        self.assertEqual(self.shapes(3), before)
        self.assertTrue(h.last_event("modified")["can_redo"])
        h.call("redo")
        self.assertEqual(self.shapes(3), before + 1)

    def test_5_select_and_status(self):
        h = self.helper
        h.call("mode", name="select")
        h.call("select_all")
        h.settle()
        sel = h.last_event("selection")
        self.assertGreater(sel["count"], 0)
        self.assertIsNotNone(sel["bbox"])
        h.call("clear_selection")
        h.settle()
        self.assertEqual(h.last_event("selection")["count"], 0)

    def test_6_resize(self):
        h = self.helper
        n = len(h.frames)
        h.send({"type": "resize", "width": 300, "height": 200})
        h.wait(lambda m: m["type"] == "damage" and m["serial"] == h.frames[-1]["serial"]
               and len(h.frames) > n)
        self.assertEqual((h.frames[-1]["width"], h.frames[-1]["height"]), (300, 200))
        self.assertFalse(os.path.exists(h.frames[n - 1]["path"]))

    def test_6b_place_and_copy(self):
        h = self.helper
        h.send({"type": "resize", "width": self.W, "height": self.H})
        h.call("zoom", what="box", box=[-1, -4, 15, 6])
        count = 'len(list(layout.cell("buf").each_inst()))'
        before = int(h.call("python", code=count))
        t = table(self.folder)
        placed = h.call("place", refs=dict(t, root=os.path.join(self.folder, "inv.gds")))
        self.assertEqual(placed["name"], "inv")  # already there: the same cell
        self.assertEqual(h.call("get_config", name="edit-inst-cell-name"), "inv")
        h.click(500, 300)
        h.settle()
        self.assertEqual(int(h.call("python", code=count)), before + 1)
        h.call("mode", name="select")
        h.call("select_all")
        h.call("copy")
        h.call("paste")
        h.input(kind="key", key="Escape")
        h.settle()
        h.call("undo")

    def to_pixel(self, x, y):
        frame = self.helper.frames[-1]
        return eval(self.helper.call("python", code=(
            "p = view.viewport_trans() * pya.DPoint(%r, %r)\n"
            "(p.x, %d - 1 - p.y)" % (x, y, frame["height"]))))

    def test_6c_top_level_selection(self):
        h = self.helper
        h.call("clear_selection")
        h.call("mode", name="select")
        x, y = self.to_pixel(0.5, 1.5)  # inside the first inv
        h.click(x, y)
        h.settle()
        sel = h.last_event("selection")
        self.assertEqual([i["kind"] for i in sel["items"]], ["instance"], sel)
        self.assertEqual(sel["items"][0]["cell"], "buf")

        # Selection inside cells turned on by hand: the shape of inv is dropped again.
        h.call("config", name="edit-top-level-selection", value="false")
        h.call("clear_selection")
        h.settle()
        n = len(h.events)
        h.click(x, y)
        h.settle()
        self.assertEqual(h.last_event("selection")["count"], 0)
        notices = [e for e in h.events[n:] if e["kind"] == "notice"]
        self.assertTrue(notices and "other views" in notices[0]["text"], notices)
        h.call("config", name="edit-top-level-selection", value="true")

        # delete refuses when such an object is selected anyway.
        with self.assertRaises(CallError) as e:
            h.call("python", code=(
                "server.editor.drop_foreign_selection = lambda: None\n"
                "it = pya.RecursiveShapeIterator(layout, layout.cell('buf'), layout.find_layer(1, 0))\n"
                "p = pya.ObjectInstPath(it, 0)\nview.select_object(p)\n"
                "server.m_delete({})"))
        self.assertIn("other views", str(e.exception))
        h.call("python", code="del server.editor.drop_foreign_selection\nview.clear_selection()")

    def test_6d_add_layer_colour(self):
        h = self.helper
        value = h.call("add_layer", layer=10, datatype=0, name="extra")
        h.settle()
        layer = h.last_event("layers")["layers"][value["id"]]
        self.assertIn("10/0", layer["source"])
        self.assertEqual(layer["name"], "extra")
        self.assertNotEqual(layer["color"], "#000000")
        self.assertNotEqual(layer["frame_color"], "#000000")

    def test_7_latency(self):
        """Not a check: prints the time from input to damage at two sizes."""
        h = self.helper
        for w, hh in ((1920, 1080), (3840, 2160)):
            h.send({"type": "resize", "width": w, "height": hh})
            h.settle()
            times = []
            for i in range(10):
                serial = h.frames[-1]["serial"]
                n = len(h.damage)
                t = time.monotonic()
                h.input(kind="move", x=100 + i, y=100, buttons=[])
                h.wait(lambda m, n=n: m["type"] == "damage" and len(h.damage) > n)
                times.append(time.monotonic() - t)
            times.sort()
            print("\n  %dx%d: median %.1f ms, max %.1f ms" % (w, hh, 1000 * times[5], 1000 * times[-1]))
        print("  helper start-up: %.2f s" % h.startup)


class DisplayOpenFirst(unittest.TestCase):
    """CDePT's order: open, then the size. The view fits by itself; save and undo
    report the modified state."""

    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="cdk-")
        cls.helper = Helper(cls.folder, display=True)
        cls.helper.call("python", code="FOLDER = %r\n" % cls.folder + FIXTURES)

    @classmethod
    def tearDownClass(cls):
        cls.helper.close()
        shutil.rmtree(cls.folder, ignore_errors=True)

    def test_fit_save_undo(self):
        h = self.helper
        h.call("open", refs=table(self.folder))
        h.send({"type": "resize", "width": 400, "height": 300})
        h.settle()
        data = h.pixels(h.frames[-1])
        colours = {data[i:i + 4] for i in range(0, len(data), 4 * 7)}
        self.assertGreater(len(colours), 3, "picture is empty without a zoom call")

        layer = next(l for l in h.last_event("layers")["layers"] if l["source"].startswith("3/0"))
        h.call("current_layer", id=layer["id"])
        h.call("mode", name="box")
        h.click(10, 10)
        h.click(40, 40)
        h.settle()
        self.assertTrue(h.last_event("modified")["dirty"])

        # Saved: "modified" (dirty false) arrives before the reply.
        h.call("save")
        self.assertFalse(h.last_event("modified")["dirty"])

        h.click(60, 60)
        h.click(90, 90)
        h.settle()
        self.assertTrue(h.last_event("modified")["dirty"])
        h.call("undo")
        h.settle()
        self.assertFalse(h.last_event("modified")["dirty"], "undo back to the saved state")
        h.call("undo")
        h.settle()
        self.assertTrue(h.last_event("modified")["dirty"])
        h.call("redo")
        h.settle()
        self.assertFalse(h.last_event("modified")["dirty"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
