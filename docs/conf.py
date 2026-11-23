# Sphinx configuration for the CDePT manual (Read the Docs style).
# Build:  pip install -r docs/requirements.txt && sphinx-build -b html docs docs/_build/html
project = "CDePT"
author = "CDePT contributors"
copyright = "CDePT contributors, MIT licence"

extensions = ["myst_parser"]
source_suffix = {".md": "markdown"}
master_doc = "index"
language = "en"

myst_enable_extensions = ["colon_fence", "deflist", "tasklist"]
myst_heading_anchors = 3

html_theme = "sphinx_rtd_theme"
html_theme_options = {
    "navigation_depth": 3,
    "collapse_navigation": False,
    "sticky_navigation": True,
    "titles_only": False,
}
html_static_path = ["_static"]
html_title = "CDePT manual"

exclude_patterns = ["_build", "HANDOFF.md", "requirements.txt"]

# The design notes are written for the code and link to files outside the manual.
suppress_warnings = ["toc.not_included", "misc.highlighting_failure"]
