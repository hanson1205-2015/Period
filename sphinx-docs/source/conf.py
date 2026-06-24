"""Sphinx configuration for Period documentation."""
import os
import sys

sys.path.insert(0, os.path.abspath('../../'))

project = 'Period'
copyright = '2026, Period Language'
author = 'Period Language'
release = '1.0.0'

extensions = [
    'sphinx.ext.autodoc',
    'sphinx.ext.viewcode',
    'sphinx.ext.napoleon',
]

templates_path = ['_templates']
exclude_patterns = []

html_theme = 'furo'
html_static_path = ['_static']
html_logo = '../../assets/period.png'
html_favicon = '../../assets/period.ico'

html_theme_options = {
    "sidebar_hide_name": True,
}
