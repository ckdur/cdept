# Glue between CDePT and VACASK's own ngspice-to-VACASK converter (ng2vc, shipped with
# VACASK in <prefix>/lib/vacask/python). CDePT runs this as a separate process:
#   python3 convert.py spec.json  ->  writes spec["result"] (JSON)
# spec: python_path, prefix, sourcepath, profile, files[{src, dst, corner}], result
# result: {"files": [{src, dst, ok, osdi, default_models, defs, mm, globals | error}],
#          "globals": [labels of the technology-wide draws]}
# The settings of the "ihp-sg13g2" profile are the ones VACASK's own sg13g2tovc.py uses.
import json
import os
import re
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

# Statistical functions. VACASK has no random numbers, so the gauss()/agauss() calls of
# the ngspice models become expressions that read draws CDePT makes for each run: the
# deck defines the vectors `mcz` (one draw per technology-wide parameter) and `mmz` (one
# draw per instance and slot, the instance's `mmk` and the call's slot say which).
GAUSS = re.compile(r"(?<![\w.])(a?gauss)\s*\(", re.I)
UNSUPPORTED = re.compile(r"(?<![\w.])(a?unif|limit)\s*\(", re.I)
NEQ = re.compile(r"([A-Za-z_]\w*)\s*!=\s*(-?\d+(?:\.\d+)?)")
LABEL = re.compile(r"(\w+)\s*=\s*['\"{(]*\s*$")
CALL = re.compile(r"cdept(ag|g)_(\w+)\(")
COMMENT = ("*", ";", "$", "//")
LEVEL = re.compile(r"\b(level|version)=(\d+)\.0+(?![\d.])", re.I)
VERSION = re.compile(r"\s*\bversion=\d+\.\d+(?![\d.])", re.I)


def preprocess(line):
    """Changes to a netlist line (ng2vc has joined the continuation lines and removed
    spaces around `=`) before ng2vc reads it:
    - gauss()/agauss() calls are renamed `cdeptag_<param>` / `cdeptg_<param>` (the
      parameter the call is assigned to names its slot, see `finish`);
    - `x!=1` becomes `abs(x-1)>0.5` (ng2vc cannot read `!=`);
    - `level=54.0` is `level=54` and `version=4.5` goes (ng2vc wants integers; the BSIM4
      version is VACASK's own)."""
    if line.lstrip().startswith(COMMENT):
        return line
    if UNSUPPORTED.search(line):
        raise ValueError("unif, aunif and limit are not supported with VACASK: " + line.strip())
    line = LEVEL.sub(lambda m: "%s=%s" % (m.group(1), m.group(2)), line)
    line = VERSION.sub("", line)
    if not GAUSS.search(line):
        return line
    new = ""
    last = 0
    for m in GAUSS.finditer(line):
        lab = LABEL.search(line[: m.start()])
        label = lab.group(1) if lab else "x"
        prefix = "cdeptag_" if m.group(1).lower() == "agauss" else "cdeptg_"
        new += line[last : m.start()] + prefix + label + "("
        last = m.end()
    new += line[last:]
    return NEQ.sub(lambda m: "abs(%s-%s)>0.5" % (m.group(1), m.group(2)), new)


_orig_preprocess = Converter.preprocess_line
Converter.preprocess_line = lambda self, line: preprocess(_orig_preprocess(self, line))


