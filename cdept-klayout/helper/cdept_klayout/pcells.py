# cdept-klayout: PCells written in CDePT libraries (docs/KLAYOUT.md §3.6).
# SPDX-License-Identifier: GPL-3.0-or-later
"""The project's PCell views, registered as KLayout libraries.

CDePT sends them with every reference table (`pcells`: CDePT library -> cell -> file
`<cell>.pcell.py`). Each CDePT library with PCell views becomes one KLayout library of
the same name, holding one PCell per view, named after the cell. A file defines its
PCell as a subclass of `pya.PCellDeclarationHelper`: the one named by a module-level
`PCELL = ...`, else the only such subclass the file defines.

Libraries are registered before a layout is read, so that saved instances come back as
live PCells, and registered again when a file changes (`refresh`): KLayout then
rebuilds every instance. Errors in a file (loading it, or producing a cell) become
messages with the file and line.
"""

import os
import sys
import traceback

import pya

# CDePT library -> {"files": {cell: (path, mtime)}, "library": pya.Library, "decls": [..]}
_registered = {}
# The last table given, so that `refresh` can look at the files again.
_wanted = {}
# Messages not handed out yet: (level, text).
_messages = []
# Messages already shown for an error that keeps coming (a PCell produced again and
# again with the same mistake): shown once.
_shown = set()


def _say(level, text):
    if text not in _shown:
        _shown.add(text)
        _messages.append((level, text))


def take_messages():
    """The messages since the last call: [(level, text)]."""
    out = list(_messages)
    del _messages[:]
    return out


def _where(error, path):
    """`file:line: message` for an exception raised in (or about) the file `path`."""
    line = None
    if isinstance(error, SyntaxError) and error.filename == path:
        line = error.lineno
    else:
        for frame in traceback.extract_tb(error.__traceback__):
            if os.path.normpath(frame.filename) == os.path.normpath(path):
                line = frame.lineno
    text = "%s: %s" % (type(error).__name__, error)
    return "%s:%s: %s" % (path, line, text) if line else "%s: %s" % (path, text)


def _state(cells):
    out = {}
    for cell, path in sorted((cells or {}).items()):
        path = os.path.normpath(path)
        try:
            mtime = os.stat(path).st_mtime_ns
        except OSError:
            mtime = None
        out[cell] = (path, mtime)
    return out


def _guarded(cls, path):
    """`cls` with its methods reporting errors with the file and line (KLayout itself
    only leaves an empty cell)."""
    def wrap(name):
        method = getattr(cls, name, None)
        if method is None:
            return

        def guarded(self, *args, **kwargs):
            try:
                return method(self, *args, **kwargs)
            except Exception as error:
                _say("error", _where(error, path))
                raise
        guarded.__name__ = name
        setattr(cls, name, guarded)
    for name in ("produce_impl", "coerce_parameters_impl", "display_text_impl",
                 "transformation_from_shape_impl", "parameters_from_shape_impl"):
        if name in cls.__dict__:
            wrap(name)
    return cls


def load_declaration(path, cell):
    """The PCell declaration class defined by file `path` (see the module doc)."""
    with open(path, encoding="utf-8") as f:
        code = f.read()
    module = "cdept_pcell_%s" % "".join(c if c.isalnum() else "_" for c in path)
    namespace = {"__name__": module, "__file__": path, "pya": pya}
    exec(compile(code, path, "exec"), namespace)
    cls = namespace.get("PCELL")
    if cls is None:
        found = [v for v in namespace.values()
                 if isinstance(v, type) and issubclass(v, pya.PCellDeclarationHelper)
                 and v is not pya.PCellDeclarationHelper and v.__module__ == module]
        if len(found) != 1:
            raise ValueError("%s defines %s subclasses of pya.PCellDeclarationHelper; "
                             "name the PCell with PCELL = <class>"
                             % (cell, len(found) if found else "no"))
        cls = found[0]
    if not (isinstance(cls, type) and issubclass(cls, pya.PCellDeclarationHelper)):
        raise ValueError("PCELL is not a subclass of pya.PCellDeclarationHelper")
    return _guarded(cls, path)


def _register(name, state):
    library = pya.Library()
    library.description = "CDePT library %s" % name
    decls = []
    for cell, (path, mtime) in state.items():
        if mtime is None:
            _say("error", "%s: the PCell file of %s/%s is gone" % (path, name, cell))
            continue
        try:
            decl = load_declaration(path, cell)()
        except Exception as error:
            _say("error", _where(error, path))
            continue
        library.layout().register_pcell(cell, decl)
        decls.append(decl)
    library.register(name)
    _registered[name] = {"files": state, "library": library, "decls": decls}


def sync(pcells):
    """Register the libraries of `pcells` ({library: {cell: file}}) that are new or
    whose files changed; drop those no longer there. True when anything changed."""
    global _wanted
    _wanted = dict(pcells or {})
    changed = False
    for name, cells in sorted(_wanted.items()):
        state = _state(cells)
        old = _registered.get(name)
        if old is not None and old["files"] == state:
            continue
        if old is not None:
            _shown.clear()  # a new version: its errors are new
        _register(name, state)
        changed = True
    for name in [n for n in _registered if n not in _wanted]:
        entry = _registered.pop(name)
        try:
            entry["library"].unregister()
        except Exception as error:  # pragma: no cover - KLayout without unregister
            print("cdept-klayout: unregister %s: %s" % (name, error), file=sys.stderr)
        changed = True
    return changed


def want(pcells):
    """The set of PCell views from now on (the project gained or lost some); takes
    effect with the next `refresh`."""
    global _wanted
    _wanted = dict(pcells or {})


def refresh():
    """Look at the files of the last table again (a file was saved). True when a
    library was registered again: KLayout has rebuilt the instances."""
    return sync(_wanted)


def libraries():
    """The CDePT libraries registered now."""
    return sorted(_registered)
