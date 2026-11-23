# cdept-klayout tests: LEF abstracts, read-only (plain python3; run through ./run.sh).
# SPDX-License-Identifier: GPL-3.0-or-later
"""docs/KLAYOUT.md §9: a LEF file opens as a read-only layout; pins, obstructions and
the macro boundary are layers of their own."""

import os
import shutil
import tempfile
import unittest

from client import CallError, Helper

TECH_LEF = """VERSION 5.8 ;
UNITS
  DATABASE MICRONS 1000 ;
END UNITS
MANUFACTURINGGRID 0.005 ;
LAYER met1
  TYPE ROUTING ;
  DIRECTION HORIZONTAL ;
  PITCH 0.46 ;
  WIDTH 0.14 ;
END met1
LAYER via1
  TYPE CUT ;
END via1
LAYER met2
  TYPE ROUTING ;
  DIRECTION VERTICAL ;
  PITCH 0.46 ;
  WIDTH 0.14 ;
END met2
END LIBRARY
"""

LEF = """VERSION 5.8 ;
BUSBITCHARS "[]" ;
DIVIDERCHAR "/" ;
MACRO inv_1
  CLASS CORE ;
  ORIGIN 0 0 ;
  SIZE 1.38 BY 2.72 ;
  PIN A
    DIRECTION INPUT ;
    PORT
      LAYER met1 ;
        RECT 0.2 0.5 0.5 0.9 ;
    END
  END A
  PIN Y
    DIRECTION OUTPUT ;
    PORT
      LAYER met1 ;
        RECT 0.9 0.5 1.2 1.9 ;
    END
  END Y
  OBS
    LAYER met2 ;
      RECT 0.1 0.1 1.3 0.4 ;
  END
END inv_1
MACRO buf_1
  CLASS CORE ;
  SIZE 2.3 BY 2.72 ;
  PIN A
    DIRECTION INPUT ;
    PORT
      LAYER met1 ;
        RECT 0.2 0.5 0.5 0.9 ;
    END
  END A
END buf_1
END LIBRARY
"""

PDK = os.environ.get("PDK_ROOT", "/opt/ext/OpenPDKs")


def table(lef, tech=None):
    return {"root": lef, "files": {lef: {"label": "t/inv/abstract", "read_only": True}},
            "lef": {"tech_lef": tech} if tech else {}}


LAYER_NAMES = (
    "n = []\nit = view.begin_layers()\n"
    "while not it.at_end():\n n.append(it.current().name); it.next()\nn")


