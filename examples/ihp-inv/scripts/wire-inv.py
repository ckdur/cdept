# Wires the generated inverter of this example a little (docs/LINKING.md §8): the
# gates (A) and the drains (Y) are joined and labelled, VDD and VSS are left open, so
# that `cdept layout check-nets cells inv` and the GUI's flylines have something to
# show. Run by KLayout with the PDK's libraries (they register at start-up):
#
#   KLAYOUT_PATH=$PDK_ROOT/ihp-sg13g2/libs.tech/klayout klayout -z -nc \
#       -r scripts/wire-inv.py -rd file=libs/cells/inv/layout/inv.gds [-rd short=1]
#
# short=1 also joins A and Y (a deliberate short, for tests).
import pya

GATPOLY, CONT, METAL1, METAL1_TEXT = (5, 0), (6, 0), (8, 0), (8, 25)

ly = pya.Layout()
ly.technology_name = "sg13g2"
ly.read(file)  # noqa: F821  (-rd file=...)
top = ly.top_cell()
layer = {k: ly.layer(*k) for k in (GATPOLY, CONT, METAL1, METAL1_TEXT)}


def path(key, points, width):
    top.shapes(layer[key]).insert(pya.DPath([pya.DPoint(*p) for p in points], width))


def box(key, l, b, r, t):
    top.shapes(layer[key]).insert(pya.DBox(l, b, r, t))


# A: from the nmos gate down-left, up, and into the pmos gate; a contact to Metal1.
path(GATPOLY, [(0.405, 1.0), (0.405, 1.6), (-0.3, 1.6), (-0.3, 2.8), (0.715, 2.8),
               (0.715, 3.3)], 0.13)
box(GATPOLY, -0.45, 2.05, -0.15, 2.35)
box(CONT, -0.38, 2.12, -0.22, 2.28)
box(METAL1, -0.5, 2.0, -0.1, 2.4)
# Y: Metal1 from the nmos drain to the pmos drain.
path(METAL1, [(0.64, 0.55), (0.64, 1.9), (0.95, 1.9), (0.95, 4.16)], 0.16)
if globals().get("short"):
    path(METAL1, [(-0.3, 2.2), (0.95, 2.2)], 0.16)

# The labels A and Y on the wires (the generated ones are in a column to the left).
texts = top.shapes(layer[METAL1_TEXT])
for shape in list(texts.each()):
    if shape.is_text() and shape.text_string in ("A", "Y"):
        texts.erase(shape)
texts.insert(pya.DText("A", pya.DTrans(pya.DVector(-0.3, 2.2))))
texts.insert(pya.DText("Y", pya.DTrans(pya.DVector(0.95, 2.6))))
ly.write(file)  # noqa: F821
