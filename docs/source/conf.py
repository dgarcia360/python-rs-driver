from __future__ import annotations

import sys
from pathlib import Path

import tomllib
from sphinx_scylladb_theme.utils import multiversion_regex_builder

DOCS_SOURCE = Path(__file__).resolve().parent
DOCS_DIR = DOCS_SOURCE.parent
REPO_ROOT = DOCS_DIR.parent
PYTHON_SOURCE = REPO_ROOT / "python"

# Point Sphinx to the Python source directory.
sys.path.insert(0, str(PYTHON_SOURCE.resolve()))


# -- Global variables --------------------------------------------------

# Publish only the default branch for now, under the stable URL.
TAGS = []

BRANCHES = ["main"]

LATEST_VERSION = "main"

UNSTABLE_VERSIONS = []

DEPRECATED_VERSIONS = []


# -- General configuration ---------------------------------------------

extensions = [
    "sphinx.ext.autodoc",
    "sphinx.ext.napoleon",
    "sphinx.ext.viewcode",
    "sphinx.ext.githubpages",
    "sphinx_sitemap",
    "sphinx_multiversion",
    "myst_parser",
    "sphinx_scylladb_theme",
    "sphinx_autodoc_typehints",
]

source_suffix = {
    ".rst": "restructuredtext",
    ".md": "markdown",
}

master_doc = "index"

project = "ScyllaDB Python RS Driver"

copyright = "2026, ScyllaDB"

author = "ScyllaDB"

with (REPO_ROOT / "Cargo.toml").open("rb") as cargo_toml:
    release = tomllib.load(cargo_toml)["package"]["version"]

version = release

exclude_patterns = ["_build", "Thumbs.db", ".DS_Store", "**/_partials"]


# -- Options for autodoc extension -------------------------------------

autodoc_default_options = {
    "members": True,
    "undoc-members": False,
    "show-inheritance": True,
}

autodoc_mock_imports = ["scylla._rust"]

# Hide parent class names in sidebar navigation.
toc_object_entries_show_parents = "hide"


# -- Options for myst parser -------------------------------------------

myst_enable_extensions = ["colon_fence"]

myst_heading_anchors = 3


# -- Options for not found extension -----------------------------------

# Use the theme's 404 page with assets served from the custom domain root.
notfound_template = "404.html"

notfound_urls_prefix = ""


# -- Options for sitemap extension -------------------------------------

sitemap_url_scheme = "/stable/{link}"


# -- Options for multiversion extension --------------------------------

smv_tag_whitelist = multiversion_regex_builder(TAGS)

smv_branch_whitelist = multiversion_regex_builder(BRANCHES)

smv_latest_version = LATEST_VERSION

smv_rename_latest_version = "stable"

smv_remote_whitelist = r"^origin$"

smv_released_pattern = r"^tags/.*$"

smv_outputdir_format = "{ref.name}"


# -- Options for HTML output -------------------------------------------

html_theme = "sphinx_scylladb_theme"

html_theme_options = {  # type: ignore
    "conf_py_path": "docs/source/",
    "github_repository": "scylladb/python-rs-driver",
    "github_issues_repository": "scylladb/python-rs-driver",
    "default_branch": "main",
    "site_description": "Documentation for the asynchronous ScyllaDB Python RS Driver.",
    "hide_edit_this_page_button": "false",
    "hide_feedback_buttons": "false",
    "hide_version_dropdown": [],
    "versions_unstable": UNSTABLE_VERSIONS,
    "versions_deprecated": DEPRECATED_VERSIONS,
}

html_title = "ScyllaDB Python RS Driver Documentation"

html_short_title = "Python RS Driver Docs"

html_sidebars = {"**": ["side-nav.html"]}

# Root URL for the multiversion GitHub Pages deployment.
html_baseurl = "https://python-rs-driver.docs.scylladb.com"

html_context = {"html_baseurl": html_baseurl}
