"""Longer-running execution-speed benchmark across languages.

This compares raw interpreter/compiled performance, not startup time.
Run with:
    python docs/benchmark_long.py
"""
from __future__ import annotations

import os
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


def augment_path() -> None:
    """Add common Windows install locations to PATH so winget-installed
    toolchains are discoverable even when the current shell was not restarted."""
    additions = []
    candidates = [
        Path(r"C:\Program Files\Go\bin"),
        Path(r"C:\Program Files\dotnet"),
    ]
    for c in candidates:
        if c.exists():
            additions.append(str(c))
    if Path(r"C:\Program Files\Eclipse Adoptium").exists():
        for jdk_bin in Path(r"C:\Program Files\Eclipse Adoptium").glob("jdk-*/bin"):
            additions.append(str(jdk_bin))
    if additions:
        os.environ["PATH"] = os.environ.get("PATH", "") + ";" + ";".join(additions)


def has_dotnet_sdk() -> bool:
    try:
        result = subprocess.run(
            ["dotnet", "--list-sdks"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        return result.returncode == 0 and result.stdout.strip() != ""
    except Exception:
        return False


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
    if lang == "rust":
        return (
            f"fn main() {{\n"
            f"    let n: i64 = {n};\n"
            f"    let mut s: i64 = 0;\n"
            f"    for i in 1..=n {{ s += i; }}\n"
            f"    println!(\"{{}}\", s);\n"
            f"}}\n"
        )
    if lang == "go":
        return (
            f"package main\n"
            f"import \"fmt\"\n"
            f"func main() {{\n"
            f"    var s int64 = 0\n"
            f"    var n int64 = {n}\n"
            f"    for i := int64(1); i <= n; i++ {{ s += i }}\n"
            f"    fmt.Println(s)\n"
            f"}}\n"
        )
    if lang == "java":
        return (
            f"class Main {{\n"
            f"    public static void main(String[] args) {{\n"
            f"        long s = 0;\n"
            f"        long n = {n}L;\n"
            f"        for (long i = 1; i <= n; i++) s += i;\n"
            f"        System.out.println(s);\n"
            f"    }}\n"
            f"}}\n"
        )
    if lang == "csharp":
        return (
            f"using System;\n"
            f"class Program {{\n"
            f"    static void Main() {{\n"
            f"        long s = 0;\n"
            f"        long n = {n}L;\n"
            f"        for (long i = 1; i <= n; i++) s += i;\n"
            f"        Console.WriteLine(s);\n"
            f"    }}\n"
            f"}}\n"
        )
    if lang == "ruby":
        return (
            f"s = 0\n"
            f"n = {n}\n"
            f"(1..n).each {{ |i| s += i }}\n"
            f"puts s\n"
        )
    if lang == "php":
        return (
            f"<?php\n"
            f"$s = 0;\n"
            f"$n = {n};\n"
            f"for ($i = 1; $i <= $n; $i++) $s += $i;\n"
            f"echo $s . \"\\n\";\n"
        )
    if lang == "lua":
        return (
            f"local s = 0\n"
            f"local n = {n}\n"
            f"for i = 1, n do s = s + i end\n"
            f"print(s)\n"
        )
    if lang == "powershell":
        return (
            f"$n = {n}\n"
            f"$s = [long]0\n"
            f"for ($i = 1; $i -le $n; $i++) {{ $s += $i }}\n"
            f"Write-Output $s\n"
        )
    raise ValueError(lang)


def run(cmd: list[str], source: str, ext: str, n: int, runs: int = 3) -> float | None:
    with tempfile.NamedTemporaryFile(mode="w", suffix=ext, delete=False) as f:
        f.write(source)
        src = Path(f.name)

    compiled_exts = {".c", ".rs", ".go"}
    exe = src.with_suffix(".exe")

    if ext == ".c":
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
    elif ext == ".rs":
        result = subprocess.run(
            ["rustc", "-O", str(src), "-o", str(exe)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode != 0:
            print(f"compile failed: {result.stderr}")
            return None
        run_cmd = [str(exe)]
    elif ext == ".go":
        result = subprocess.run(
            ["go", "build", "-o", str(exe), str(src)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode != 0:
            print(f"compile failed: {result.stderr}")
            return None
        run_cmd = [str(exe)]
    elif ext == ".java":
        result = subprocess.run(
            ["javac", str(src)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode != 0:
            print(f"compile failed: {result.stderr}")
            return None
        run_cmd = ["java", "-cp", str(src.parent), src.stem]
    elif ext == ".cs":
        # Requires the .NET SDK (dotnet new / build).
        proj_dir = src.parent / src.stem
        proj_dir.mkdir(exist_ok=True)
        result = subprocess.run(
            ["dotnet", "new", "console", "--force", "-o", str(proj_dir)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode != 0:
            print(f"dotnet new failed: {result.stderr}")
            return None
        (proj_dir / "Program.cs").write_text(source)
        result = subprocess.run(
            ["dotnet", "build", "-c", "Release", str(proj_dir)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode != 0:
            print(f"dotnet build failed: {result.stderr}")
            return None
        exe_candidates = list((proj_dir / "bin" / "Release").glob("net*/*.exe"))
        if not exe_candidates:
            print("dotnet build did not produce an executable")
            return None
        run_cmd = [str(exe_candidates[0])]
    elif ext == ".ps1":
        run_cmd = ["powershell", "-ExecutionPolicy", "Bypass", "-File", str(src)]
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
    if ext in compiled_exts:
        exe.unlink(missing_ok=True)
    if ext == ".java":
        for cls in src.parent.glob("*.class"):
            cls.unlink(missing_ok=True)
    if ext == ".cs":
        shutil.rmtree(proj_dir, ignore_errors=True)

    return sum(times) / len(times) * 1000


def main() -> None:
    augment_path()

    if not PERIOD_EXE.exists():
        print(f"Period not found at {PERIOD_EXE}; run scripts/build_dist.py first")
        return
    if not TCC_EXE.exists():
        print(f"TCC not found at {TCC_EXE}")
        return

    languages = [
        ("C (Release)", [str(TCC_EXE)], ".c"),
        ("Period", [str(PERIOD_EXE)], ".period"),
        ("Rust", ["rustc"], ".rs"),
        ("Go", ["go"], ".go"),
        ("Java", ["javac"], ".java"),
        ("C#", ["dotnet"], ".cs"),
    ]

    # The dotnet runtime alone is not enough to compile C#; require the SDK.
    if not has_dotnet_sdk():
        languages = [entry for entry in languages if entry[0] != "C#"]

    lang_key = {
        "C (Release)": "c",
        "Period": "period",
        "Rust": "rust",
        "Go": "go",
        "Java": "java",
        "C#": "csharp",
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
