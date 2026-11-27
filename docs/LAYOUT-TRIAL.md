# Layout trial: an inverter routed by hand in CDePT

2026-11-28, IHP SG13G2, `examples/ihp-inv`, CDePT's embedded KLayout.

## Verdict

**It works.** From Generate from Schematic, using only what the layout tab offers, the
inverter reaches **DRC clean** (the PDK's `ihp-sg13g2.drc`), **LVS match** (`sg13g2.lvs`,
no options; extracted: one `sg13_lv_nmos` W=0.74u L=0.13u, one `sg13_lv_pmos` W=1.5u
L=0.13u) and a clean net check (4 ok). A replay drove the real window through it. No
person has done it by hand yet.

It needed one fix first: without a window, KLayout's grid was 1 nm, with no setting in
CDePT, so hand-drawn corners landed off the PDKs' 5 nm grid. The editor now sets
5 nm (`cdept-klayout/helper/cdept_klayout/editor.py`). A second fix is for comfort: the
layer list (378 layers, 13 rows visible) has a "Find a layer" box now
(`cdept-gui/src/views/layout/panels.rs`).

## The recipe

All numbers are micrometres; a box is given as left, bottom, right, top.

**Drawing a box exactly.** Pick the layer in the Layers list, choose **Box** in the
toolbar, click one corner, click the opposite one (dragging works too). The status line
under the picture shows the pointer's x and y. Clicks snap to 5 nm, but one pixel must
be smaller than that: zoom in with the wheel on each corner, also between the two
clicks. Another way: draw roughly, select the box, press **Q** and type its edges.

**Moving a part exactly.** **Select** mode, click the part, press **Q**, type `x` and
`y`, OK.

### 0. Generate

Layout > Generate / Update from Schematic. It places the nmos at (0, 0.18), the pmos at
(0.31, 3.41) and four pins in a column at x = -2.25. DRC now shows 5 violations: LU.b
(no substrate tie) and Pin.e (pins without metal).

### 1. Align the gates

Select the pmos: `x` = 0, `y` = 3.41. Both gates are now at x 0.34 to 0.47, and both
pairs of source/drain Metal1 bars at x 0.07-0.23 and 0.58-0.74.

### 2. Two ties and the gate contact (Layout > Place Library Cell, library `SG13_dev`, cell `via_stack`)

Click anywhere free to place, return to Select mode, set the position with Q.

| Part | b_layer | t_layer | vn_columns × vn_rows | x, y |
|---|---|---|---|---|
| substrate tie | Activ | Metal1 | 2 × 2 | 0.15, -0.7 |
| n-well tie | Activ | Metal1 | 2 × 2 | 0.15, 5.8 |
| gate contact | GatPoly | Metal1 | 1 × 1 | 0.0, 2.1 |

**Do not use the PDK's `ptap1` and `ntap1` cells** for the ties: the LVS deck extracts
each as a device of its own, which the schematic does not have, and LVS fails.

### 3. Boxes

| Layer | Box | What for |
|---|---|---|
| `GatPoly.drawing` (5/0) | 0.34, 1.1, 0.47, 3.23 | joins the two gates |
| `GatPoly.drawing` | 0.15, 1.95, 0.34, 2.25 | gate to the contact |
| `pSD.drawing` (14/0) | -0.27, -1.12, 0.57, -0.28 | makes the lower tie p+ (a substrate tie) |
| `NWell.drawing` (31/0) | -0.45, 3.1, 1.12, 6.4 | one well round the pmos and its tie |
| `Metal1.drawing` (8/0) | -0.2, 1.95, 0.15, 2.25 | pad of A (the contact's own metal is under the minimum area) |
| `Metal1.drawing` | 0.58, 0.92, 0.74, 3.41 | Y: drain to drain |
| `Metal1.drawing` | 0.07, -0.6, 0.23, 0.18 | VSS: nmos source to its tie |
| `Metal1.drawing` | 0.07, 4.91, 0.23, 5.65 | VDD: pmos source to its tie |

The two straps run into the middle of the ties' metal on purpose: a strap that only
partly overlaps a tie's metal gives M1.a or M1.b.

### 4. Pins

Layout > Pins Panel. **Delete** the four generated pins. Then for each: pick the
current layer `Metal1.drawing`, give name and direction under "New pin", press
**Draw**, draw the box. A pin box must lie inside Metal1 (Pin.e).

| Pin | Direction | Box |
|---|---|---|
| A | input | -0.15, 2.0, 0.1, 2.2 |
| Y | output | 0.58, 2.0, 0.74, 2.3 |
| VSS | inout | 0.0, -0.8, 0.3, -0.55 |
| VDD | inout | 0.0, 5.7, 0.3, 5.95 |

The labels LVS reads come with the pins; no Text mode is needed.

### 5. Check

Save (Ctrl+S). Layout > Update Connectivity: "4 ok, 0 open, 0 shorted". **DRC** button:
"DRC clean". **LVS** button: "The layout matches the schematic."

## What a person will stumble over

Open (not fixed):

1. **No path width and no text setting.** Path mode draws 0.1 µm wide paths (below the
   Metal1 minimum 0.16 and the gate length 0.13) and Text mode writes "ABC". Both can
   be changed afterwards in Properties (Q). Boxes avoid the problem.
2. **No editor options at all**: grid, snapping to objects (off), angle constraint
   (any).
3. **The picture changes size** when the Pins, Nets or check panels open (they take
   height from it; with all three, little is left on a 1600×1000 window), and the side
   panel can grow wider when the Selection list gets a long line.
4. **The generated pins land in a column** away from the devices; the recipe deletes and
   draws them again. Moving them instead was not tried.
5. **The ties have to be known**: no tie or guard-ring helper, and the cells that look
   right (`ptap1`, `ntap1`) fail LVS against a schematic without them.
6. **Layer names**: a beginner has to know that the p+ implant is `pSD` and that boxes
   go on the `.drawing` layers.
7. DRC here is the default deck's cell-level check; density rules were not looked at.

## What this means for the tutorials

I6 and I7 can be written from the recipe. A guided version cannot point at places in
the layout picture; it would give coordinates (and the status line shows them), and
tick steps off from checks of the layout (parts placed, nets joined, DRC and LVS
results).

A mistake to plant for I7, seen in the trial: end the VSS strap at -0.4 instead of -0.6
(just short of the tie's metal). The net check reports VSS open, DRC reports M1.b (a
notch/space under 0.18) and LVS does not match.

## The five-transistor OTA

`cdept layout link cells ota5t` works: 6 transistors and 6 pins placed; DRC 7
violations of the same two kinds (LU.b, Pin.e); net check 2 ok, 6 open. Routing it was
**not** attempted. It is much more work than the inverter:

- the transistors are single fingers (`ng=1`, `m=1`) of W = 30 and 32 µm, so very long
  thin devices; a sensible layout splits them into fingers or parallel devices, which
  means changing the schematic first. There are no parallel groups as drawn;
- six nets are open, with 3 to 5 pieces each (tail, vdd, x, ibias, vss, out), several
  needing contacts to GatPoly, and probably Metal2 where nets cross;
- the ties must be within 20 µm of all n+ areas (LU.b) and the pmos wells joined or
  spaced apart;
- matching (interdigitated or common-centroid pair) has no generator: every finger is
  placed and routed by hand.

A guided O10 down to LVS clean should not be promised before someone has done that
routing.