class LefAbstract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="cdk-lef-")
        cls.lef = os.path.join(cls.folder, "cells.lef")
        cls.tech = os.path.join(cls.folder, "cells_tech.lef")
        with open(cls.lef, "w") as f:
            f.write(LEF)
        with open(cls.tech, "w") as f:
            f.write(TECH_LEF)
        cls.before = os.listdir(cls.folder)
        cls.helper = Helper(cls.folder)

    @classmethod
    def tearDownClass(cls):
        cls.helper.close()
        shutil.rmtree(cls.folder, ignore_errors=True)

    def names(self):
        return eval(self.helper.call("python", code=LAYER_NAMES))

    def test_opens_read_only_with_one_layer_per_purpose(self):
        value = self.helper.call("open", refs=table(self.lef, self.tech), editable=True)
        self.assertIn(value["top"], ("inv_1", "buf_1"))
        names = self.names()
        for want in ("met1 pin", "met2 obstruction", "macro boundary"):
            self.assertIn(want, names)
        cells = eval(self.helper.call("python", code="sorted(c.name for c in layout.each_cell())"))
        self.assertEqual(cells, ["buf_1", "inv_1"])
        # Not editable even though the caller said so.
        self.assertEqual(self.helper.call("python", code="view.is_editable()"), "False")
        # The macro boundary has the macro's size.
        box = self.helper.call("python", code="str(layout.cell('inv_1').dbbox())")
        self.assertIn("(0,0;1.38,2.72)", box)

    def test_pins_and_obstructions_hold_the_shapes(self):
        self.helper.call("open", refs=table(self.lef, self.tech))
        counts = eval(self.helper.call("python", code=(
            "r = {}\n"
            "for li in layout.layer_indexes():\n"
            "    r[layout.get_info(li).name] = layout.cell('inv_1').shapes(li).size()\n"
            "r")))
        pins = [n for n in counts if n.endswith(".PIN") and counts[n]]
        obs = [n for n in counts if n.endswith(".OBS") and counts[n]]
        self.assertEqual(pins, ["met1.PIN"])
        self.assertEqual(counts["met1.PIN"], 2)
        self.assertEqual(obs, ["met2.OBS"])

    def test_save_and_export_are_refused_and_the_folder_is_untouched(self):
        self.helper.call("open", refs=table(self.lef, self.tech))
        for method, params in (("save", {}), ("export", {"out": os.path.join(self.folder, "x.gds")})):
            with self.assertRaises(CallError) as e:
                self.helper.call(method, **params)
            self.assertIn("LEF abstract: read-only", str(e.exception))
        self.assertFalse(os.path.exists(os.path.join(self.folder, "x.gds")))
        self.assertFalse(os.path.exists(self.lef + ".json"))

    def test_render_one_macro(self):
        out = os.path.join(self.folder, "buf.png")
        self.helper.call("render", refs=table(self.lef, self.tech), cell="buf_1",
                         width=200, height=150, out=out)
        self.assertTrue(os.path.getsize(out) > 100)
        with self.assertRaises(CallError):
            self.helper.call("render", refs=table(self.lef, self.tech), cell="nope",
                             width=200, height=150, out=out)

    def test_without_a_tech_lef_the_layers_still_come(self):
        self.helper.call("open", refs=table(self.lef))
        self.assertIn("met1 pin", self.names())


@unittest.skipUnless(os.path.isdir(os.path.join(PDK, "ihp-sg13g2/libs.ref/sg13g2_stdcell/lef")),
                     "needs the IHP PDK")
class IhpStandardCells(unittest.TestCase):
    """The technology's layer map and layer properties give the PDK's layers and colours."""

    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="cdk-lef-ihp-")
        base = os.path.join(PDK, "ihp-sg13g2")
        os.environ["KLAYOUT_PATH"] = os.path.join(base, "libs.tech/klayout")
        cls.lib = os.path.join(base, "libs.ref/sg13g2_stdcell/lef")
        cls.helper = Helper(cls.folder)

    @classmethod
    def tearDownClass(cls):
        cls.helper.close()
        os.environ.pop("KLAYOUT_PATH", None)
        shutil.rmtree(cls.folder, ignore_errors=True)

    def test_pdk_numbers_and_colours(self):
        lef = os.path.join(self.lib, "sg13g2_stdcell.lef")
        lyp = os.path.join(PDK, "ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyp")
        value = self.helper.call("open", technology="sg13g2", layer_properties=lyp,
                                 refs=table(lef, os.path.join(self.lib, "sg13g2_tech.lef")))
        self.assertTrue(value["top"])
        sources = eval(self.helper.call("python", code=(
            "r = {}\nit = view.begin_layers()\n"
            "while not it.at_end():\n"
            " n = it.current(); r[n.name] = (n.source.split('@')[0].split(' ')[-1], n.fill_color); it.next()\n"
            "r")))
        self.assertEqual(sources["Metal1 pin"][0], "8/2")
        self.assertEqual(sources["Metal1 obstruction"][0], "8/4")
        self.assertEqual(sources["macro boundary"][0], "189/4")
        # The PDK's colour of Metal1 (its layer properties), not a generated one.
        self.assertNotEqual(sources["Metal1 pin"][1], 0)


if __name__ == "__main__":
    unittest.main()
