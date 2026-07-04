"""Build a distribution of Period.

The distribution contains:
  - period.exe        tiny fast-path wrapper (C)
  - period-core.dll   full Rust interpreter / LSP server as an in-process DLL
  - period-core.exe   fallback standalone core executable
  - stdlib/           Period standard library stubs
  - period.ico        Windows icon

Run with:
    python scripts/build_dist.py
"""
from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PERIOD_DIR = ROOT / "period"
TCC_EXE = ROOT / ".tools" / "tcc" / "tcc" / "tcc.exe"
DIST = ROOT / "dist"
SET_VERSION = ROOT / "scripts" / "set_version.py"


def run(cmd: list[str | Path], cwd: Path | None = None) -> None:
    print("$", " ".join(str(c) for c in cmd))
    subprocess.run([str(c) for c in cmd], cwd=cwd, check=True)


def main() -> None:
    if not TCC_EXE.exists():
        print(f"TCC not found at {TCC_EXE}")
        sys.exit(1)

    print("Synchronising version numbers with git tag...")
    run(["python", SET_VERSION])

    print("Building release Rust binary...")
    run(["cargo", "build", "--release"], cwd=PERIOD_DIR)

    print("Preparing dist directory...")
    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir(parents=True)

    release = PERIOD_DIR / "target" / "release"
    shutil.copy(release / "period_core.dll", DIST / "period-core.dll")
    shutil.copy(release / "period.exe", DIST / "period-core.exe")
    shutil.copytree(ROOT / "period" / "stdlib", DIST / "stdlib")
    shutil.copy(ROOT / "assets" / "period.ico", DIST / "period.ico")

    print("Compiling fast-path wrapper...")
    run([TCC_EXE, PERIOD_DIR / "wrapper.c", "-o", DIST / "period.exe"])

    print(f"Done. Distribution is in {DIST}")


if __name__ == "__main__":
    main()
