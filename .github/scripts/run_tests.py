#!/usr/bin/env python3
"""Integration tests for the Period compiler/runtime.

Run with: python .github/scripts/run_tests.py
Expects the Period debug binary to exist at period/target/debug/period
(or period/target/debug/period.exe on Windows).
"""

import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest


def find_period_binary():
    """Locate the Period debug binary."""
    root = os.path.normpath(os.path.join(os.path.dirname(__file__), "../.."))
    candidates = [
        os.path.join(root, "period", "target", "debug", "period.exe"),
        os.path.join(root, "period", "target", "debug", "period"),
    ]
    for path in candidates:
        if os.path.isfile(path):
            return path
    raise FileNotFoundError("Period debug binary not found. Run 'cargo build' in period/.")


PERIOD = find_period_binary()


def run_period(args, input_text=None, cwd=None):
    """Run period with the given args and optional stdin."""
    result = subprocess.run(
        [PERIOD] + args,
        input=input_text,
        text=True,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    return result


def run_file(source, expected_lines=None, should_fail=False):
    """Write source to a temp file, run it, and assert the outcome."""
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "test.period")
        with open(path, "w", encoding="utf-8") as f:
            f.write(textwrap.dedent(source).strip() + "\n")
        result = run_period([path])
        if should_fail:
            if result.returncode == 0:
                raise AssertionError(
                    f"Expected failure but succeeded.\nOutput:\n{result.stdout}"
                )
            return result.stdout
        if result.returncode != 0:
            raise AssertionError(
                f"Expected success but got exit {result.returncode}.\nOutput:\n{result.stdout}"
            )
        if expected_lines is not None:
            # Filter out JIT fallback messages; the interpreter still produces correct output.
            ignored = {
                "compilation failed; falling back to interpreter.",
                "no C compiler available; falling back to interpreter.",
            }
            output_lines = [
                line for line in result.stdout.splitlines()
                if line and line not in ignored
            ]
            if output_lines != expected_lines:
                raise AssertionError(
                    f"Output mismatch.\nExpected: {expected_lines}\nGot: {output_lines}\nRaw:\n{result.stdout}"
                )
        return result.stdout


class TestLocalModules(unittest.TestCase):
    def test_local_module_import_with_exports(self):
        with tempfile.TemporaryDirectory() as tmp:
            helper = os.path.join(tmp, "helper.period")
            main = os.path.join(tmp, "main.period")
            with open(helper, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    export add, sub.
                    define add with a, b:
                        return a + b.
                    define sub with a, b:
                        return a - b.
                    define hidden with x:
                        return "hidden".
                """).strip() + "\n")
            with open(main, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    import .helper.
                    show add with 2, 3.
                    show sub with 5, 2.
                """).strip() + "\n")
            result = run_period([main])
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(
                [line for line in result.stdout.splitlines() if line],
                ["5", "3"],
            )

    def test_local_module_hidden_name_not_imported(self):
        with tempfile.TemporaryDirectory() as tmp:
            helper = os.path.join(tmp, "helper.period")
            main = os.path.join(tmp, "main.period")
            with open(helper, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    export add.
                    define add with a, b:
                        return a + b.
                    define hidden with x:
                        return "hidden".
                """).strip() + "\n")
            with open(main, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    import .helper.
                    show hidden with 1.
                """).strip() + "\n")
            result = run_period([main])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("undefined function 'hidden'", result.stdout)


class TestLexerErrors(unittest.TestCase):
    def test_file_with_double_dot_reports_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "dots.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write("..\n")
            result = run_period([path])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unexpected '..'", result.stdout)

    def test_repl_double_dot_does_not_crash(self):
        result = run_period([], input_text="..\nexit.\n")
        self.assertEqual(result.returncode, 0)
        self.assertIn("unexpected '..'", result.stdout)


