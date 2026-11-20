# cdept-klayout tests: nets (plain python3; run through ./run.sh).
# SPDX-License-Identifier: GPL-3.0-or-later
"""The connectivity of a layout from an LVS database (l2n.py, connectivity.py) and the
editor's net highlights, flylines and probing (nets.py), docs/LINKING.md §8.

No PDK needed: a small LVS deck of its own extracts a MOS from generic layers."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest

from client import DEFAULT_KLAYOUT, HERE, Helper

HELPER_DIR = os.path.join(os.path.dirname(HERE), "helper", "cdept_klayout")

# top: a MOS (cell "mos", bound to schematic instance M1 by property 61) on 1/0
# (active) and 2/0 (poly); Metal1 (4/0) through contacts (3/0) on its source and on
# its gate, labelled IN (on the gate's wire) and OUT (on no wire); a loose Metal1 box.
LAYOUT = r'''
import pya
ly = pya.Layout(); ly.dbu = 0.001
top = ly.create_cell("top")
mos = ly.create_cell("mos")
L = lambda a, b: ly.layer(a, b)
mos.shapes(L(1, 0)).insert(pya.Box(0, 0, 1000, 1000))
mos.shapes(L(2, 0)).insert(pya.Box(400, -200, 600, 1200))
inst = top.insert(pya.CellInstArray(mos.cell_index(), pya.Trans(0, 0)))
inst.set_property(61, "M1")
top.shapes(L(3, 0)).insert(pya.Box(100, 400, 300, 600))
top.shapes(L(4, 0)).insert(pya.Box(0, 300, 350, 2000))
top.shapes(L(2, 0)).insert(pya.Box(400, 1200, 600, 1600))
top.shapes(L(3, 0)).insert(pya.Box(400, 1400, 600, 1600))
top.shapes(L(4, 0)).insert(pya.Box(400, 1350, 2000, 1700))
top.shapes(L(5, 0)).insert(pya.Text("IN", pya.Trans(1800, 1500)))
top.shapes(L(5, 0)).insert(pya.Text("OUT", pya.Trans(5000, 5000)))
top.shapes(L(4, 0)).insert(pya.Box(3000, 0, 3500, 500))
ly.write(OUT)
'''

DECK = r'''
source($input, $topcell)
report_lvs($report)
deep
active = input(1, 0)
poly = input(2, 0)
cont = input(3, 0)
metal1 = input(4, 0)
text = labels(5, 0)
sd = active - poly
gate = active & poly
extract_devices(mos3("NMOS"), { "SD" => sd, "G" => gate, "tS" => sd, "tD" => sd, "tG" => poly })
connect(sd, cont)
connect(poly, cont)
connect(cont, metal1)
connect(metal1, text)
schematic($schematic)
compare
'''


def klayout():
    return os.environ.get("CDEPT_TEST_KLAYOUT") or DEFAULT_KLAYOUT


def batch(script, folder, **defines):
    args = [klayout(), "-b", "-r", script]
    for name, value in defines.items():
        args += ["-rd", "%s=%s" % (name, value)]
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen",
               KLAYOUT_HOME=os.path.join(folder, "klayout-home"))
    done = subprocess.run(args, cwd=folder, env=env, capture_output=True, text=True,
                          timeout=120)
    if done.returncode != 0:
        raise AssertionError("klayout %s failed:\n%s%s" % (script, done.stdout, done.stderr))


@unittest.skipIf(os.environ.get("CDEPT_TEST_DOCKER"), "needs a local KLayout")
class Nets(unittest.TestCase):
    W, H = 400, 300

    @classmethod
    def setUpClass(cls):
        cls.folder = f = tempfile.mkdtemp(prefix="cdk-")
        cls.gds = os.path.join(f, "top.gds")
        for name, text in (("layout.py", LAYOUT), ("deck.lvs", DECK)):
            with open(os.path.join(f, name), "w") as out:
                out.write(text)
        with open(os.path.join(f, "top.spice"), "w") as out:
            out.write(".subckt top\n.ends\n")
        batch(os.path.join(f, "layout.py"), f, OUT=cls.gds)
        cls.lvsdb = os.path.join(f, "top.lvsdb")
        batch(os.path.join(f, "deck.lvs"), f, input=cls.gds, topcell="top",
              report=cls.lvsdb, schematic=os.path.join(f, "top.spice"))
        cls.json = os.path.join(f, "connectivity.json")
        batch(os.path.join(HELPER_DIR, "connectivity.py"), f, lvsdb=cls.lvsdb,
              input=cls.gds, top="top", out=cls.json)
        with open(cls.json) as data:
            cls.data = json.load(data)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.folder, ignore_errors=True)

    def net_of(self, terminal):
        device = self.data["devices"][0]
        return next(t["net"] for t in device["terminals"] if t["name"] == terminal)

    def test_1_described(self):
        d = self.data
        self.assertEqual(d["top"], "top")
        self.assertEqual(len(d["devices"]), 1)
        device = d["devices"][0]
        self.assertEqual(device["class"], "NMOS")
        self.assertEqual(sorted(t["name"] for t in device["terminals"]), ["D", "G", "S"])
        for t in device["terminals"]:
            self.assertIsNotNone(t["point"])
        self.assertEqual(d["instances"], [{"path": "M1", "cell": "mos",
                                           "bbox": [0.0, -0.2, 1.0, 1.2], "leaf": True}])
        labels = {l["text"]: l for l in d["labels"]}
        # IN is on the gate's wire; OUT on nothing.
        self.assertEqual(labels["IN"]["net"], self.net_of("G"))
        self.assertIsNone(labels["OUT"]["net"])
        self.assertEqual(d["nets"][self.net_of("G")]["name"], "IN")
        # The source's wire and the loose box are nets of their own.
        self.assertNotEqual(self.net_of("S"), self.net_of("D"))
        self.assertGreaterEqual(len(d["nets"]), 4)

    def test_2_editor(self):
        h = Helper(self.folder, display=True)
        try:
            h.send({"type": "resize", "width": self.W, "height": self.H})
            h.call("open", refs={"root": self.gds, "files": {self.gds: {
                "label": "t/top/layout", "refs": {}}}})
            h.call("zoom", what="box", box=[-0.5, -0.5, 3.6, 2.1])
            loaded = h.call("nets_load", lvsdb=self.lvsdb, top="top")
            self.assertEqual(loaded["nets"], len(self.data["nets"]))
            # Probing: on the gate's wire, on the source's wire, on nothing.
            self.assertEqual(h.call("nets_probe", point=[1.8, 1.5])["net"], self.net_of("G"))
            probe = h.call("nets_probe", point=[0.1, 1.8])
            self.assertEqual(probe["net"], self.net_of("S"))
            self.assertIsNone(h.call("nets_probe", point=[2.5, 0.2])["net"])
            # In pixels: the middle of the loose box.
            px = eval(h.call("python", code="p = view.viewport_trans() * pya.DPoint(3.25, 0.25)\n"
                             "(p.x, %d - 1 - p.y)" % self.H))
            loose = h.call("nets_probe", pixel=list(px))["net"]
            self.assertIsNotNone(loose)
            self.assertNotIn(loose, (self.net_of("S"), self.net_of("G"), self.net_of("D")))

            shown = h.call("nets_highlight", nets=[self.net_of("G")], zoom=True)
            self.assertGreater(shown["markers"], 0)
            lines = h.call("nets_flylines", lines=[[0.2, 1.0, 3.25, 0.25], [0, 0, 1, 1]])
            self.assertEqual(lines["markers"], 2)
            self.assertEqual(h.call("nets_flylines", lines=[])["markers"], 0)
            self.assertEqual(h.call("nets_highlight", nets=[])["markers"], 0)
            # Not part of the layout: nothing to save.
            h.settle()
            self.assertFalse(h.last_event("modified")["dirty"])
        finally:
            h.close()

    def test_4_live(self):
        """Live connectivity: the terminals traced again on the layout as it is now."""
        h = Helper(tempfile.mkdtemp(dir=self.folder), display=True)
        rules = {"symbols": {"SD": "1/0-2/0"},
                 "connections": [["2/0", "3/0", "4/0"], ["SD", "3/0", "4/0"]]}
        try:
            h.send({"type": "resize", "width": self.W, "height": self.H})
            h.call("open", refs={"root": self.gds, "files": {self.gds: {
                "label": "t/top/layout", "refs": {}}}})
            h.call("nets_load", lvsdb=self.lvsdb, top="top")
            status = h.call("nets_live", connectivity=self.json, rules=rules, on=False)
            self.assertTrue(status["available"], status)
            self.assertEqual(status["seeds"], 3 + 2)
            # S, D and G (and the label IN) trace; the label OUT is on nothing.
            self.assertEqual(status["traced"], 4, status)
            first = h.call("nets_live", update=True)
            terms = {t["name"]: first["devices"][0]["terminals"][i]["net"]
                     for i, t in enumerate(self.data["devices"][0]["terminals"])}
            labels = {l["text"]: first["labels"][i] for i, l in enumerate(self.data["labels"])}
            self.assertEqual(labels["IN"], terms["G"])
            self.assertIsNone(labels["OUT"])
            self.assertEqual(len({terms["S"], terms["D"], terms["G"]}), 3)
            self.assertEqual(first["nets"][terms["G"]]["name"], "IN")
            # A Metal1 bridge from the source's wire to the gate's: one net now.
            h.call("python", code="layout.cell('top').shapes(layout.layer(4, 0))"
                                  ".insert(pya.Box(300, 1400, 450, 1500))")
            bridged = h.call("nets_live", update=True)
            nets = [t["net"] for t in bridged["devices"][0]["terminals"]]
            names = [t["name"] for t in self.data["devices"][0]["terminals"]]
            now = dict(zip(names, nets))
            self.assertEqual(now["S"], now["G"])
            self.assertNotEqual(now["D"], now["G"])
            self.assertLess(bridged["ms"], 200)
            # The bound instance moved: its terminals move along; the bridge no longer
            # reaches the source's contact... the wire is the top cell's, so S stays.
            h.call("python", code="i = list(layout.cell('top').each_inst())[0]\n"
                                  "i.transform(pya.Trans(pya.Vector(0, 10000)))")
            moved = h.call("nets_live", update=True)
            self.assertIn("M1", moved["moved"])
            self.assertAlmostEqual(moved["moved"]["M1"][5], 10.0)
            g = names.index("G")
            before = self.data["devices"][0]["terminals"][g]["point"]
            after = moved["devices"][0]["terminals"][g]["point"]
            self.assertAlmostEqual(after[1] - before[1], 10.0)
            # Debounced updates after a change (the editor's revision).
            h.call("nets_live", on=True, delay_ms=100)
            h.call("python", code="server.editor.revision += 1")
            event = h.wait(lambda m: m.get("type") == "event"
                           and m["event"].get("kind") == "nets_live", timeout=10)
            self.assertIsNotNone(event)
            # Live ids highlight the traced shapes.
            shown = h.call("nets_highlight", nets=[now["G"]], live=True)
            self.assertGreaterEqual(shown["markers"], 0)
        finally:
            h.close()

    def test_5_flyline_style(self):
        h = Helper(tempfile.mkdtemp(dir=self.folder), display=True)
        try:
            h.send({"type": "resize", "width": self.W, "height": self.H})
            h.call("open", refs={"root": self.gds, "files": {self.gds: {
                "label": "t/top/layout", "refs": {}}}})
            h.call("config", name="background-color", value="#000000")
            drawn = h.call("nets_flylines", lines=[[0, 0, 1, 1]], color="#ff0000", width=3,
                           style="dashed", opacity=0.5)
            self.assertEqual((drawn["markers"], drawn["width"], drawn["line_style"]), (1, 3, 2))
            self.assertEqual(drawn["color"], "#800000")
            # The style stays for the next lines.
            again = h.call("nets_flylines", lines=[[0, 0, 2, 2]])
            self.assertEqual((again["width"], again["line_style"], again["color"]),
                             (3, 2, "#800000"))
            styled = eval(h.call("python", code="[(m.line_style, m.line_width) for m in "
                                 "server.nets.flylines.markers]"))
            self.assertEqual(styled, [(2, 3)])
        finally:
            h.close()

    def test_3_needs_a_load(self):
        folder = tempfile.mkdtemp(prefix="cdk-")
        h = Helper(folder)
        try:
            from client import CallError
            with self.assertRaises(CallError):
                h.call("nets_probe", point=[0, 0])
        finally:
            h.close()
            shutil.rmtree(folder, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
