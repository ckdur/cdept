# cdept-klayout tests: the technology scan script (plain python3; run through ./run.sh).
# SPDX-License-Identifier: GPL-3.0-or-later
"""`techscan.py` is run by KLayout on its own (not by the helper) and writes the PCell
libraries KLayout registers as JSON."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest

from client import DEFAULT_KLAYOUT

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(os.path.dirname(HERE), "helper", "cdept_klayout", "techscan.py")


def klayout_command(folder, path):
    """argv and environment that run KLayout on its own with `path` on KLAYOUT_PATH
    (in the docker image when CDEPT_TEST_DOCKER names one, like the helper's tests)."""
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", KLAYOUT_PATH=path,
               KLAYOUT_HOME=os.path.join(folder, "klayout-home"))
    image = os.environ.get("CDEPT_TEST_DOCKER")
    if image:
        mounts = []
        for mounted in sorted({folder, os.path.dirname(SCRIPT)}):
            mounts += ["-v", "%s:%s" % (mounted, mounted)]
        return ["docker", "run", "--rm", "-i", "--user", "%d:%d" % (os.getuid(), os.getgid()),
                "-e", "QT_QPA_PLATFORM=offscreen", "-e", "HOME=/tmp",
                "-e", "KLAYOUT_PATH=" + path] + mounts + ["--entrypoint", "klayout", image], env
    return [os.environ.get("CDEPT_TEST_KLAYOUT") or DEFAULT_KLAYOUT], env

# A technology with a PCell library registered by an autorun macro, the way a PDK
# does it.
LYT = """<?xml version="1.0" encoding="utf-8"?>
<technology>
 <name>scantech</name>
 <description>Scan test technology</description>
 <dbu>0.002</dbu>
 <layer-properties_file/>
</technology>
"""

MACRO = """<?xml version="1.0" encoding="utf-8"?>
<klayout-macro>
 <description>register the scan test library</description>
 <autorun>true</autorun>
 <autorun-early>false</autorun-early>
 <show-in-menu>false</show-in-menu>
 <interpreter>python</interpreter>
 <text>
import pya

class Res(pya.PCellDeclarationHelper):
    def __init__(self):
        super(Res, self).__init__()
        self.param("w", self.TypeDouble, "Geometry\\tWidth", default=0.5, unit="um")
        self.param("n", self.TypeInt, "Fingers", default=2)
        self.param("ring", self.TypeBoolean, "Guard ring", default=True)
        self.param("kind", self.TypeString, "Kind", default="poly",
                   choices=[["Poly", "poly"], ["Metal", "metal"]])
        self.param("area", self.TypeDouble, "Area", default=0.0, readonly=True)
        self.param("secret", self.TypeString, "Secret", default="x", hidden=True)
        self.param("l", self.TypeLayer, "Layer", default=pya.LayerInfo(1, 0))

    def display_text_impl(self):
        return "Res(w=%s)" % self.w

    def produce_impl(self):
        pass

class ScanLib(pya.Library):
    def __init__(self):
        self.description = "Scan test PCells"
        self.technology = "scantech"
        self.layout().register_pcell("res", Res())
        self.layout().create_cell("fixed_cell")
        self.register("ScanLib")

ScanLib()
 </text>
</klayout-macro>
"""


class TechScan(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="cdk-scan-")
        path = os.path.join(cls.folder, "path")
        os.makedirs(os.path.join(path, "tech"))
        os.makedirs(os.path.join(path, "pymacros"))
        with open(os.path.join(path, "tech", "scantech.lyt"), "w") as f:
            f.write(LYT)
        with open(os.path.join(path, "pymacros", "scanlib.lym"), "w") as f:
            f.write(MACRO)
        cls.out = os.path.join(cls.folder, "out.json")
        command, env = klayout_command(cls.folder, path)
        result = subprocess.run(
            command + ["-zz", "-nc", "-r", SCRIPT, "-rd", "out_file=" + cls.out],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        cls.printed = result.stdout.decode("utf-8", "replace")
        cls.status = result.returncode

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.folder, ignore_errors=True)

    def answer(self):
        self.assertTrue(os.path.exists(self.out),
                        "no answer (status %s): %s" % (self.status, self.printed))
        with open(self.out) as f:
            return json.load(f)

    def test_technologies(self):
        answer = self.answer()
        self.assertIn("KLayout", answer["klayout"])
        tech = [t for t in answer["technologies"] if t["name"] == "scantech"]
        self.assertEqual(len(tech), 1, answer["technologies"])
        self.assertAlmostEqual(tech[0]["dbu"], 0.002)
        self.assertEqual(tech[0]["description"], "Scan test technology")

    def test_library_and_parameters(self):
        answer = self.answer()
        self.assertEqual(answer["errors"], [])
        names = [l["name"] for l in answer["libraries"]]
        self.assertIn("Basic", names)
        lib = next(l for l in answer["libraries"] if l["name"] == "ScanLib")
        self.assertEqual(lib["description"], "Scan test PCells")
        self.assertEqual(lib["technologies"], ["scantech"])
        self.assertEqual(lib["cells"], ["fixed_cell"])
        self.assertEqual([p["name"] for p in lib["pcells"]], ["res"])
        params = {p["name"]: p for p in lib["pcells"][0]["parameters"]}
        w = params["w"]
        self.assertEqual((w["type"], w["default"], w["unit"]), ("double", "0.5", "um"))
        self.assertEqual((w["group"], w["description"]), ("Geometry", "Width"))
        self.assertEqual((params["n"]["type"], params["n"]["default"]), ("int", "2"))
        self.assertEqual((params["ring"]["type"], params["ring"]["default"]),
                         ("boolean", "true"))
        self.assertEqual(params["kind"]["choices"],
                         [{"label": "Poly", "value": "poly"},
                          {"label": "Metal", "value": "metal"}])
        self.assertTrue(params["area"]["readonly"])
        self.assertTrue(params["secret"]["hidden"])
        self.assertFalse(params["w"]["hidden"])
        self.assertEqual(params["l"]["type"], "layer")
        self.assertEqual(params["l"]["default"], "1/0")


if __name__ == "__main__":
    unittest.main()
