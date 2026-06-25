"""Build script that compiles compiler.py into a standalone period.exe using Nuitka."""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DIST = ROOT / "dist"
DIST.mkdir(exist_ok=True)


def main():
    cmd = [
        sys.executable,
        "-m",
        "nuitka",
        "--standalone",
        "--onefile",
        "--windows-icon-from-ico=assets/period.ico",
        "--company-name=Period Language",
        "--product-name=Period",
        "--file-version=1.0.2.0",
        "--product-version=1.0.2.0",
        f"--output-dir={DIST}",
        "--output-filename=period",
        "compiler.py",
    ]
    print("Running:", " ".join(cmd))
    subprocess.run(cmd, check=True)
    exe = DIST / "period.exe"
    print(f"Built: {exe}")
    print(f"Size: {exe.stat().st_size} bytes")


if __name__ == "__main__":
    main()
