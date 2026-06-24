"""Setup script for the Period language."""
from setuptools import setup, find_packages

setup(
    name="period-lang",
    version="0.0.1",
    description="Period - an elegant English programming language ending every statement with a period.",
    author="Period Language",
    license="MIT",
    packages=find_packages(),
    python_requires=">=3.9",
    entry_points={
        "console_scripts": [
            "period=compiler:main",
        ],
    },
    install_requires=[
        "Pillow>=10.0.0",
    ],
)
