# cdept-klayout tests: PCells (plain python3; run through ./run.sh).
# SPDX-License-Identifier: GPL-3.0-or-later
"""Shape parameters and their handles, replacing an instance's cell, the project's
PCell views (docs/KLAYOUT.md §3.6), PDK PCells saved and read back."""

import os
import shutil
import tempfile
import time
import unittest

from client import CallError, Helper
from test_helper import FIXTURES, READBACK, table

# A PCell view as CDePT's template writes it, shortened: a bar of l × w on layer 1/0.
BAR = r'''
import pya


class Bar(pya.PCellDeclarationHelper):
    def __init__(self):
        super().__init__()
        self.param("body", self.TypeLayer, "Body layer", default=pya.LayerInfo(1, 0))
        self.param("l", self.TypeDouble, "Length", default=2.0)
        self.param("w", self.TypeDouble, "Width", default=0.5)

    def produce_impl(self):
        self.cell.shapes(self.body_layer).insert(pya.DBox(0, 0, self.l * SCALE, self.w))
'''


def bar(scale=1):
    return BAR.replace("SCALE", repr(scale))


class Base(unittest.TestCase):
    W, H = 600, 400

    def start(self, refs=None, technology=None):
        self.folder = tempfile.mkdtemp(prefix="cdk-")
        self.helper = Helper(self.folder, display=True)
        self.helper.call("python", code="FOLDER = %r\n" % self.folder + FIXTURES)
        self.helper.send({"type": "resize", "width": self.W, "height": self.H})

    def open(self, refs, technology=None):
        h = self.helper
        params = {"refs": refs}
        if technology:
            params["technology"] = technology
        h.call("open", **params)
        h.call("zoom", what="box", box=[-1, -4, 15, 6])
        h.settle()

    def tearDown(self):
        self.helper.close()
        shutil.rmtree(self.folder, ignore_errors=True)

    def py(self, code):
        return eval(self.helper.call("python", code=code))

    def to_pixel(self, x, y):
        return self.py("p = view.viewport_trans() * pya.DPoint(%r, %r)\n(p.x, %d - 1 - p.y)"
                       % (x, y, self.H))

    def place(self, library, cell, at, values=None):
        h = self.helper
        h.call("place_library", library=library, cell=cell, values=values or {})
        h.click(*self.to_pixel(*at))
        h.call("mode", name="select")
        h.settle()

    def select_at(self, x, y):
        h = self.helper
        h.call("mode", name="select")
        h.call("clear_selection")
        time.sleep(0.45)  # not a double click with an earlier click there
        h.click(*self.to_pixel(x, y))
        h.settle()
        return h.call("properties")

    def pcells(self, cell="buf"):
        return self.py("[(i.cell.library().name(), i.pcell_declaration().name(), "
                       "{k: (v if isinstance(v, (int, float, str)) else str(v)) "
                       "for k, v in i.pcell_parameters_by_name().items()}) "
                       "for i in layout.cell(%r).each_inst() if i.is_pcell()]" % cell)


