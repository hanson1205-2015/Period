"""Longer-running execution-speed benchmark across languages.

This compares raw interpreter/compiled performance, not startup time.
Run with:
    python docs/benchmark_long.py
"""
from __future__ import annotations

import shutil
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
PERIOD_EXE = DIST / "period.exe"
TCC_EXE = ROOT / ".tools" / "tcc" / "tcc" / "tcc.exe"

NS = [1_000_000, 5_000_000]


def find_release_c_compiler() -> list[str] | None:
    """Return an optimizing C compiler command if one is available.

    Prefer a real optimizing compiler (MSVC cl /O2, gcc -O2, clang -O2).
    If none are on PATH, fall back to the bundled TCC with -O2.
    """
    if shutil.which("cl"):
        return ["cl", "/O2"]
    if shutil.which("gcc"):
        return ["gcc", "-O2"]
    if shutil.which("clang"):
        return ["clang", "-O2"]
    return None


def source_for(lang: str, n: int) -> str:
    if lang == "period":
        return (
            f"let sum be 0.\n"
            f"let i be 1.\n"
            f"while i <= {n} repeat:\n"
            f"    set sum to sum + i.\n"
            f"    set i to i + 1.\n"
            f"show sum.\n"
        )
    if lang == "python":
        return (
            f"s = 0\n"
            f"for i in range(1, {n + 1}):\n"
            f"    s += i\n"
            f"print(s)\n"
        )
    if lang == "node":
        return (
            f"let s = 0;\n"
            f"for (let i = 1; i <= {n}; i++) {{\n"
            f"    s += i;\n"
            f"}}\n"
            f"console.log(s);\n"
        )
    if lang == "perl":
        return (
            f"my $s = 0;\n"
            f"for my $i (1..{n}) {{ $s += $i; }}\n"
            f"print \"$s\\n\";\n"
        )
    if lang == "c":
        return (
            f"#include <stdio.h>\n"
            f"int main(void) {{\n"
            f"    long long s = 0;\n"
            f"    for (long long i = 1; i <= {n}; i++) s += i;\n"
            f"    printf(\"%lld\\n\", s);\n"
            f"    return 0;\n"
            f"}}\n"
        )
    raise ValueError(lang)


def run(cmd: list[str], source: str, ext: str, n: int, runs: int = 3) -> float | None:
    with tempfile.NamedTemporaryFile(mode="w", suffix=ext, delete=False) as f:
        f.write(source)
        src = Path(f.name)

    if ext == ".c":
        exe = src.with_suffix(".exe")
        release_cc = find_release_c_compiler()
        if release_cc:
            compile_cmd = release_cc + [str(src), "-o", str(exe)]
        else:
            compile_cmd = [str(TCC_EXE), "-O2", str(src), "-o", str(exe)]
        result = subprocess.run(
            compile_cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode != 0:
            print(f"compile failed: {result.stderr}")
            return None
        run_cmd = [str(exe)]
    else:
        run_cmd = cmd + [str(src)]

    # Warm-up.
    subprocess.run(run_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    times = []
    for _ in range(runs):
        start = time.perf_counter()
        subprocess.run(run_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        times.append(time.perf_counter() - start)

    src.unlink(missing_ok=True)
    if ext == ".c":
        exe.unlink(missing_ok=True)

    return sum(times) / len(times) * 1000


def main() -> None:
    if not PERIOD_EXE.exists():
        print(f"Period not found at {PERIOD_EXE}; run scripts/build_dist.py first")
        return
    if not TCC_EXE.exists():
        print(f"TCC not found at {TCC_EXE}")
        return

    languages = [
        ("C (Release)", [str(TCC_EXE)], ".c"),
        ("Period", [str(PERIOD_EXE)], ".period"),
        ("Python", ["python"], ".py"),
        ("Node.js", ["node"], ".js"),
        ("Perl", ["perl"], ".pl"),
    ]

    lang_key = {
        "C (Release)": "c",
        "Period": "period",
        "Python": "python",
        "Node.js": "node",
        "Perl": "perl",
    }

    print(f"{'Language':<12}", end="")
    for n in NS:
        print(f"{n:>15,}", end="")
    print()
    print("-" * (12 + 15 * len(NS)))

    for name, cmd, ext in languages:
        first = cmd[0]
        if shutil.which(first) is None and not Path(first).exists():
            continue
        print(f"{name:<12}", end="")
        for n in NS:
            ms = run(cmd, source_for(lang_key[name], n), ext, n)
            if ms is None:
                print(f"{'failed':>15}", end="")
            else:
                print(f"{ms:>14.1f}ms", end="")
        print()


if __name__ == "__main__":
    main()