class TestLanguageFeatures(unittest.TestCase):
    def test_string_interpolation(self):
        run_file(
            """
            let name be "World".
            show "Hello, {name}!".
            """,
            expected_lines=["Hello, World!"],
        )

    def test_try_catch(self):
        run_file(
            """
            try:
                let x be 10.
                show x.
            catch err:
                show err.
            """,
            expected_lines=["10"],
        )

    def test_qualified_builtin_access(self):
        run_file(
            """
            import math.
            show sqrt from math with 16.
            """,
            expected_lines=["4"],
        )

    def test_for_loop_with_range(self):
        run_file(
            """
            for i in range with 1, 4 repeat:
                show i.
            """,
            expected_lines=["1", "2", "3"],
        )

    def test_while_loop(self):
        run_file(
            """
            let n be 3.
            while n > 0 repeat:
                show n.
                set n to n - 1.
            """,
            expected_lines=["3", "2", "1"],
        )

    def test_if_otherwise(self):
        run_file(
            """
            let x be 5.
            if x > 3, then:
                show "big".
            otherwise:
                show "small".
            """,
            expected_lines=["big"],
        )

    def test_classes(self):
        run_file(
            """
            class Person:
                init with name, age:
                    set this name to name.
                    set this age to age.
                define greet with greeting:
                    return greeting + ", " + this name + "!".
            let ada be new Person with "Ada", 37.
            show tell ada to greet with "Hi".
            show the age of ada.
            """,
            expected_lines=["Hi, Ada!", "37"],
        )

    def test_list_negative_index(self):
        run_file(
            """
            let items be [10, 20, 30].
            show items[-1].
            """,
            expected_lines=["30"],
        )

    def test_dict(self):
        run_file(
            """
            let d be {"name": "Ada", "age": 37}.
            show d["name"].
            """,
            expected_lines=["Ada"],
        )

    def test_file_io(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = os.path.join(tmp, "data.txt")
            main = os.path.join(tmp, "main.period")
            with open(main, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    write "hello file" to "data.txt".
                    read content from "data.txt".
                    show content.
                """).strip() + "\n")
            result = run_period([main], cwd=tmp)
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertIn("hello file", result.stdout)

    def test_numeric_jit(self):
        # Pure numeric loops are compiled to a cached DLL by the JIT backend.
        run_file(
            """
            let sum be 0.
            let i be 1.
            while i <= 100000 repeat:
                set sum to sum + i.
                set i to i + 1.
            show sum.
            """,
            expected_lines=["5000050000"],
        )


class TestSemanticChecks(unittest.TestCase):
    def test_undefined_variable_caught(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "undefined.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write("show unknown.\n")
            result = run_period([path])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("undefined variable 'unknown'", result.stdout)


class TestLSP(unittest.TestCase):
    def _lsp_message(self, obj):
        body = json.dumps(obj)
        return f"Content-Length: {len(body)}\r\n\r\n{body}".encode("utf-8")

    def test_lsp_initialize(self):
        # Use binary mode pipes; the LSP server relies on exact CRLF framing.
        proc = subprocess.Popen(
            [PERIOD, "--lsp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            init = self._lsp_message({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"processId": None, "rootUri": None, "capabilities": {}},
            })
            proc.stdin.write(init)
            proc.stdin.flush()

            # Read Content-Length header.
            header = b""
            while b"\r\n\r\n" not in header:
                chunk = proc.stdout.read(1)
                if not chunk:
                    break
                header += chunk
            length = 0
            for line in header.decode("utf-8").split("\r\n"):
                if line.lower().startswith("content-length:"):
                    length = int(line.split(":", 1)[1].strip())
                    break
            self.assertGreater(length, 0, "No Content-Length in LSP initialize response")
            response = proc.stdout.read(length).decode("utf-8")
            self.assertIn('"result"', response, f"Initialize response missing result: {response}")
        finally:
            proc.stdin.close()
            proc.stdout.close()
            proc.stderr.close()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()


if __name__ == "__main__":
    unittest.main(verbosity=2)