class Shapes(Base):
    """Shape (guiding shape) parameters: in the form, set from it, dragged."""

    def setUp(self):
        self.start()
        self.open(table(self.folder))
        self.place("Basic", "CIRCLE", (7, 2), {"actual_radius": 1.0, "layer": "3/0"})

    def circle(self):
        return [p for _, n, p in self.pcells() if n == "CIRCLE"][0]

    def test_form_and_set(self):
        h = self.helper
        props = self.select_at(7.5, 2)
        inst = props["objects"][0]
        self.assertEqual(inst["kind"], "instance")
        self.assertEqual(inst["pcell"]["values"]["handle"], {"point": [-1.0, 0.0]})
        params = {p["name"]: p for p in inst["pcell"]["params"]}
        self.assertEqual(params["handle"]["type"], "shape")
        # The handle moved by the form: the radius follows (CIRCLE's coercion).
        props = h.call("set_properties", index=0, key=inst["key"], object=inst,
                       pcell={"handle": {"point": [-2.0, 0.0]}})
        self.assertEqual(self.circle()["actual_radius"], 2.0)
        self.assertEqual(props["objects"][0]["pcell"]["values"]["handle"], {"point": [-2.0, 0.0]})
        # And the radius moves the handle.
        inst = props["objects"][0]
        props = h.call("set_properties", index=0, key=inst["key"], object=inst,
                       pcell={"actual_radius": 0.5})
        self.assertEqual(props["objects"][0]["pcell"]["values"]["handle"], {"point": [-0.5, 0.0]})
        h.call("undo")
        h.call("undo")
        self.assertEqual(self.circle()["actual_radius"], 1.0)

    def test_drag_handle(self):
        h = self.helper
        self.select_at(7.5, 2)
        self.assertEqual(self.py("len(server.editor.handles.markers)"), 1)
        (x0, y0), (x1, y1) = self.to_pixel(6, 2), self.to_pixel(4.5, 2)
        h.input(kind="move", x=x0, y=y0, buttons=[])
        h.input(kind="press", x=x0, y=y0, button="left")
        for k in range(1, 6):
            h.input(kind="move", x=x0 + (x1 - x0) * k / 5, y=y0, buttons=["left"])
        h.input(kind="release", x=x1, y=y1, button="left")
        h.settle()
        self.assertAlmostEqual(self.circle()["actual_radius"], 2.5, places=2)
        self.assertTrue(h.last_event("modified")["dirty"])
        # Still selected, with the handle where it was dropped.
        self.assertEqual(h.last_event("selection")["count"], 1)
        h.call("undo")
        h.settle()
        self.assertEqual(self.circle()["actual_radius"], 1.0)
        # A press away from the handle selects as before (a rubber band: nothing).
        a, b = self.to_pixel(12, 4), self.to_pixel(13, 5)
        h.input(kind="press", x=a[0], y=a[1], button="left")
        h.input(kind="move", x=b[0], y=b[1], buttons=["left"])
        h.input(kind="release", x=b[0], y=b[1], button="left")
        h.settle()
        self.assertEqual(self.circle()["actual_radius"], 1.0)


class Replace(Base):
    """Another cell for a selected instance, keeping where it is."""

    def setUp(self):
        self.start()
        self.open(table(self.folder))

    def test_library_cell_then_view(self):
        h = self.helper
        props = self.select_at(5.5, 1.5)  # the second inv
        inst = props["objects"][0]
        self.assertEqual(inst["external"], "logic/inv/layout")
        props = h.call("replace_cell", index=0, key=inst["key"], library="Basic",
                       cell="CIRCLE", values={"actual_radius": 0.4, "layer": "1/0"})
        now = props["objects"][0]
        self.assertEqual((now["cell"], now["library"], now["x"], now["y"]),
                         ("CIRCLE", "Basic", 5.0, 0.0))
        self.assertEqual(now["pcell"]["values"]["actual_radius"], 0.4)
        self.assertEqual(len(self.pcells()), 1)
        self.assertTrue(h.last_event("modified")["dirty"])
        # Back to a view of the project: cap.gds, given by its own table.
        cap = os.path.join(self.folder, "cap.gds")
        h.call("python", code="ly = pya.Layout(); c = ly.create_cell('cap')\n"
               "c.shapes(ly.layer(1, 0)).insert(pya.DBox(0, 0, 1, 1)); ly.write(%r)" % cap)
        refs = {"root": cap, "files": {cap: {"label": "t/cap/layout", "refs": {}}}}
        props = h.call("replace_cell", index=0, key=now["key"], refs=refs, name="cap")
        now = props["objects"][0]
        self.assertEqual((now["external"], now["x"]), ("t/cap/layout", 5.0))
        self.assertEqual(self.pcells(), [])
        # Saved: buf places inv once and loop once, both as references.
        report = h.call("save")
        self.assertEqual(sorted(report["references"]), ["cap", "inv", "res"])
        self.assertEqual(report["references"]["cap"]["file"], cap)
        back = eval(h.call("python", code="PATH = %r\n" % os.path.join(self.folder, "buf.gds")
                           + READBACK))
        self.assertEqual(back["buf"][2], ["cap", "inv", "res"])
        self.assertTrue(back["cap"][0])  # a ghost
        # Undo goes back step by step.
        h.call("undo")
        self.assertEqual(len(self.pcells()), 1)
        h.call("undo")
        insts = self.py("sorted(i.cell.name for i in layout.cell('buf').each_inst())")
        self.assertEqual(insts, ["inv", "inv", "res"])
        with self.assertRaises(CallError):
            h.call("replace_cell", index=0, key="nope", library="Basic", cell="CIRCLE")


