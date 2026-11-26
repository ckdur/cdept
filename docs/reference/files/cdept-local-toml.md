# cdept.local.toml

`cdept.local.toml` holds what differs per user and per machine for one project: where
PDKs are installed and some extra variables. It sits next to [cdept.toml](cdept-toml.md)
and is not committed to git.

```toml
# CDePT settings of this user and this machine for the project next to this file.
# Not for version control (CDePT adds the file to .gitignore). What is set here wins
# over cdept.toml.

# Where PDKs are installed on this machine ($PDK_ROOT):
pdk_root = "/opt/ext/OpenPDKs"

# Variables for $VAR in the project's paths, also given to every tool CDePT starts:
[env]
MY_LIBS = "/work/libs"
```

The file is optional. Without it, `$PDK_ROOT` comes from `pdk_root` in `cdept.toml`,
else from the environment.

## Keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `pdk_root` | string | not set | The folder PDKs are installed in: `$PDK_ROOT` for library paths and for every tool. A relative path is relative to the project folder. `$VAR` and a leading `~` are expanded; `$PDK_ROOT` itself is not allowed here. An empty string counts as not set. |
| `python` | string | not set | This user's Python for the project's Python tools: a program, or a virtual environment folder. Wins over `[python]` of `cdept.toml`. Relative to the project folder. |
| `[env]` | table of strings | empty | Variables for `$VAR` in the paths of `cdept.toml`, also added to the environment of every tool CDePT starts for the project. |

`[env]` rules:

- A name uses letters, digits and `_` and does not start with a digit.
- A value is a string. It may use `~` and `$VAR` of the process environment.
- `PDK_ROOT`, `PDK` and `PDKPATH` set here are not passed on to tools as given: those
  three always come from the resolution of the project's PDK (`pdk_root` above, the
  project's `technology`).
- In path expansion the lookup order is: `$PDK_ROOT` and `$PDK` of the project, then
  `[env]`, then the process environment.

Any other key is a warning (`unknown key ..., known: pdk_root, python, [env]`) and is ignored.
A value of the wrong type is a warning for that entry only; the rest of the file is
used. A file that is not valid TOML is a warning and counts as empty.

## Precedence

`cdept.local.toml` wins over `cdept.toml`, and `cdept.toml` over `tech.toml` and the
environment. For `pdk_root`:

1. `pdk_root` in `cdept.local.toml`
2. `pdk_root` in `cdept.toml` (still read, but CDePT no longer writes it there)
3. `PDK_ROOT` in the environment CDePT was started in

`cdept info` prints the value in use and which of these gave it:

```text
PDK          ihp-sg13g2  (project)
PDK_ROOT     /opt/ext/OpenPDKs  (cdept.local.toml)
env          MY_LIBS=/home/me/x  (cdept.local.toml)
```

## Who writes it

CDePT writes the file only when it is needed:

- `cdept init --pdk-root DIR` and the New Project wizard write `pdk_root`.
- A PDK scan (`cdept init --technology`, `cdept tech scan` and `refresh`) writes
  `pdk_root` when the PDK is not in the folder the project would otherwise use, and
  removes the line when the environment already points at it.
- Project Settings in the GUI changes `pdk_root` here, not in
  `cdept.toml`.

A new file starts from the commented template shown above; when `pdk_root` is later
removed the line stays as a comment, so setting it again finds its place. Comments and
other keys are kept when CDePT edits the file.

## Git

When CDePT creates the file inside a git repository it adds a line `cdept.local.toml`
to the `.gitignore` in the project folder, unless the file is already ignored (by the
name, by `/cdept.local.toml` or by `*.local.toml`). A project outside a repository gets
no `.gitignore`. A watcher in the GUI treats a change of this file like a change of
`cdept.toml` and loads the project again.

## See also

- [cdept.toml](cdept-toml.md): the project file and the precedence of all the files
- [tech.toml](tech-toml.md): the technology snapshot
- [Environment variables](../environment.md)
- [TECH.md](../../TECH.md): `$PDK`, `$PDK_ROOT` and the scan
- [Project folders](project-folders.md)
