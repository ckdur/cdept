#!/usr/bin/env python3
"""A stand-in for UberWave with IPC protocol 1, for tests and GUI scripts
(CDEPT_UBERWAVE=<this file>). Python 3 standard library only.

  fake-uberwave-ipc.py --version
      prints "UberWaveViewer fake ipc 1" and exits.
  fake-uberwave-ipc.py [--ipc SOCKET] [--analysis A] [--tag T ...] FILE [FILE...]
      records its command line, serves the socket protocol on SOCKET (when given)
      until it gets `quit` or nothing happens for FAKE_UBERWAVE_IDLE seconds
      (default 60), and removes the socket.

The log, one JSON object per line, is the file named by $FAKE_UBERWAVE_LOG (default:
fake-uberwave-ipc.log in the working folder):
  {"start": {"argv": [...], "cwd": "...", "tags": [...], "files": [...], "analysis": ..}}
  {"request": {"cmd": "add_trace", "name": "...", "n": 1234, "y_first": .., "y_last": ..}}
  (x and y arrays are not logged, only their length and ends)
Like fake-uberwave.sh it also writes the arguments (one per line) to
uberwave-args.txt and the working folder to uberwave-cwd.txt in the working folder.

The files given are the session: `list_files` answers them with their analyses and
`list_signals` the variables of their raw header (a trivial parse of the text header).
Pushed traces are kept and listed by `list_signals` (file "external").
FAKE_UBERWAVE_REFUSE=<cmd> makes that command answer ok:false (to test errors).
"""
import json
import os
import socket
import sys
import time

VERSION = "UberWaveViewer fake ipc 1"


def parse_raw_header(path):
    """[(plot name, [(signal, unit)])] from the text header of an ngspice raw file."""
    plots = []
    try:
        with open(path, "rb") as f:
            data = f.read(4 * 1024 * 1024)
    except OSError:
        return plots
    text = data.decode("latin-1")
    cur = None
    invars = False
    for line in text.splitlines():
        if line.startswith("Plotname:"):
            cur = [line.split(":", 1)[1].strip(), []]
            plots.append(cur)
            invars = False
        elif line.startswith("Variables:"):
            invars = True
        elif line.startswith(("Values:", "Binary:")):
            invars = False
        elif invars and cur is not None and line.startswith(("\t", " ")):
            parts = line.split()
            if len(parts) >= 3:
                cur[1].append((parts[1], parts[2]))
    return plots


def kind_of(name):
    n = name.lower()
    for k, word in (("tran", "transient"), ("ac", "ac "), ("dc", "dc "), ("noise", "noise"), ("op", "operating")):
        if word in n:
            return k
    return "other"


class State:
    def __init__(self):
        self.files = []  # dicts: tag, path, analyses
        self.traces = {}
        self.cursors = {"a": None, "b": None}
        self.analysis = None
        self.shown = []
        self.panes = 0
        self.events = []

    def add_file(self, path, tag=None, analysis=None):
        plots = parse_raw_header(path)
        stem = os.path.splitext(os.path.basename(path))[0]
        self.files.append({"tag": tag or stem, "path": path, "analyses": [kind_of(p[0]) for p in plots],
                           "plots": plots, "analysis": analysis})


def log_to(path, obj):
    with open(path, "a") as f:
        f.write(json.dumps(obj) + "\n")


def trimmed(req):
    out = {}
    for k, v in req.items():
        if k in ("x", "y") and isinstance(v, list):
            out["n" if k == "x" else "n_y"] = len(v)
            if v:
                out[k + "_first"] = v[0]
                out[k + "_last"] = v[-1]
        else:
            out[k] = v
    return out


