# cdept-klayout tests: DRC/LVS markers (plain python3; run through ./run.sh).
# SPDX-License-Identifier: GPL-3.0-or-later
"""`markers` draws highlights over the view and zooms to them."""

import shutil
import tempfile
import unittest

from client import CallError, Helper
from test_helper import FIXTURES, table


class Markers(unittest.TestCase):
    W, H = 400, 300

    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="cdk-")
        cls.helper = Helper(cls.folder, display=True)
        cls.helper.call("python", code="FOLDER = %r\n" % cls.folder + FIXTURES)
        cls.helper.send({"type": "resize", "width": cls.W, "height": cls.H})
        cls.helper.call("open", refs=table(cls.folder))
        cls.helper.settle()

    @classmethod
    def tearDownClass(cls):
        cls.helper.close()
        shutil.rmtree(cls.folder, ignore_errors=True)

    def centre(self):
        h = self.helper
        h.settle()
        data = h.pixels(h.frames[-1])
        i = ((self.H // 2) * self.W + self.W // 2) * 4
        return data[i:i + 4]

    def test_markers_zoom_and_clear(self):
        h = self.helper
        before = self.centre()
        # A box in empty space (above buf), zoomed to: its centre is the marker.
        value = h.call("markers", color="#ff0000", zoom=True, shapes=[
            {"kind": "box", "box": [20, 20, 21, 21]},
            {"kind": "polygon", "points": [[20, 20], [21, 20], [20.5, 21]]},
            {"kind": "edge", "points": [[20, 20], [21, 21]]},
            {"kind": "text", "at": [20.5, 20.5], "text": "M1.a"},
            {"kind": "nonsense"},
        ])
        self.assertEqual(value["markers"], 4)
        box = eval(h.call("python", code="b = view.box()\n(b.left, b.bottom, b.right, b.top)"))
        self.assertLess(box[0], 20)
        self.assertGreater(box[2], 21)
        self.assertLess(box[2] - box[0], 10, "zoomed to the markers")
        marked = self.centre()
        b, g, r = marked[0], marked[1], marked[2]  # frames are B G R A
        self.assertGreater(r, 100, "marker colour at the centre: %r" % (marked,))
        self.assertLess(g, 100)
        self.assertNotEqual(marked, before)

        self.assertEqual(h.call("markers", shapes=[])["markers"], 0)
        cleared = self.centre()
        self.assertNotEqual(cleared, marked)

    def test_markers_need_a_layout(self):
        folder = tempfile.mkdtemp(prefix="cdk-")
        h = Helper(folder)
        try:
            with self.assertRaises(CallError):
                h.call("markers", shapes=[{"kind": "box", "box": [0, 0, 1, 1]}])
        finally:
            h.close()
            shutil.rmtree(folder, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
