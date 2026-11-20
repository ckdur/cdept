# PCell "rpoly" for KLayout, registered by CDePT in the KLayout library named after
# this CDePT library. Conventions (docs/KLAYOUT.md §3.6):
# - the file is <cell>.pcell.py in the cell's pcell view; the PCell is named after the cell;
# - it defines one subclass of pya.PCellDeclarationHelper (or names it: PCELL = ...);
# - saving this file updates every layout open in CDePT that places the PCell.
import pya


class Rpoly(pya.PCellDeclarationHelper):
    def __init__(self):
        super().__init__()
        self.param("body", self.TypeLayer, "Body layer", default=pya.LayerInfo(1, 0))
        self.param("contact", self.TypeLayer, "Contact layer", default=pya.LayerInfo(2, 0))
        self.param("l", self.TypeDouble, "Length", default=2.0, unit="µm")
        self.param("w", self.TypeDouble, "Width", default=0.5, unit="µm")
        self.param("c", self.TypeDouble, "Contact size", default=0.3, unit="µm")

    def display_text_impl(self):
        return "rpoly(l=%g, w=%g)" % (self.l, self.w)

    def coerce_parameters_impl(self):
        self.w = max(self.w, self.c)
        self.l = max(self.l, 2 * self.c)

    def produce_impl(self):
        l, w, c = self.l, self.w, self.c
        self.cell.shapes(self.body_layer).insert(pya.DBox(0, 0, l, w))
        y = (w - c) / 2
        for x in (0, l - c):
            self.cell.shapes(self.contact_layer).insert(pya.DBox(x, y, x + c, y + c))


PCELL = Rpoly