def split_args(text, start):
    """The arguments of the call whose `(` is just before text[start]; (args, end)."""
    depth = 1
    args = []
    cur = ""
    i = start
    while i < len(text):
        c = text[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                args.append(cur.strip())
                return args, i + 1
        elif c == "," and depth == 1:
            args.append(cur.strip())
            cur = ""
            i += 1
            continue
        cur += c
        i += 1
    raise ValueError("unbalanced parentheses in a gauss() call")


GLOBAL_LABELS = []  # the technology-wide draws: mcz[index]


def finish(path):
    """Replace the renamed calls of a converted file by expressions that read the draw
    vectors and give every subcircuit with per-instance draws a `mmk` parameter (-1:
    no draws, the nominal value). Returns the names the file defines, the subcircuits
    with their slots, and the technology-wide labels it uses."""
    lines = open(path).read().split("\n")
    out = []
    sub = None
    sub_labels = []
    header_at = 0
    mm = {}
    used_globals = []
    defs = []
    for line in lines:
        is_comment = line.lstrip().startswith("//")
        m = None if is_comment else re.match(r"\s*subckt\s+([^\s(]+)", line)
        if m:
            sub, sub_labels = m.group(1), []
            defs.append(sub)
            header_at = len(out)
        while not is_comment:
            c = CALL.search(line)
            if not c:
                break
            kind, label = c.group(1), c.group(2)
            args, end = split_args(line, c.end())
            if len(args) != 3:
                raise ValueError("gauss and agauss take three arguments: " + line.strip())
            a, b, n = args
            if sub is None:
                if label not in GLOBAL_LABELS:
                    GLOBAL_LABELS.append(label)
                if label not in used_globals:
                    used_globals.append(label)
                z = "mcz[%d]" % GLOBAL_LABELS.index(label)
                off = "(%s)==0" % n
            else:
                if label not in sub_labels:
                    sub_labels.append(label)
                z = "mmz[mmk*mmnz+%d]" % sub_labels.index(label)
                off = "(%s)==0||mmk<0" % n
            if kind == "ag":
                expr = "((%s)?(%s):((%s)+(%s)/(%s)*%s))" % (off, a, a, b, n, z)
            else:
                expr = "((%s)?(%s):((%s)*(1+(%s)/(%s)*%s)))" % (off, a, a, b, n, z)
            line = line[: c.start()] + expr + line[end:]
        out.append(line)
        if not is_comment and re.match(r"\s*ends\b", line) and sub is not None:
            if sub_labels:
                mm[sub] = list(sub_labels)
                out.insert(header_at + 1, "  parameters mmk=-1")
            sub = None
    open(path, "w").write("\n".join(out))
    return {"defs": defs, "mm": mm, "globals": used_globals}


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
            "patch": profile.get("patch", {}),
            "original_case_subckt": True,
            "original_case_model": True,
        })
        # ng2vc has no BSIM4 (levels 14 and 54); VACASK ships it as OSDI.
        cfg["family_map"].update({
            ("mos", 14, None): ("spice/bsim4v8.osdi", "sp_bsim4v8", {}),
            ("mos", 54, None): ("spice/bsim4v8.osdi", "sp_bsim4v8", {}),
        })
        cfg["family_map"].update(profile.get("family_map", {}))
        cfg["remove_model_params"].update(profile.get("remove_model_params", {}))
        cfg["subckt_multiplier"].update(profile.get("subckt_multiplier", {}))
        cfg["signature"] = "// Converted by ng2vc for CDePT\n"
        cvt = Converter(cfg, indent=4, debug=0)
        os.makedirs(os.path.dirname(f["dst"]), exist_ok=True)
        cvt.convert(f["src"], f["dst"])
        r.update(finish(f["dst"]))
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


def share_parameters():
    """VACASK refuses a parameter defined twice, ngspice does not: a library parameter
    that several library files define with the same literal value (`num_sigmas` of every
    `_stat` file) moves out of the files into one list the deck defines first."""
    pattern = re.compile(r"^parameters\s+(\w+)=([-+0-9.eE]+)\s*$")
    found = {}
    scanned = []
    for r in results:
        if not r["ok"] or r["corner"]:
            continue
        lines = open(r["dst"]).read().split("\n")
        depth = 0
        for n, line in enumerate(lines):
            if re.match(r"\s*subckt\b", line):
                depth += 1
            elif re.match(r"\s*ends\b", line):
                depth -= 1
            m = pattern.match(line) if depth == 0 else None
            if m:
                found.setdefault(m.group(1), []).append((r["dst"], n, m.group(2)))
        scanned.append((r["dst"], lines))
    shared = {k: v[0][2] for k, v in found.items() if len(v) > 1 and len({x[2] for x in v}) == 1}
    drop = {}
    for k in shared:
        for dst, n, _ in found[k]:
            drop.setdefault(dst, set()).add(n)
    for dst, lines in scanned:
        if dst in drop:
            keep = [l for n, l in enumerate(lines) if n not in drop[dst]]
            open(dst, "w").write("\n".join(keep))
    return shared


for r in results:
    r["corner"] = next(f["corner"] for f in spec["files"] if f["src"] == r["src"])
shared = share_parameters()
json.dump({"files": results, "globals": GLOBAL_LABELS, "shared": shared}, open(spec["result"], "w"))
