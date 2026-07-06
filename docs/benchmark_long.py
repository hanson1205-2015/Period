"""Longer-running execution-speed benchmark for Period.

Period is a single tree-walking interpreter, so it is not intended to compete
with compiled languages on numeric loops. This script tracks Period's own
performance over time and shows where it stands relative to common compiled
and JIT implementations for reference only.

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


def c_compiler_candidates(src: Path, exe: Path) -> list[list[str]]:
    """Return C compiler commands to try, from strongest to fallback.

    Prefer a real optimizing compiler (MSVC cl /O2, gcc -O2, clang -O2).
    Also look in common Windows install locations. If nothing works, fall
    back to the bundled TCC with -O2.
    """
    candidates: list[list[str]] = []

    if shutil.which("cl"):
        candidates.append(["cl", "/O2", f"/Fe:{exe}", str(src)])

    gcc_locations = [
        Path(r"C:\msys64\mingw64\bin\gcc.exe"),
        Path(r"C:\mingw64\bin\gcc.exe"),
    ]
    for gcc in gcc_locations:
        if gcc.exists():
            candidates.append([str(gcc), "-O2", "-march=native", str(src), "-o", str(exe)])
            break
    if shutil.which("gcc"):
        candidates.append(["gcc", "-O2", "-march=native", str(src), "-o", str(exe)])

    clang_locations = [
        Path(r"C:\Program Files\LLVM\bin\clang.exe"),
        Path(r"C:\Program Files (x86)\LLVM\bin\clang.exe"),
        Path(r"C:\msys64\mingw64\bin\clang.exe"),
    ]
    for clang in clang_locations:
        if clang.exists():
            candidates.append([str(clang), "-O2", "-march=native", str(src), "-o", str(exe)])
            break
    if shutil.which("clang"):
        candidates.append(["clang", "-O2", "-march=native", str(src), "-o", str(exe)])

    candidates.append([str(TCC_EXE), "-O2", str(src), "-o", str(exe)])
    return candidates


WORKLOADS = {
    "sum": 20_000_000,
    "div3or5": 20_000_000,
    "count_div3and5": 20_000_000,
    "sum_multiples_3or5": 20_000_000,
}


def source_for(lang: str, workload: str, n: int) -> str:
    if workload == "sum":
        if lang == "period":
            return (
                f"let sum be 0.\n"
                f"let i be 1.\n"
                f"while i <= {n} repeat:\n"
                f"    set sum to sum + i.\n"
                f"    set i to i + 1.\n"
                f"show sum.\n"
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

    if workload == "div3or5":
        if lang == "period":
            return (
                f"let count be 0.\n"
                f"let i be 1.\n"
                f"while i <= {n} repeat:\n"
                f"    if i % 3 == 0 or i % 5 == 0 then:\n"
                f"        set count to count + 1.\n"
                f"    set i to i + 1.\n"
                f"show count.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"int main(void) {{\n"
                f"    long long count = 0;\n"
                f"    long long n = {n}LL;\n"
                f"    for (long long i = 1; i <= n; i++)\n"
                f"        if (i % 3 == 0 || i % 5 == 0) count++;\n"
                f"    printf(\"%lld\\n\", count);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"fn main() {{\n"
                f"    let n: i64 = {n};\n"
                f"    let mut count = 0;\n"
                f"    for i in 1..=n {{\n"
                f"        if i % 3 == 0 || i % 5 == 0 {{ count += 1; }}\n"
                f"    }}\n"
                f"    println!(\"{{}}\", count);\n"
                f"}}\n"
            )
        if lang == "go":
            return (
                f"package main\n"
                f"import \"fmt\"\n"
                f"func main() {{\n"
                f"    var n int64 = {n}\n"
                f"    var count int64 = 0\n"
                f"    for i := int64(1); i <= n; i++ {{\n"
                f"        if i%3 == 0 || i%5 == 0 {{ count++ }}\n"
                f"    }}\n"
                f"    fmt.Println(count)\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"class Main {{\n"
                f"    public static void main(String[] args) {{\n"
                f"        long count = 0;\n"
                f"        long n = {n}L;\n"
                f"        for (long i = 1; i <= n; i++)\n"
                f"            if (i % 3 == 0 || i % 5 == 0) count++;\n"
                f"        System.out.println(count);\n"
                f"    }}\n"
                f"}}\n"
            )
        if lang == "csharp":
            return (
                f"using System;\n"
                f"class Program {{\n"
                f"    static void Main() {{\n"
                f"        long count = 0;\n"
                f"        long n = {n}L;\n"
                f"        for (long i = 1; i <= n; i++)\n"
                f"            if (i % 3 == 0 || i % 5 == 0) count++;\n"
                f"        Console.WriteLine(count);\n"
                f"    }}\n"
                f"}}\n"
            )

    if workload == "sum_multiples_3or5":
        if lang == "period":
            return (
                f"let sum be 0.\n"
                f"let i be 1.\n"
                f"while i <= {n} repeat:\n"
                f"    if i % 3 == 0 or i % 5 == 0 then:\n"
                f"        set sum to sum + i.\n"
                f"    set i to i + 1.\n"
                f"show sum.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"int main(void) {{\n"
                f"    long long s = 0;\n"
                f"    long long n = {n}LL;\n"
                f"    for (long long i = 1; i <= n; i++)\n"
                f"        if (i % 3 == 0 || i % 5 == 0) s += i;\n"
                f"    printf(\"%lld\\n\", s);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"fn main() {{\n"
                f"    let n: i64 = {n};\n"
                f"    let mut s: i64 = 0;\n"
                f"    for i in 1..=n {{\n"
                f"        if i % 3 == 0 || i % 5 == 0 {{ s += i; }}\n"
                f"    }}\n"
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
                f"    for i := int64(1); i <= n; i++ {{\n"
                f"        if i%3 == 0 || i%5 == 0 {{ s += i }}\n"
                f"    }}\n"
                f"    fmt.Println(s)\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"class Main {{\n"
                f"    public static void main(String[] args) {{\n"
                f"        long s = 0;\n"
                f"        long n = {n}L;\n"
                f"        for (long i = 1; i <= n; i++)\n"
                f"            if (i % 3 == 0 || i % 5 == 0) s += i;\n"
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
                f"        for (long i = 1; i <= n; i++)\n"
                f"            if (i % 3 == 0 || i % 5 == 0) s += i;\n"
                f"        Console.WriteLine(s);\n"
                f"    }}\n"
                f"}}\n"
            )

    if workload == "count_div3and5":
        if lang == "period":
            return (
                f"let count be 0.\n"
                f"let i be 1.\n"
                f"while i <= {n} repeat:\n"
                f"    if i % 3 == 0 and i % 5 == 0 then:\n"
                f"        set count to count + 1.\n"
                f"    set i to i + 1.\n"
                f"show count.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"int main(void) {{\n"
                f"    long long count = 0;\n"
                f"    long long n = {n}LL;\n"
                f"    for (long long i = 1; i <= n; i++)\n"
                f"        if (i % 3 == 0 && i % 5 == 0) count++;\n"
                f"    printf(\"%lld\\n\", count);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"fn main() {{\n"
                f"    let n: i64 = {n};\n"
                f"    let mut count = 0;\n"
                f"    for i in 1..=n {{\n"
                f"        if i % 3 == 0 && i % 5 == 0 {{ count += 1; }}\n"
                f"    }}\n"
                f"    println!(\"{{}}\", count);\n"
                f"}}\n"
            )
        if lang == "go":
            return (
                f"package main\n"
                f"import \"fmt\"\n"
                f"func main() {{\n"
                f"    var n int64 = {n}\n"
                f"    var count int64 = 0\n"
                f"    for i := int64(1); i <= n; i++ {{\n"
                f"        if i%3 == 0 && i%5 == 0 {{ count++ }}\n"
                f"    }}\n"
                f"    fmt.Println(count)\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"class Main {{\n"
                f"    public static void main(String[] args) {{\n"
                f"        long count = 0;\n"
                f"        long n = {n}L;\n"
                f"        for (long i = 1; i <= n; i++)\n"
                f"            if (i % 3 == 0 && i % 5 == 0) count++;\n"
                f"        System.out.println(count);\n"
                f"    }}\n"
                f"}}\n"
            )
        if lang == "csharp":
            return (
                f"using System;\n"
                f"class Program {{\n"
                f"    static void Main() {{\n"
                f"        long count = 0;\n"
                f"        long n = {n}L;\n"
                f"        for (long i = 1; i <= n; i++)\n"
                f"            if (i % 3 == 0 && i % 5 == 0) count++;\n"
                f"        Console.WriteLine(count);\n"
                f"    }}\n"
                f"}}\n"
            )

    raise ValueError((lang, workload))


def run(cmd: list[str], source: str, ext: str, n: int, runs: int = 10) -> float | None:
    with tempfile.NamedTemporaryFile(mode="w", suffix=ext, delete=False) as f:
        f.write(source)
        src = Path(f.name)

    compiled_exts = {".c", ".rs", ".go"}
    exe = src.with_suffix(".exe")

    if ext == ".c":
        run_cmd = None
        last_error = ""
        for compile_cmd in c_compiler_candidates(src, exe):
            result = subprocess.run(
                compile_cmd,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True,
            )
            if result.returncode == 0:
                run_cmd = [str(exe)]
                break
            last_error = result.stderr
        if run_cmd is None:
            print(f"all C compilers failed; last error: {last_error}")
            return None
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
        src.with_suffix(".obj").unlink(missing_ok=True)
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

    for workload, n in WORKLOADS.items():
        print(f"\nWorkload: {workload} (n={n:,})")
        print(f"{'Language':<12}", end="")
        print(f"{'time (ms)':>15}")
        print("-" * 28)

        for name, cmd, ext in languages:
            first = cmd[0]
            if shutil.which(first) is None and not Path(first).exists():
                continue
            ms = run(cmd, source_for(lang_key[name], workload, n), ext, n)
            if ms is None:
                print(f"{name:<12}{'failed':>15}")
            else:
                print(f"{name:<12}{ms:>14.1f}ms")


if __name__ == "__main__":
    main()
