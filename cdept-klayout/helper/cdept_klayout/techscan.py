# cdept-klayout: what only KLayout itself can tell about a technology.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run on its own by KLayout, not by the helper:

    klayout -zz -nc -r techscan.py -rd out_file=<file.json>

with the PDK on KLAYOUT_PATH. Not `-b`: that leaves out the autorun macros, and
those are what registers a PDK's PCell libraries. Writes one JSON object:

    {"klayout": "0.30.x",
     "technologies": [{"name", "description", "group", "dbu", "base_path",
                       "layer_properties_file"}],
     "libraries": [{"name", "description", "technologies": [...],
                    "cells": [static cell names],
                    "pcells": [{"name", "description",
                                "parameters": [{"name", "type", "default",
                                                "description", "group", "unit",
                                                "choices": [{"label", "value"}],
                                                "hidden", "readonly"}]}]}],
     "errors": ["..."]}

Values are text: the caller keeps them as written. This file imports nothing of the
helper package, so nothing else of it is started.
"""

import json
import sys

import pya

_TYPES = {
    "TypeInt": "int", "TypeDouble": "double", "TypeString": "string",
    "TypeBoolean": "boolean", "TypeList": "list", "TypeLayer": "layer",
    "TypeShape": "shape", "TypeCallback": "callback", "TypeNone": "none",
}
TYPE_NAMES = {getattr(pya.PCellParameterDeclaration, k): v for k, v in _TYPES.items()
              if hasattr(pya.PCellParameterDeclaration, k)}


def _text(value):
    """A parameter value as text: numbers and booleans as JSON writes them."""
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, (list, tuple)):
        return json.dumps([_text(v) for v in value])
    return str(value)


def parameters(decl):
    out = []
    for p in decl.get_parameters():
        description = p.description or ""
        group = ""
        if "\t" in description:  # KLayout's "Group<TAB>Label"
            group, description = description.split("\t", 1)
        choices = []
        try:
            for label, value in zip(p.choice_descriptions(), p.choice_values()):
                choices.append({"label": str(label), "value": _text(value)})
        except Exception:
            pass
        out.append({
            "name": p.name, "type": TYPE_NAMES.get(p.type, "none"),
            "default": _text(p.default), "description": description, "group": group,
            "unit": p.unit or "", "choices": choices,
            "hidden": bool(p.hidden), "readonly": bool(p.readonly),
        })
    return out


def libraries(errors):
    out = []
    # By id: `library_by_name` misses libraries bound to a technology.
    for i in pya.Library.library_ids():
        lib = pya.Library.library_by_id(i)
        if lib is None:
            continue
        ly = lib.layout()
        pcells = []
        for name in sorted(ly.pcell_names()):
            entry = {"name": name, "description": "", "parameters": []}
            try:
                decl = ly.pcell_declaration(name)
                try:
                    entry["description"] = decl.display_text(
                        [p.default for p in decl.get_parameters()]) or ""
                except Exception:
                    pass
                entry["parameters"] = parameters(decl)
            except Exception as error:  # a PCell that fails must not lose the rest
                errors.append("%s/%s: %s" % (lib.name(), name, error))
            pcells.append(entry)
        cells = sorted(c.name for c in ly.top_cells()
                       if not c.is_pcell_variant() and not c.is_proxy())
        out.append({
            "name": lib.name(), "description": lib.description or "",
            "technologies": list(lib.technologies()),
            "cells": cells, "pcells": pcells,
        })
    return out


def technologies():
    out = []
    for name in pya.Technology.technology_names():
        tech = pya.Technology.technology_by_name(name)
        if tech is None or not name:
            continue
        out.append({
            "name": name, "description": tech.description or "",
            "group": tech.group or "", "dbu": tech.dbu,
            "base_path": tech.base_path() or "",
            "layer_properties_file": tech.layer_properties_file or "",
        })
    return out


def scan():
    errors = []
    result = {"klayout": pya.Application.instance().version(), "errors": errors}
    try:
        result["technologies"] = technologies()
    except Exception as error:
        result["technologies"] = []
        errors.append("technologies: %s" % error)
    try:
        result["libraries"] = libraries(errors)
    except Exception as error:
        result["libraries"] = []
        errors.append("libraries: %s" % error)
    return result


def main():
    target = globals().get("out_file")
    text = json.dumps(scan(), indent=1)
    if isinstance(target, str) and target:
        with open(target, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text + "\n")


if __name__ == "__main__" or "out_file" in globals():
    main()