class ProjectPcells(Base):
    """PCell views of the project: registered, placed, saved, read back, refreshed."""

    def setUp(self):
        self.start()
        self.pcell_dir = os.path.join(self.folder, "bar", "pcell")
        os.makedirs(self.pcell_dir)
        self.bar = os.path.join(self.pcell_dir, "bar.pcell.py")
        with open(self.bar, "w") as f:
            f.write(bar())
        self.refs = table(self.folder)
        self.refs["pcells"] = {"mylib": {"bar": self.bar}}
        self.open(self.refs)

    def width(self):
        return self.py("[round(i.bbox().width() * layout.dbu, 3) for i in "
                       "layout.cell('buf').each_inst() if i.is_pcell()]")

    def test_register_place_save_reload_refresh(self):
        h = self.helper
        libs = {l["name"]: l for l in h.call("libraries")}
        self.assertTrue(libs["mylib"]["project"])
        self.assertFalse(libs["Basic"]["project"])
        self.assertEqual(libs["mylib"]["cells"], [{"name": "bar", "pcell": True}])
        form = h.call("pcell_form", library="mylib", cell="bar")
        self.assertEqual(form["values"]["l"], 2.0)
        self.place("mylib", "bar", (7, 2), {"l": 3.0})
        self.assertEqual(self.pcells()[0][:2], ("mylib", "bar"))
        self.assertEqual(self.width(), [3.0])
        h.call("save")
        buf = os.path.join(self.folder, "buf.gds")
        # A new helper reads it back as a live PCell of the project's library.
        h.call("open", refs=self.refs)
        self.assertEqual(self.pcells()[0][:2], ("mylib", "bar"))
        self.assertEqual(self.pcells()[0][2]["l"], 3.0)
        # The file changes: instances are made again; that is not an edit.
        time.sleep(0.01)
        with open(self.bar, "w") as f:
            f.write(bar(scale=2))
        os.utime(self.bar, (time.time() + 2, time.time() + 2))
        answer = h.call("refresh_pcells")
        self.assertTrue(answer["changed"])
        self.assertEqual(answer["libraries"], ["mylib"])
        self.assertEqual(self.width(), [6.0])
        h.call("info")
        h.settle()
        self.assertFalse(h.last_event("modified")["dirty"])
        # Export: plain geometry, no KLayout context.
        out = os.path.join(self.folder, "flat.gds")
        h.call("export", out=out)
        with open(out, "rb") as f:
            data = f.read()
        self.assertNotIn(b"$$$CONTEXT_INFO$$$", data)
        with open(buf, "rb") as f:
            self.assertIn(b"$$$CONTEXT_INFO$$$", f.read())
        flat = eval(h.call("python", code="PATH = %r\n" % out + READBACK))
        bar_cells = [v for k, v in flat.items() if k.startswith("bar")]
        self.assertEqual(len(bar_cells), 1)
        self.assertEqual(bar_cells[0][1], 1)

    def test_errors_name_file_and_line(self):
        h = self.helper
        with open(self.bar, "w") as f:
            f.write(bar().replace("self.cell.shapes", "undefined_name + self.cell.shapes"))
        os.utime(self.bar, (time.time() + 2, time.time() + 2))
        h.call("refresh_pcells")
        self.place("mylib", "bar", (7, 2))
        h.call("info")
        deadline = time.time() + 5
        notice = None
        while time.time() < deadline and notice is None:
            h.settle()
            notice = next((e for e in h.events if e.get("kind") == "notice"
                           and "bar.pcell.py:" in e.get("text", "")), None)
        self.assertIsNotNone(notice, [e for e in h.events if e.get("kind") == "notice"])
        line = BAR.splitlines().index(
            "        self.cell.shapes(self.body_layer).insert(pya.DBox(0, 0, self.l * SCALE, "
            "self.w))") + 1
        self.assertIn("bar.pcell.py:%d: NameError" % line, notice["text"])
        # A file that does not load: said once, with its line.
        with open(self.bar, "w") as f:
            f.write("import pya\n\nclass (:\n")
        os.utime(self.bar, (time.time() + 4, time.time() + 4))
        h.call("refresh_pcells")
        h.call("info")
        h.settle()
        texts = [e["text"] for e in h.events if e.get("kind") == "notice"]
        self.assertTrue(any("bar.pcell.py:3: SyntaxError" in t for t in texts), texts)


