"""Module resolution for the Period import system."""
from pathlib import Path
from typing import Optional, Union

try:
    from .stdlib import BUILTIN_MODULES
except Exception:  # pragma: no cover - stdlib may be missing in minimal installs
    BUILTIN_MODULES = {}


ModuleRef = Union[Path, str]


def _base_dir(importer_filename: str) -> Path:
    if importer_filename and not importer_filename.startswith("<"):
        return Path(importer_filename).resolve().parent
    return Path.cwd()


def resolve_module(module_path: str, importer_filename: str = "<stdin>") -> Optional[ModuleRef]:
    """Resolve a Period module path to a file path or built-in module name.

    Relative imports (``.abc``, ``..abc``) are resolved from the importer's
    directory. Absolute imports first look next to the importer, then in the
    ``lib/`` directory of the current working directory.
    """
    base_dir = _base_dir(importer_filename)
    stripped = module_path.lstrip(".")
    dots = len(module_path) - len(stripped)
    rel_file = stripped.replace(".", "/") + ".period"

    if dots == 0:
        # Absolute import: local file first, then built-in lib/ directory.
        candidate = base_dir / rel_file
        if candidate.exists():
            return candidate
        lib_candidate = Path.cwd() / "lib" / rel_file
        if lib_candidate.exists():
            return lib_candidate
        # Fall back to built-in module.
        if module_path in BUILTIN_MODULES:
            return module_path
        return None

    # Relative import: one dot = importer's directory, two dots = parent, etc.
    relative_base = base_dir
    for _ in range(dots - 1):
        relative_base = relative_base.parent
    candidate = relative_base / rel_file
    return candidate if candidate.exists() else None
