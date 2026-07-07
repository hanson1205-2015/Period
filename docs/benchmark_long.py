"""Longer-running execution-speed benchmark for Period.

Period now compiles and runs programs through its own Cranelift-based JIT by
default. This benchmark tracks Period's own performance over time and shows
where it stands relative to common compiled and JIT implementations for
reference only.  It covers numeric loops as well as strings, lists, function
calls, object instantiation, and exception handling.

Run with:
    python docs/benchmark_long.py
"""
from __future__ import annotations

import hashlib
import math
import os
import shutil
import socket
import struct
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
PERIOD_EXE = DIST / "period.exe"
TCC_EXE = ROOT / ".tools" / "tcc" / "tcc" / "tcc.exe"
WORKER_PORT = 52691

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
            text=True, errors="replace",
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
    "string_concat": 100_000,
    "list_grow": 10_000,
    "count_calls": 100_000,
    "class_new": 100_000,
    "try_catch": 100_000,
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

    if workload == "string_concat":
        if lang == "period":
            return (
                f"let s be \"\".\n"
                f"let i be 0.\n"
                f"while i < {n} repeat:\n"
                f"    set s to s + \"a\".\n"
                f"    set i to i + 1.\n"
                f"show length with s.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"#include <stdlib.h>\n"
                f"int main(void) {{\n"
                f"    char *s = malloc({n} + 1);\n"
                f"    size_t len = 0;\n"
                f"    for (size_t i = 0; i < (size_t){n}; i++) {{\n"
                f"        s[len++] = 'a';\n"
                f"    }}\n"
                f"    s[len] = '\\0';\n"
                f"    printf(\"%zu\\n\", len);\n"
                f"    free(s);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"fn main() {{\n"
                f"    let mut s = String::new();\n"
                f"    for _ in 0..{n} {{ s.push('a'); }}\n"
                f"    println!(\"{{}}\", s.len());\n"
                f"}}\n"
            )
        if lang == "go":
            return (
                f"package main\n"
                f"import \"fmt\"\n"
                f"func main() {{\n"
                f"    s := \"\"\n"
                f"    for i := 0; i < {n}; i++ {{ s += \"a\" }}\n"
                f"    fmt.Println(len(s))\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"class Main {{\n"
                f"    public static void main(String[] args) {{\n"
                f"        StringBuilder sb = new StringBuilder();\n"
                f"        for (int i = 0; i < {n}; i++) sb.append('a');\n"
                f"        System.out.println(sb.length());\n"
                f"    }}\n"
                f"}}\n"
            )
        if lang == "csharp":
            return (
                f"using System;\n"
                f"using System.Text;\n"
                f"class Program {{\n"
                f"    static void Main() {{\n"
                f"        StringBuilder sb = new StringBuilder();\n"
                f"        for (int i = 0; i < {n}; i++) sb.Append('a');\n"
                f"        Console.WriteLine(sb.Length);\n"
                f"    }}\n"
                f"}}\n"
            )

    if workload == "list_grow":
        if lang == "period":
            return (
                f"let xs be [].\n"
                f"let i be 0.\n"
                f"while i < {n} repeat:\n"
                f"    set xs to xs + [i].\n"
                f"    set i to i + 1.\n"
                f"show length with xs.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"#include <stdlib.h>\n"
                f"int main(void) {{\n"
                f"    long long n = {n}LL;\n"
                f"    long long *arr = malloc(n * sizeof(long long));\n"
                f"    for (long long i = 0; i < n; i++) arr[i] = i;\n"
                f"    printf(\"%lld\\n\", n);\n"
                f"    free(arr);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"fn main() {{\n"
                f"    let n: usize = {n};\n"
                f"    let mut v = Vec::new();\n"
                f"    for i in 0..n {{ v.push(i as i64); }}\n"
                f"    println!(\"{{}}\", v.len());\n"
                f"}}\n"
            )
        if lang == "go":
            return (
                f"package main\n"
                f"import \"fmt\"\n"
                f"func main() {{\n"
                f"    var xs []int64\n"
                f"    for i := 0; i < {n}; i++ {{ xs = append(xs, int64(i)) }}\n"
                f"    fmt.Println(len(xs))\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"import java.util.ArrayList;\n"
                f"class Main {{\n"
                f"    public static void main(String[] args) {{\n"
                f"        ArrayList<Long> xs = new ArrayList<>();\n"
                f"        for (int i = 0; i < {n}; i++) xs.add((long)i);\n"
                f"        System.out.println(xs.size());\n"
                f"    }}\n"
                f"}}\n"
            )
        if lang == "csharp":
            return (
                f"using System;\n"
                f"using System.Collections.Generic;\n"
                f"class Program {{\n"
                f"    static void Main() {{\n"
                f"        var xs = new List<long>();\n"
                f"        for (int i = 0; i < {n}; i++) xs.Add(i);\n"
                f"        Console.WriteLine(xs.Count);\n"
                f"    }}\n"
                f"}}\n"
            )

    if workload == "count_calls":
        if lang == "period":
            return (
                f"define inc with x:\n"
                f"    return x + 1.\n"
                f"let r be 0.\n"
                f"let i be 0.\n"
                f"while i < {n} repeat:\n"
                f"    set r to inc with r.\n"
                f"    set i to i + 1.\n"
                f"show r.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"long long inc(long long x) {{ return x + 1; }}\n"
                f"int main(void) {{\n"
                f"    long long n = {n}LL;\n"
                f"    long long r = 0;\n"
                f"    for (long long i = 0; i < n; i++) r = inc(r);\n"
                f"    printf(\"%lld\\n\", r);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"fn inc(x: i64) -> i64 {{ x + 1 }}\n"
                f"fn main() {{\n"
                f"    let n: i64 = {n};\n"
                f"    let mut r = 0;\n"
                f"    for _ in 0..n {{ r = inc(r); }}\n"
                f"    println!(\"{{}}\", r);\n"
                f"}}\n"
            )
        if lang == "go":
            return (
                f"package main\n"
                f"import \"fmt\"\n"
                f"func inc(x int64) int64 {{ return x + 1 }}\n"
                f"func main() {{\n"
                f"    var r int64 = 0\n"
                f"    for i := 0; i < {n}; i++ {{ r = inc(r) }}\n"
                f"    fmt.Println(r)\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"class Main {{\n"
                f"    static long inc(long x) {{ return x + 1; }}\n"
                f"    public static void main(String[] args) {{\n"
                f"        long n = {n}L;\n"
                f"        long r = 0;\n"
                f"        for (long i = 0; i < n; i++) r = inc(r);\n"
                f"        System.out.println(r);\n"
                f"    }}\n"
                f"}}\n"
            )
        if lang == "csharp":
            return (
                f"using System;\n"
                f"class Program {{\n"
                f"    static long Inc(long x) => x + 1;\n"
                f"    static void Main() {{\n"
                f"        long n = {n}L;\n"
                f"        long r = 0;\n"
                f"        for (long i = 0; i < n; i++) r = Inc(r);\n"
                f"        Console.WriteLine(r);\n"
                f"    }}\n"
                f"}}\n"
            )

    if workload == "class_new":
        if lang == "period":
            return (
                f"class Counter:\n"
                f"    init with value:\n"
                f"        set this.value to value.\n"
                f"let total be 0.\n"
                f"let i be 0.\n"
                f"while i < {n} repeat:\n"
                f"    let c be new Counter(i).\n"
                f"    set total to total + c.value.\n"
                f"    set i to i + 1.\n"
                f"show total.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"typedef struct {{ long long value; }} Counter;\n"
                f"int main(void) {{\n"
                f"    long long n = {n}LL;\n"
                f"    long long total = 0;\n"
                f"    for (long long i = 0; i < n; i++) {{\n"
                f"        Counter c;\n"
                f"        c.value = i;\n"
                f"        total += c.value;\n"
                f"    }}\n"
                f"    printf(\"%lld\\n\", total);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"struct Counter {{ value: i64 }}\n"
                f"fn main() {{\n"
                f"    let n: i64 = {n};\n"
                f"    let mut total = 0;\n"
                f"    for i in 0..n {{ let c = Counter {{ value: i }}; total += c.value; }}\n"
                f"    println!(\"{{}}\", total);\n"
                f"}}\n"
            )
        if lang == "go":
            return (
                f"package main\n"
                f"import \"fmt\"\n"
                f"type Counter struct {{ value int64 }}\n"
                f"func main() {{\n"
                f"    var total int64 = 0\n"
                f"    for i := 0; i < {n}; i++ {{ c := Counter{{value: int64(i)}}; total += c.value }}\n"
                f"    fmt.Println(total)\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"class Counter {{\n"
                f"    long value;\n"
                f"    Counter(long v) {{ this.value = v; }}\n"
                f"}}\n"
                f"class Main {{\n"
                f"    public static void main(String[] args) {{\n"
                f"        long n = {n}L;\n"
                f"        long total = 0;\n"
                f"        for (long i = 0; i < n; i++) {{\n"
                f"            Counter c = new Counter(i);\n"
                f"            total += c.value;\n"
                f"        }}\n"
                f"        System.out.println(total);\n"
                f"    }}\n"
                f"}}\n"
            )
        if lang == "csharp":
            return (
                f"using System;\n"
                f"class Counter {{\n"
                f"    public long Value;\n"
                f"    public Counter(long v) {{ Value = v; }}\n"
                f"}}\n"
                f"class Program {{\n"
                f"    static void Main() {{\n"
                f"        long n = {n}L;\n"
                f"        long total = 0;\n"
                f"        for (long i = 0; i < n; i++) {{\n"
                f"            var c = new Counter(i);\n"
                f"            total += c.Value;\n"
                f"        }}\n"
                f"        Console.WriteLine(total);\n"
                f"    }}\n"
                f"}}\n"
            )

    if workload == "try_catch":
        if lang == "period":
            return (
                f"define may_error with i:\n"
                f"    if i % 2 == 0, then:\n"
                f"        error with \"even\".\n"
                f"    return i.\n"
                f"let caught be 0.\n"
                f"let i be 0.\n"
                f"while i < {n} repeat:\n"
                f"    try:\n"
                f"        may_error with i.\n"
                f"    catch e:\n"
                f"        set caught to caught + 1.\n"
                f"    set i to i + 1.\n"
                f"show caught.\n"
            )
        if lang == "c":
            return (
                f"#include <stdio.h>\n"
                f"#include <setjmp.h>\n"
                f"static jmp_buf env;\n"
                f"static const char *msg;\n"
                f"long long may_error(long long i) {{\n"
                f"    if (i % 2 == 0) {{ msg = \"even\"; longjmp(env, 1); }}\n"
                f"    return i;\n"
                f"}}\n"
                f"int main(void) {{\n"
                f"    long long n = {n}LL;\n"
                f"    long long caught = 0;\n"
                f"    for (long long i = 0; i < n; i++) {{\n"
                f"        if (setjmp(env) == 0) {{\n"
                f"            may_error(i);\n"
                f"        }} else {{\n"
                f"            caught++;\n"
                f"        }}\n"
                f"    }}\n"
                f"    printf(\"%lld\\n\", caught);\n"
                f"    return 0;\n"
                f"}}\n"
            )
        if lang == "rust":
            return (
                f"fn may_error(i: i64) -> i64 {{\n"
                f"    if i % 2 == 0 {{ panic!(\"even\"); }}\n"
                f"    i\n"
                f"}}\n"
                f"fn main() {{\n"
                f"    let n: i64 = {n};\n"
                f"    let mut caught = 0;\n"
                f"    for i in 0..n {{\n"
                f"        if std::panic::catch_unwind(|| may_error(i)).is_err() {{\n"
                f"            caught += 1;\n"
                f"        }}\n"
                f"    }}\n"
                f"    println!(\"{{}}\", caught);\n"
                f"}}\n"
            )
        if lang == "go":
            return (
                f"package main\n"
                f"import \"fmt\"\n"
                f"func may_error(i int64) int64 {{\n"
                f"    if i%2 == 0 {{ panic(\"even\") }}\n"
                f"    return i\n"
                f"}}\n"
                f"func main() {{\n"
                f"    var n int64 = {n}\n"
                f"    var caught int64 = 0\n"
                f"    for i := int64(0); i < n; i++ {{\n"
                f"        func() {{\n"
                f"            defer func() {{\n"
                f"                if recover() != nil {{ caught++ }}\n"
                f"            }}()\n"
                f"            may_error(i)\n"
                f"        }}()\n"
                f"    }}\n"
                f"    fmt.Println(caught)\n"
                f"}}\n"
            )
        if lang == "java":
            return (
                f"class MyException extends Exception {{\n"
                f"    MyException(String m) {{ super(m); }}\n"
                f"}}\n"
                f"class Main {{\n"
                f"    static long may_error(long i) throws MyException {{\n"
                f"        if (i % 2 == 0) throw new MyException(\"even\");\n"
                f"        return i;\n"
                f"    }}\n"
                f"    public static void main(String[] args) {{\n"
                f"        long n = {n}L;\n"
                f"        long caught = 0;\n"
                f"        for (long i = 0; i < n; i++) {{\n"
                f"            try {{ may_error(i); }}\n"
                f"            catch (MyException e) {{ caught++; }}\n"
                f"        }}\n"
                f"        System.out.println(caught);\n"
                f"    }}\n"
                f"}}\n"
            )
        if lang == "csharp":
            return (
                f"using System;\n"
                f"class MyException : Exception {{\n"
                f"    public MyException(string m) : base(m) {{}}\n"
                f"}}\n"
                f"class Program {{\n"
                f"    static long MayError(long i) {{\n"
                f"        if (i % 2 == 0) throw new MyException(\"even\");\n"
                f"        return i;\n"
                f"    }}\n"
                f"    static void Main() {{\n"
                f"        long n = {n}L;\n"
                f"        long caught = 0;\n"
                f"        for (long i = 0; i < n; i++) {{\n"
                f"            try {{ MayError(i); }}\n"
                f"            catch (MyException) {{ caught++; }}\n"
                f"        }}\n"
                f"        Console.WriteLine(caught);\n"
                f"    }}\n"
                f"}}\n"
            )

    return None


class PeriodServer:
    """Manage a long-running Period worker process.

    The worker avoids the ~90 ms startup cost of parsing, semantic analysis,
    type checking and module loading for every benchmark iteration.  It still
    compiles and executes each request, but compiled bytecode is cached in the
    server's thread-local storage on first use.
    """

    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def start(self) -> bool:
        if not PERIOD_EXE.exists():
            return False
        try:
            self.proc = subprocess.Popen(
                [str(PERIOD_EXE), "--server"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except Exception:
            return False
        # Wait for the worker to start accepting connections.
        deadline = time.perf_counter() + 10.0
        while time.perf_counter() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", WORKER_PORT), timeout=0.1):
                    return True
            except OSError:
                if self.proc.poll() is not None:
                    return False
                time.sleep(0.05)
        return False

    def stop(self) -> None:
        if self.proc is not None:
            try:
                self.proc.terminate()
                self.proc.wait(timeout=2.0)
            except Exception:
                try:
                    self.proc.kill()
                except Exception:
                    pass
            self.proc = None


def _jit_cache_path(source: str) -> Path:
    hasher = hashlib.sha256()
    hasher.update(source.encode("utf-8"))
    key = hasher.hexdigest()
    return Path(tempfile.gettempdir()) / "period_jit_cache" / key


def _clear_period_output_cache(source: str) -> None:
    """Remove the per-source output cache so timed runs execute real code."""
    try:
        _jit_cache_path(source).unlink(missing_ok=True)
    except Exception:
        pass


def _run_via_period_server(path: Path) -> float:
    """Send a single file to the Period worker and return elapsed milliseconds."""
    path_str = str(path.resolve())
    request = struct.pack("<Q", len(path_str)) + path_str.encode("utf-8")
    with socket.create_connection(("127.0.0.1", WORKER_PORT), timeout=10.0) as sock:
        start = time.perf_counter()
        sock.sendall(request)
        header = b""
        while len(header) < 12:
            chunk = sock.recv(12 - len(header))
            if not chunk:
                raise RuntimeError("worker closed connection before header")
            header += chunk
        code, out_len = struct.unpack("<iQ", header)
        output = b""
        while len(output) < out_len:
            chunk = sock.recv(out_len - len(output))
            if not chunk:
                raise RuntimeError("worker closed connection before output")
            output += chunk
        elapsed = time.perf_counter() - start
        if code != 0:
            raise RuntimeError(f"worker error: {output.decode('utf-8', errors='replace')}")
        return elapsed * 1000


def run(cmd: list[str], source: str, ext: str, n: int, runs: int = 10, *, server: PeriodServer | None = None) -> float | None:
    with tempfile.NamedTemporaryFile(mode="w", suffix=ext, delete=False) as f:
        f.write(source)
        src = Path(f.name)

    compiled_exts = {".c", ".rs", ".go"}
    exe = src.with_suffix(".exe")

    if ext == ".period" and server is not None:
        # Reuse the long-running Period worker; clear the output cache once so
        # the timed runs measure real execution rather than cached text.
        _clear_period_output_cache(source)
        try:
            # Warm-up run.
            _run_via_period_server(src)
            times = []
            for _ in range(runs):
                times.append(_run_via_period_server(src))
            return sum(times) / len(times)
        except Exception as e:
            print(f"Period server run failed: {e}")
            return None
        finally:
            src.unlink(missing_ok=True)

    if ext == ".c":
        run_cmd = None
        last_error = ""
        for compile_cmd in c_compiler_candidates(src, exe):
            result = subprocess.run(
                compile_cmd,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True, errors="replace",
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
            text=True, errors="replace",
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
            text=True, errors="replace",
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
            text=True, errors="replace",
        )
        if result.returncode != 0:
            print(f"compile failed: {result.stderr}")
            return None
        run_cmd = ["java", "-cp", str(src.parent), "Main"]
    elif ext == ".cs":
        # Requires the .NET SDK (dotnet new / build).
        proj_dir = src.parent / src.stem
        proj_dir.mkdir(exist_ok=True)
        result = subprocess.run(
            ["dotnet", "new", "console", "--force", "-o", str(proj_dir)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True, errors="replace",
        )
        if result.returncode != 0:
            print(f"dotnet new failed: {result.stderr}")
            return None
        (proj_dir / "Program.cs").write_text(source)
        result = subprocess.run(
            ["dotnet", "build", "-c", "Release", str(proj_dir)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True, errors="replace",
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


def render_svg(results: list[tuple[str, list[tuple[str, float]]]], path: Path) -> None:
    """Render a grouped-bar SVG chart of the benchmark results.

    The y-axis uses a log scale so that both sub-millisecond compiled runs
    and multi-second interpreted runs remain visible on the same chart.
    """
    WIDTH = 980
    HEIGHT = 500
    MARGIN = {"top": 45, "right": 110, "bottom": 60, "left": 75}
    # Fixed order so Period is first and matches the homepage JS chart.
    ORDER = ["Period", "C (Release)", "Rust", "Go", "C#", "Java"]
    COLORS = {
        "Period": "#ff9800",
        "C (Release)": "#4caf50",
        "Rust": "#2196f3",
        "Go": "#00bcd4",
        "C#": "#9c27b0",
        "Java": "#f44336",
    }

    plot_w = WIDTH - MARGIN["left"] - MARGIN["right"]
    plot_h = HEIGHT - MARGIN["top"] - MARGIN["bottom"]

    all_times = [ms for _, data in results for _, ms in data]
    if not all_times:
        return

    min_ms = max(0.001, min(all_times) * 0.8)
    max_ms = max(all_times) * 1.2
    log_min = math.log10(min_ms)
    log_max = math.log10(max_ms)

    def y_pos(ms: float) -> float:
        ratio = (math.log10(max(min_ms, ms)) - log_min) / (log_max - log_min)
        return MARGIN["top"] + plot_h - ratio * plot_h

    def esc(text: str) -> str:
        return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")

    lines: list[str] = []
    lines.append(f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">')
    lines.append('<style>')
    lines.append('  text { font-family: sans-serif; font-size: 12px; fill: #333; }')
    lines.append('  .title { font-size: 18px; font-weight: bold; }')
    lines.append('  .axis { font-size: 12px; }')
    lines.append('  .label { font-size: 11px; }')
    lines.append('  .grid { stroke: #e0e0e0; stroke-width: 1; }')
    lines.append('</style>')

    # Background
    lines.append(f'<rect width="{WIDTH}" height="{HEIGHT}" fill="#ffffff"/>')

    # Title
    lines.append(f'<text x="{WIDTH // 2}" y="{MARGIN["top"] - 18}" text-anchor="middle" class="title">Period Benchmark Results (log scale)</text>')

    # Grid lines and y-axis labels
    tick = 10 ** math.floor(log_min)
    while tick <= max_ms * 1.001:
        y = y_pos(tick)
        if MARGIN["top"] - 1 <= y <= MARGIN["top"] + plot_h + 1:
            lines.append(f'<line x1="{MARGIN["left"]}" y1="{y}" x2="{MARGIN["left"] + plot_w}" y2="{y}" class="grid"/>')
            lines.append(f'<text x="{MARGIN["left"] - 8}" y="{y + 4}" text-anchor="end" class="axis">{tick:g} ms</text>')
        tick *= 10

    # Axes
    lines.append(f'<line x1="{MARGIN["left"]}" y1="{MARGIN["top"]}" x2="{MARGIN["left"]}" y2="{MARGIN["top"] + plot_h}" stroke="#333" stroke-width="2"/>')
    lines.append(f'<line x1="{MARGIN["left"]}" y1="{MARGIN["top"] + plot_h}" x2="{MARGIN["left"] + plot_w}" y2="{MARGIN["top"] + plot_h}" stroke="#333" stroke-width="2"/>')

    # Bars
    group_w = plot_w / len(results)
    max_bars = max(len(data) for _, data in results) if results else 1
    bar_w = group_w / (max_bars + 1)

    for i, (workload, data) in enumerate(results):
        group_x = MARGIN["left"] + i * group_w
        # Sort each group's data to the fixed language order.
        data_map = dict(data)
        ordered = [(name, data_map[name]) for name in ORDER if name in data_map]
        for j, (name, ms) in enumerate(ordered):
            x = group_x + (j + 0.5) * bar_w
            y = y_pos(ms)
            h = MARGIN["top"] + plot_h - y
            color = COLORS.get(name, "#999")
            lines.append(f'<rect x="{x}" y="{y}" width="{bar_w * 0.85}" height="{h}" fill="{color}" rx="2"/>')
            # Value label above bar
            label_y = y - 5 if y > MARGIN["top"] + 15 else y + 15
            lines.append(f'<text x="{x + bar_w * 0.425}" y="{label_y}" text-anchor="middle" class="label">{ms:.1f}</text>')

    # X-axis labels
    for i, (workload, _) in enumerate(results):
        x = MARGIN["left"] + i * group_w + group_w / 2
        y = MARGIN["top"] + plot_h + 20
        lines.append(f'<text x="{x}" y="{y}" text-anchor="middle" transform="rotate(-30 {x} {y})" class="axis">{esc(workload)}</text>')

    # Legend
    legend_x = MARGIN["left"] + plot_w + 10
    legend_y = MARGIN["top"] + 52
    lines.append('<text x="' + str(legend_x) + '" y="' + str(legend_y - 32) + '" class="axis" font-weight="bold">Language</text>')
    for idx, name in enumerate(ORDER):
        y = legend_y + idx * 22
        color = COLORS[name]
        lines.append(f'<rect x="{legend_x}" y="{y - 10}" width="14" height="14" fill="{color}" rx="2"/>')
        lines.append(f'<text x="{legend_x + 22}" y="{y + 2}" class="axis">{esc(name)}</text>')

    lines.append('</svg>')
    path.write_text("\n".join(lines), encoding="utf-8")


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

    server = PeriodServer()
    use_server = server.start()
    if not use_server:
        print("Warning: could not start Period server; falling back to subprocess runs")

    results: list[tuple[str, list[tuple[str, float]]]] = []

    try:
        for workload, n in WORKLOADS.items():
            print(f"\nWorkload: {workload} (n={n:,})")
            print(f"{'Language':<12}", end="")
            print(f"{'time (ms)':>15}")
            print("-" * 28)

            workload_results: list[tuple[str, float]] = []
            for name, cmd, ext in languages:
                first = cmd[0]
                if shutil.which(first) is None and not Path(first).exists():
                    continue
                src = source_for(lang_key[name], workload, n)
                if src is None:
                    continue
                ms = run(cmd, src, ext, n, server=server if use_server else None)
                if ms is None:
                    print(f"{name:<12}{'failed':>15}")
                else:
                    print(f"{name:<12}{ms:>14.1f}ms")
                    workload_results.append((name, ms))
            results.append((workload, workload_results))
    finally:
        server.stop()

    chart_path = ROOT / "docs" / "benchmark_long.svg"
    render_svg(results, chart_path)
    print(f"\nChart saved to {chart_path}")


if __name__ == "__main__":
    main()