class InPlaceLibraryCells(Base):
    """A PCell inside an external file stays a PCell when that file is written back."""

    def setUp(self):
        self.start()
        self.helper.call("python", code=(
            "ly = pya.Layout(); ly.read(%r)\n"
            "c = ly.create_cell('CIRCLE', 'Basic', {'layer': pya.LayerInfo(1, 0), "
            "'actual_radius': 0.25})\n"
            "ly.cell('inv').insert(pya.DCellInstArray(c.cell_index(), pya.DTrans(0.5, 1.5)))\n"
            "ly.write(%r)" % ((os.path.join(self.folder, "inv.gds"),) * 2)))
        self.open(table(self.folder))

    def test_descend_edit_save_keeps_pcell(self):
        h = self.helper
        # The PCell inside inv is a library cell here too.
        self.assertTrue(self.py("[c for c in layout.each_cell() if c.is_pcell_variant()]"
                                "[0].is_library_cell()"))
        self.select_at(0.9, 0.5)
        h.call("descend")
        layer = next(l for l in h.last_event("layers")["layers"] if l["source"].startswith("2/0"))
        h.call("current_layer", id=layer["id"])
        h.call("mode", name="box")
        h.click(*self.to_pixel(0.1, 0.1))
        h.click(*self.to_pixel(0.3, 0.3))
        h.settle()
        report = h.call("save")
        inv = os.path.join(self.folder, "inv.gds")
        self.assertEqual([a["file"] for a in report["also"]], [inv])
        self.assertEqual(report["also"][0]["references"], {})
        found = self.py("ly = pya.Layout(); ly.read(%r)\n"
                        "[(i.cell.library().name(), i.pcell_parameters_by_name()['actual_radius']) "
                        "for i in ly.cell('inv').each_inst() if i.is_pcell()]" % inv)
        self.assertEqual(found, [("Basic", 0.25)])
        back = eval(h.call("python", code="PATH = %r\n" % inv + READBACK))
        self.assertEqual(sorted(k for k, v in back.items() if not v[0]), ["CIRCLE", "inv"])


PDK_KLAYOUT = os.environ.get("CDEPT_TEST_PDK_KLAYOUT",
                             "/opt/ext/OpenPDKs/ihp-sg13g2/libs.tech/klayout")


@unittest.skipUnless(os.path.isdir(os.path.join(PDK_KLAYOUT, "python", "sg13g2_pycell_lib")),
                     "IHP sg13g2 PDK not found (CDEPT_TEST_PDK_KLAYOUT)")
class PdkSaveReload(Base):
    """IHP SG13_dev PCells: placed, changed, saved and read back live."""

    def setUp(self):
        os.environ["KLAYOUT_PATH"] = PDK_KLAYOUT
        self.start()
        self.open(table(self.folder), technology="sg13g2")

    def tearDown(self):
        super().tearDown()
        os.environ.pop("KLAYOUT_PATH", None)

    def test_nmos_rsil_cmim(self):
        h = self.helper
        self.place("SG13_dev", "nmos", (7, 2), {"w": "1u"})
        self.place("SG13_dev", "rsil", (9, 4))
        self.place("SG13_dev", "cmim", (12, 2))
        names = sorted(n for _, n, _ in self.pcells())
        self.assertEqual(names, ["cmim", "nmos", "rsil"])
        props = self.select_at(7.1, 2.1)
        inst = next(o for o in props["objects"] if o.get("pcell"))
        self.assertEqual(inst["pcell"]["name"], "nmos")
        props = h.call("set_properties", index=inst["index"], key=inst["key"], object=inst,
                       pcell={"w": "2u"})
        nmos = [p for _, n, p in self.pcells() if n == "nmos"][0]
        self.assertEqual(nmos["w"], "2u")
        h.call("save")
        h.call("open", refs=table(self.folder), technology="sg13g2")
        back = {n: p for lib, n, p in self.pcells() if lib == "SG13_dev"}
        self.assertEqual(sorted(back), ["cmim", "nmos", "rsil"])
        self.assertEqual(back["nmos"]["w"], "2u")
        self.assertTrue(self.py("all(i.bbox().width() > 0 for i in layout.cell('buf').each_inst() "
                                "if i.is_pcell())"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
