# Glue between CDePT and VACASK's own ngspice-to-VACASK converter (ng2vc, shipped with
# VACASK in <prefix>/lib/vacask/python). CDePT runs this as a separate process:
#   python3 convert.py spec.json  ->  writes spec["result"] (JSON)
# spec: python_path, prefix, sourcepath, profile, files[{src, dst, corner}], result
# The settings of the "ihp-sg13g2" profile are the ones VACASK's own sg13g2tovc.py uses.
import json
import os
import sys

spec = json.load(open(sys.argv[1]))
sys.path.insert(0, spec["python_path"])
from ng2vclib.converter import Converter  # noqa: E402
from ng2vclib.dfl import default_config  # noqa: E402

PROFILES = {
    "ihp-sg13g2": {
        "patch": {
            # Bugs of the ngspice libraries
            "sg13g2_esd.lib": [(
                ".MODEL diodevss_mod D (tnom = 27 level = 1 is=9.017E-019 rs=200   n=1.03 isr=3.776E-015   ikf=0.0001754 cj0=9.42E-016  m=0.3012  vj=0.6684 bv=11.28 ibv=1E-009 8 nbv=1.324   eg=1.17 xti=3  )",
                ".MODEL diodevss_mod D (tnom = 27 level = 1 is=9.017E-019 rs=200   n=1.03 isr=3.776E-015   ikf=0.0001754 cj0=9.42E-016  m=0.3012  vj=0.6684 bv=11.28 ibv=1E-009 nbv=1.324   eg=1.17 xti=3  )",
            )],
            "sg13g2_svaricaphv_mod.lib": [("+ stuac 40", "+ stuac=40")],
            # ng2vc splits parameters at '=' and trips over '!='
            # (the mismatch parameters are agauss(...) with sigma 0 unless mm_ok=1; they are
            # replaced by their nominal value below)
            "resistors_mod.lib": [("+sw_mman=", "+sw_mman='(abs(mm_ok-1)<0.5?1:0)'")],
            # A global parameter; VACASK sets swsoa itself
            "sg13g2_moshv_mod.lib": [(".param SWSOA = 0", "")],
            "sg13g2_moshv_mod_mismatch.lib": [(".param SWSOA = 0", "")],
            "sg13g2_moslv_mod.lib": [(".param SWSOA = 0", "")],
            "sg13g2_moslv_mod_mismatch.lib": [(".param SWSOA = 0", "")],
            "sg13g2_moscap_mod.lib": [(".param SWSOA = 0", "")],
            "sg13g2_moscap_mod_mismatch.lib": [(".param SWSOA = 0", "")],
        },
        "family_map": {
            ("bjt", 4, None): ("vbic_1p3_5t.osdi", "vbic13_5t", {}),
            ("bjt", 9, None): ("vbic_1p3_5t.osdi", "vbic13_5t", {}),
            ("bjt", 1, None): ("spice/full/bjt.osdi", "sp_bjt", {}),
            ("bjt", None, None): ("spice/full/bjt.osdi", "sp_bjt", {}),
        },
        "remove_model_params": {"vbic13_5t": set(["vbe_max", "vbc_max", "vce_max"])},
        "subckt_multiplier": {"cap_cmim": ["m", True], "cap_rfcmim": ["m", True]},
    }
}
profile = PROFILES.get(spec.get("profile", ""), {})

import re  # noqa: E402

AGAUSS = re.compile(r"^(\.param\s+\w+\s*=\s*|\+\s*\w+\s*=\s*)'agauss\(\s*([^,()]+?)\s*,")


def nominal_patches(src):
    """`.param x='agauss(mean, sigma, n)'` is `mean` when no mismatch is wanted (n=0):
    one patch (whole line) for each such line. VACASK has no agauss."""
    out = []
    try:
        lines = open(src, errors="replace").read().splitlines()
    except OSError:
        return out
    for line in lines:
        m = AGAUSS.match(line)
        if m:
            out.append((line, m.group(1) + m.group(2)))
    return out


def merged_patch(base, name, src):
    patch = {k: list(v) for k, v in base.items()}
    extra = nominal_patches(src)
    if extra:
        patch.setdefault(name, []).extend(extra)
    return patch


results = []
for f in spec["files"]:
    r = {"src": f["src"], "dst": f["dst"], "ok": False}
    try:
        cfg = default_config()
        depth = 0 if f["corner"] else 1
        out_depth = 0 if f["corner"] else None
        cfg.update({
            "default_model_prefix": spec["prefix"],
            "sourcepath": spec["sourcepath"],
            "read_depth": depth,
            "process_depth": depth,
            "output_depth": out_depth,
            "patch": merged_patch(profile.get("patch", {}), os.path.basename(f["src"]), f["src"]),
            "original_case_subckt": True,
            "original_case_model": True,
        })
        cfg["family_map"].update(profile.get("family_map", {}))
        cfg["remove_model_params"].update(profile.get("remove_model_params", {}))
        cfg["subckt_multiplier"].update(profile.get("subckt_multiplier", {}))
        cfg["signature"] = "// Converted by ng2vc for CDePT\n"
        cvt = Converter(cfg, indent=4, debug=0)
        os.makedirs(os.path.dirname(f["dst"]), exist_ok=True)
        cvt.convert(f["src"], f["dst"])
        osdi = set()
        for mname, in_sub in cvt.data["model_usage"]:
            builtin, mtype, family, level, version, _ = cvt.data["models"][in_sub][mname]
            k = family, level, version
            if k in cvt.cfg["family_map"]:
                osdi.add(cvt.cfg["family_map"][k][0])
        defaults = {}
        for mt in cvt.data["default_models_needed"]:
            file, module = cvt.cfg["default_models"][mt]
            osdi.add(file)
            defaults[mt] = module
        r.update({"ok": True, "osdi": sorted(osdi), "default_models": defaults})
    except Exception as e:  # the converter raises its own errors with the line
        r["error"] = str(e).strip() or repr(e)
    results.append(r)
json.dump(results, open(spec["result"], "w"))
