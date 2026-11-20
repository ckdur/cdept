# cdept-klayout: parameter coercion of KLayout's "Basic" library PCells.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Python ports of `coerce_parameters` of KLayout's Basic library (src/lib/lib/
libBasic*.cc, GPL-3.0-or-later like this helper).

Basic's PCells are C++, and KLayout's Python binding of a C++ PCell's
`coerce_parameters` runs the base class's (which does nothing). KLayout's own dialogs
and its partial mode call the real one; CDePT's forms and handles need the same
result, so that a moved handle changes the radius and a new radius moves the handle.
Each function takes and returns the parameters by name.
"""

import math

import pya

EPS = 1e-6


def _f(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def _pt(value):
    return value if isinstance(value, pya.DPoint) else None


def circle(p):
    ru, r = _f(p.get("radius")), _f(p.get("actual_radius"))
    h = _pt(p.get("handle"))
    rs = h.distance(pya.DPoint()) if h is not None else ru
    if abs(ru - r) > EPS:
        ru = r
        p["handle"] = pya.DPoint(-r, 0)
    else:
        ru = r = rs
        p["actual_radius"] = r
    p["radius"] = ru
    return p


def ellipse(p):
    for axis, at in (("x", lambda v: pya.DPoint(-v, 0)), ("y", lambda v: pya.DPoint(0, v))):
        ru, r = _f(p.get("radius_" + axis)), _f(p.get("actual_radius_" + axis))
        h = _pt(p.get("handle_" + axis))
        rs = abs(h.x if axis == "x" else h.y) if h is not None else ru
        if abs(ru - r) > EPS:
            ru = r
        else:
            ru = r = rs
            p["actual_radius_" + axis] = r
        p["handle_" + axis] = at(r)
        p["radius_" + axis] = ru
    return p


def donut(p):
    ru1, r1 = _f(p.get("radius1")), _f(p.get("actual_radius1"))
    ru2, r2 = _f(p.get("radius2")), _f(p.get("actual_radius2"))
    h1, h2 = _pt(p.get("handle1")), _pt(p.get("handle2"))
    rs1 = h1.distance(pya.DPoint()) if h1 is not None else ru1
    rs2 = h2.distance(pya.DPoint()) if h2 is not None else ru2
    if abs(ru1 - r1) > EPS or abs(ru2 - r2) > EPS:
        ru1, ru2 = r1, r2
        p["handle1"] = pya.DPoint(-r1, 0)
        p["handle2"] = pya.DPoint(-r2, 0)
    else:
        ru1 = r1 = rs1
        ru2 = r2 = rs2
        p["actual_radius1"] = r1
        p["actual_radius2"] = r2
    p["radius1"] = ru1
    p["radius2"] = ru2
    return p


def _angles(p, radii, rs):
    """The part PIE and ARC share. `radii`: [(used name, actual name, handle radius)]."""
    a1u, a1 = _f(p.get("start_angle")), _f(p.get("actual_start_angle"))
    a2u, a2 = _f(p.get("end_angle")), _f(p.get("actual_end_angle"))
    h1u = _pt(p.get("handle1")) or pya.DPoint()
    h1 = _pt(p.get("actual_handle1")) or pya.DPoint()
    h2u = _pt(p.get("handle2")) or pya.DPoint()
    h2 = _pt(p.get("actual_handle2")) or pya.DPoint()
    used = {u: _f(p.get(u)) for u, _, _ in radii}
    actual = {a: _f(p.get(a)) for _, a, _ in radii}
    if (any(abs(used[u] - actual[a]) > EPS for u, a, _ in radii)
            or abs(a1u - a1) > EPS or abs(a2u - a2) > EPS):
        for u, a, _ in radii:
            used[u] = actual[a]
        a1u, a2u = a1, a2
        r1 = actual[radii[0][1]]
        r2 = actual[radii[-1][1]]
        h1u = pya.DPoint(r1 * math.cos(math.radians(a1)), r1 * math.sin(math.radians(a1)))
        h2u = pya.DPoint(r2 * math.cos(math.radians(a2)), r2 * math.sin(math.radians(a2)))
        p["actual_handle1"] = h1u
        p["actual_handle2"] = h2u
    elif h1u.distance(h1) > EPS or h2u.distance(h2) > EPS:
        a1u = math.degrees(math.atan2(h1.y, h1.x))
        a2u = math.degrees(math.atan2(h2.y, h2.x))
        for (u, a, _), r in zip(radii, rs):
            used[u] = r
            p[a] = r
        h1u, h2u = h1, h2
        p["actual_start_angle"] = a1u
        p["actual_end_angle"] = a2u
    for u in used:
        p[u] = used[u]
    p["start_angle"] = a1u
    p["end_angle"] = a2u
    p["handle1"] = h1u
    p["handle2"] = h2u
    return p


def pie(p):
    ru = _f(p.get("radius"))
    h1, h2 = _pt(p.get("actual_handle1")), _pt(p.get("actual_handle2"))
    o = pya.DPoint()
    if h1 is not None:
        rs = h1.distance(o)
        if h2 is not None:
            rs = max(rs, h2.distance(o))
    elif h2 is not None:
        rs = h2.distance(o)
    else:
        rs = ru
    return _angles(p, [("radius", "actual_radius", None)], [rs])


def arc(p):
    o = pya.DPoint()
    h1, h2 = _pt(p.get("actual_handle1")), _pt(p.get("actual_handle2"))
    rs1 = h1.distance(o) if h1 is not None else _f(p.get("radius1"))
    rs2 = h2.distance(o) if h2 is not None else _f(p.get("radius2"))
    return _angles(p, [("radius1", "actual_radius1", None), ("radius2", "actual_radius2", None)],
                   [rs1, rs2])


COERCE = {"CIRCLE": circle, "ELLIPSE": ellipse, "DONUT": donut, "PIE": pie, "ARC": arc}


def coerce(name, values):
    """The parameters of Basic PCell `name` made consistent; None when it has no rule
    here (the others need none for what CDePT edits)."""
    rule = COERCE.get(name)
    return rule(dict(values)) if rule else None
