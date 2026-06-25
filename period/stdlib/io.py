"""Input/output utilities for Period."""
from pathlib import Path

EXPORTS = [
    "read",
    "write",
]


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    Path(path).write_text(content, encoding="utf-8")