def handle(st, req):
    cmd = req.get("cmd")
    refuse = os.environ.get("FAKE_UBERWAVE_REFUSE")
    if refuse and cmd == refuse:
        return {"ok": False, "error": "refused by FAKE_UBERWAVE_REFUSE"}
    if cmd == "hello":
        return {"ok": True, "app": "UberWaveViewer", "version": "fake", "protocol": 1,
                "features": ["open", "import", "add_trace", "list_signals", "subscribe"]}
    if cmd in ("open", "import"):
        path = req.get("path")
        if not path or not os.path.exists(path):
            return {"ok": False, "error": "no such file: %s" % path}
        if cmd == "open":
            st.files = []
            st.traces = {}
        st.add_file(path, req.get("tag"), req.get("analysis"))
        return {"ok": True, "tag": st.files[-1]["tag"]}
    if cmd == "list_files":
        return {"ok": True, "files": [{"tag": f["tag"], "path": f["path"], "analyses": f["analyses"]} for f in st.files]}
    if cmd == "list_signals":
        want, flt, out = req.get("file"), (req.get("filter") or "").lower(), []
        for f in st.files:
            if want and f["tag"] != want:
                continue
            for _plot, sigs in f["plots"]:
                for name, unit in sigs:
                    if flt in name.lower():
                        out.append({"name": name, "file": f["tag"], "unit": unit, "complex": False})
        for name, t in st.traces.items():
            if flt in name.lower():
                out.append({"name": name, "file": "external", "unit": t.get("unit", ""), "complex": False})
        return {"ok": True, "signals": out}
    if cmd == "show":
        st.shown += req.get("signals", [])
        pane = req.get("pane")
        if pane == "new" or pane is None:
            pane = st.panes
            st.panes += 1
        elif not isinstance(pane, int) or pane >= st.panes:
            return {"ok": False, "error": "no pane %s" % pane}
        return {"ok": True, "pane": pane, "shown": req.get("signals", [])}
    if cmd == "hide":
        st.shown = [s for s in st.shown if s not in req.get("signals", [])]
        return {"ok": True}
    if cmd == "set_analysis":
        st.analysis = req.get("name")
        return {"ok": True}
    if cmd == "set_cursors":
        for k in ("a", "b"):
            if k in req:
                st.cursors[k] = req[k]
        return {"ok": True}
    if cmd == "get_cursors":
        return {"ok": True, "a": st.cursors["a"], "b": st.cursors["b"], "values": {}}
    if cmd == "set_range":
        return {"ok": True}
    if cmd == "add_trace":
        name, x, y = req.get("name"), req.get("x"), req.get("y")
        if not isinstance(name, str) or not isinstance(x, list) or not isinstance(y, list):
            return {"ok": False, "error": "add_trace needs name, x and y"}
        if len(x) != len(y):
            return {"ok": False, "error": "x and y differ in length"}
        if name in st.traces:
            return {"ok": False, "error": "a trace named '%s' exists" % name}
        st.traces[name] = {"n": len(x), "unit": req.get("unit", "")}
        return {"ok": True, "points": len(x)}
    if cmd == "remove_trace":
        if st.traces.pop(req.get("name"), None) is None:
            return {"ok": False, "error": "no trace named '%s'" % req.get("name")}
        return {"ok": True}
    if cmd == "subscribe":
        st.events = req.get("events", [])
        return {"ok": True}
    if cmd == "quit":
        return {"ok": True, "_quit": True}
    return {"ok": False, "error": "unknown command '%s'" % cmd}


def serve(sock_path, st, log):
    if os.path.exists(sock_path):
        os.unlink(sock_path)
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(sock_path)
    srv.listen(8)
    idle = float(os.environ.get("FAKE_UBERWAVE_IDLE", "60"))
    srv.settimeout(0.2)
    last = time.time()
    quit_ = False
    conns = []  # (socket, buffer)
    try:
        while not quit_ and time.time() - last < idle:
            try:
                c, _ = srv.accept()
                c.setblocking(False)
                conns.append([c, b""])
                last = time.time()
            except socket.timeout:
                pass
            for entry in list(conns):
                c = entry[0]
                try:
                    data = c.recv(1 << 20)
                except BlockingIOError:
                    continue
                except OSError:
                    data = b""
                if not data:
                    conns.remove(entry)
                    c.close()
                    continue
                last = time.time()
                entry[1] += data
                while b"\n" in entry[1]:
                    line, entry[1] = entry[1].split(b"\n", 1)
                    if not line.strip():
                        continue
                    try:
                        req = json.loads(line.decode("utf-8"))
                    except ValueError:
                        reply = {"ok": False, "error": "not JSON"}
                        req = {}
                    else:
                        log_to(log, {"request": trimmed(req)})
                        reply = handle(st, req)
                    if reply.pop("_quit", False):
                        quit_ = True
                    reply["id"] = req.get("id")
                    c.setblocking(True)
                    try:
                        c.sendall((json.dumps(reply) + "\n").encode("utf-8"))
                    except OSError:
                        pass
                    c.setblocking(False)
    finally:
        for entry in conns:
            entry[0].close()
        srv.close()
        try:
            os.unlink(sock_path)
        except OSError:
            pass


def main(argv):
    if "--version" in argv:
        print(VERSION)
        return 0
    sock_path, analysis, tags, files = None, None, [], []
    it = iter(argv)
    for a in it:
        if a == "--ipc":
            sock_path = next(it)
        elif a == "--analysis":
            analysis = next(it)
        elif a == "--tag":
            tags.append(next(it))
        else:
            files.append(a)
    log = os.environ.get("FAKE_UBERWAVE_LOG", "fake-uberwave-ipc.log")
    with open("uberwave-args.txt", "w") as f:
        f.write("".join(a + "\n" for a in argv))
    with open("uberwave-cwd.txt", "w") as f:
        f.write(os.getcwd() + "\n")
    log_to(log, {"start": {"argv": argv, "cwd": os.getcwd(), "tags": tags, "files": files, "analysis": analysis,
                           "ipc": sock_path}})
    st = State()
    for i, path in enumerate(files):
        st.add_file(path, tags[i] if i < len(tags) else None, analysis)
    if sock_path:
        serve(sock_path, st, log)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
