"""Input/output utilities for Period."""
from pathlib import Path

EXPORTS = [
    "read",
    "write",
]


def read(path: str) -> str:
    """Read the contents of a file as a string."""
    return Path(path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    """Write a string to a file."""
    Path(path).write_text(content, encoding="utf-8")


# Hover documentation.
DOCS = {
    "read": ("read with <path> -> string", "Read the contents of a file as a string."),
    "write": ("write with <path>, <content>", "Write a string to a file."),
}
