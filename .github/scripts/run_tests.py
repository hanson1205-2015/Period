#!/usr/bin/env python3
"""Integration tests for the Period compiler/runtime.

Run with: python .github/scripts/run_tests.py
Expects the Period debug binary to exist at period/target/debug/period
(or period/target/debug/period.exe on Windows).
"""

import json
import os
import re
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
            output_lines = [line for line in result.stdout.splitlines() if line]
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
                    import ./helper.
                    show add with 2, 3.
                    show sub with 5, 2.
                """).strip() + "\n")
            result = run_period([main])
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(
                [line for line in result.stdout.splitlines() if line],
                ["5", "3"],
            )

    def test_missing_local_module_has_span(self):
        with tempfile.TemporaryDirectory() as tmp:
            main = os.path.join(tmp, "main.period")
            with open(main, "w", encoding="utf-8") as f:
                f.write("import ./foo.\nshow 1.\n")
            result = run_period([main])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("module not found './foo'", result.stdout)
            self.assertIn("main.period:1:", result.stdout)

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
                    import ./helper.
                    show hidden with 1.
                """).strip() + "\n")
            result = run_period([main])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("undefined function 'hidden'", result.stdout)

    def test_circular_import_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            a = os.path.join(tmp, "a.period")
            b = os.path.join(tmp, "b.period")
            main = os.path.join(tmp, "main.period")
            with open(a, "w", encoding="utf-8") as f:
                f.write("import ./b.\nexport f.\n")
            with open(b, "w", encoding="utf-8") as f:
                f.write("import ./a.\nexport g.\n")
            with open(main, "w", encoding="utf-8") as f:
                f.write("import ./a.\nshow 1.\n")
            result = run_period([main])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Circular import detected", result.stdout)

    def test_self_import_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "selfmod.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write("import ./selfmod.\n")
            result = run_period([path])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Circular import detected", result.stdout)


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

    def test_non_ascii_identifier_does_not_panic(self):
        # Regression: multi-byte (e.g. Chinese) identifiers used to panic the
        # lexer with a byte-index-out-of-bounds slice.
        run_file(
            """
            let 变量 be 42.
            show 变量.
            """,
            expected_lines=["42"],
        )


class TestLanguageFeatures(unittest.TestCase):
    def test_string_interpolation(self):
        run_file(
            """
            let name be "World".
            show "Hello, {name}!".
            """,
            expected_lines=["Hello, World!"],
        )

    def test_single_quoted_strings(self):
        run_file(
            """
            let name be 'World'.
            show 'Hello, {name}!'.
            show 'say "hi"'.
            show 'it\\'s'.
            """,
            expected_lines=["Hello, World!", 'say "hi"', "it's"],
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

    def test_stdlib_source_module_list(self):
        run_file(
            """
            import list.
            show sum with [1, 2, 3].
            """,
            expected_lines=["6"],
        )

    def test_stdlib_source_module_text(self):
        run_file(
            """
            import text.
            show join with ["a", "b"], "-".
            """,
            expected_lines=["a-b"],
        )

    def test_stdlib_function_argument_type_mismatch_is_caught_statically(self):
        out = run_file(
            """
            import text.
            show join with "-", ["a", "b"].
            """,
            should_fail=True,
        )
        self.assertIn("argument 1 type mismatch", out)

    def test_stdlib_runtime_error_points_at_call_site(self):
        out = run_file(
            """
            import list.
            show max with [].
            """,
            should_fail=True,
        )
        self.assertIn("Index out of range (list is empty)", out)

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
                    set the name of this to name.
                    set the age of this to age.
                define greet with greeting:
                    return greeting + ", " + the name of this + "!".
            let ada be new Person with "Ada", 37.
            show tell ada to greet with "Hi".
            show the age of ada.
            """,
            expected_lines=["Hi, Ada!", "37"],
        )

    def test_class_field_initialized_in_init_body(self):
        run_file(
            """
            class A:
                init:
                    set the x of this to 0.
            let a be new A.
            show the x of a.
            """,
            expected_lines=["0"],
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

    def test_integer_literals_and_string_repetition(self):
        run_file(
            """
            show type with 5.
            show type with 5.0.
            show "ha" * 3.
            """,
            expected_lines=["integer", "number", "hahaha"],
        )

    def test_mixed_integer_number_arithmetic(self):
        run_file(
            """
            show 1 + 2.5.
            show 2.5 + 1.
            show 2 * 3.5.
            show 3.5 * 2.
            """,
            expected_lines=["3.5", "3.5", "7", "7"],
        )

    def test_boolean_operators_require_booleans(self):
        out = run_file(
            """
            show true and 1.
            """,
            should_fail=True,
        )
        self.assertIn("'and' requires booleans", out)

        out = run_file(
            """
            show 1 and true.
            """,
            should_fail=True,
        )
        self.assertIn("'and' requires booleans", out)

        out = run_file(
            """
            show not 1.
            """,
            should_fail=True,
        )
        self.assertIn("'not' requires a boolean", out)

    def test_arbitrary_precision_integer(self):
        run_file(
            """
            let x be 1000000000000000000000000000000.
            show x + 1.
            """,
            expected_lines=["1000000000000000000000000000001"],
        )

    def test_zero_to_negative_power_is_division_by_zero(self):
        out = run_file(
            """
            show 0 ** -1.
            """,
            should_fail=True,
        )
        self.assertIn("Division by zero", out)

    def test_string_brace_escape(self):
        run_file(
            """
            show "\\{not interpolated\\}.".
            """,
            expected_lines=["{not interpolated}."],
        )

    def test_large_integer_number_comparison(self):
        run_file(
            """
            let a be 9223372036854775807.
            let b be 9223372036854775808.
            show a == b.
            show a < b.
            show a > b.
            show 9007199254740993 == 9007199254740992.0.
            show 9007199254740992 == 9007199254740992.0.
            """,
            expected_lines=["false", "true", "false", "false", "true"],
        )

    def test_float_dict_keys(self):
        run_file(
            """
            let d be {1.5: "a", 2.0: "b"}.
            show d[1.5].
            show d[2.0].
            """,
            expected_lines=["a", "b"],
        )

    def test_nested_function(self):
        run_file(
            """
            define outer:
                define inner:
                    show "inside inner".
                inner.
            outer.
            """,
            expected_lines=["inside inner"],
        )

    def test_recursive_function(self):
        run_file(
            """
            define factorial with number n returns number:
                if n <= 1 then:
                    return 1.
                return n * factorial with (n - 1).
            show factorial with 5.
            """,
            expected_lines=["120"],
        )

    def test_prime_example_outputs_primes(self):
        # Regression test: integer modulo and equality must agree for the
        # official prime-number example to produce primes rather than all numbers.
        import os
        root = os.path.normpath(os.path.join(os.path.dirname(__file__), "../.."))
        path = os.path.join(root, "examples", "primes.period")
        result = run_period([path])
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(
            [line for line in result.stdout.splitlines() if line],
            ["2", "3", "5", "7", "11", "13", "17", "19", "23", "29",
             "31", "37", "41", "43", "47", "53", "59", "61", "67", "71"],
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

    def test_numeric_loop(self):
        # Pure numeric loops run through the tree-walking interpreter.
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

    def test_parse_error_reports_source_location(self):
        # Regression test for issue #5: parse errors must be reported with a
        # source line, column and caret, not as a Rust panic message.
        out = run_file(
            """
            let x be 5
            """,
            should_fail=True,
        )
        self.assertIn("expected '.' at end of let", out)
        # Column is 11 on Unix and 12 on Windows, where the temp file is
        # written with \r\n line endings.
        self.assertRegex(out, r":1:1[12]: error")
        self.assertIn("^", out)

    def test_compact_show_returns_zero(self):
        # Regression test for issue #6: compact call syntax must not produce
        # a non-zero exit code.
        run_file(
            """
            show("Hello,World").
            """,
            expected_lines=["Hello,World"],
        )

    def test_function_call_argument_contains_binary_operator(self):
        # "f with a + b" is parsed as "f(a + b)".
        run_file(
            """
            define double with number x returns number:
                return x * 2.
            show double with 3 + 4.
            show 10 - double with 3.
            """,
            expected_lines=["14", "4"],
        )

    def test_multi_argument_call_uses_full_expressions(self):
        # Each argument to 'with' is parsed as a full expression.
        run_file(
            """
            define add with number a, number b returns number:
                return a + b.
            show add with 1 + 2, 3 + 4.
            show add with (1 + 2), (3 + 4).
            """,
            expected_lines=["10", "10"],
        )

    def test_multi_argument_call_without_parentheses_works(self):
        # "add with 1 + 2, 3" parses as add(1 + 2, 3).
        run_file(
            """
            define add with number a, number b returns number:
                return a + b.
            show add with 1 + 2, 3.
            """,
            expected_lines=["6"],
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

    def test_property_assignment_type_mismatch_caught(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "prop_type.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    class A:
                        init with number x:
                            set the x of this to x.
                    let a be new A with 5.
                    set the x of a to "hello".
                """).strip() + "\n")
            result = run_period([path])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("assignment type mismatch", result.stdout)

    def test_missing_return_caught(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "missing_return.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    define f returns number:
                        show 1.
                """).strip() + "\n")
            result = run_period([path])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("may not return a value on all paths", result.stdout)

    def test_method_accessed_as_property_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "method_prop.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    class Person:
                        init with string name:
                            set the name of this to name.
                        define greet returns string:
                            return "Hi, " + the name of this.
                    let p be new Person with "Ada".
                    show the greet of p.
                """).strip() + "\n")
            result = run_period([path])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("method 'greet' must be called with 'tell", result.stdout)

    def test_duplicate_definition_warning(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "dup.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    let x be 1.
                    let x be 2.
                    show x.
                """).strip() + "\n")
            result = run_period([path])
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertIn("warning:", result.stdout)
            self.assertIn("redefinition of 'x'", result.stdout)

    def test_duplicate_function_warning_reported_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "dup.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    define f with n:
                        return n.

                    define f with n:
                        return n.
                """).strip() + "\n")
            result = run_period([path])
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(result.stdout.count("warning:"), 1)
            self.assertIn("redefinition of 'f'", result.stdout)

    def test_duplicate_class_warning_has_source_location(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "dup.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    class C:
                        init with x:
                            return nothing.

                    class C:
                        init with x:
                            return nothing.
                """).strip() + "\n")
            result = run_period([path])
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertIn("warning:", result.stdout)
            self.assertIn("redefinition of 'C'", result.stdout)
            self.assertNotIn(":0:0:", result.stdout)

    def test_duplicate_import_warning_is_per_module(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "dup.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    import math.
                    import math.
                    show sin with 0.
                """).strip() + "\n")
            result = run_period([path])
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(result.stdout.count("warning:"), 1)
            self.assertIn("duplicate import of 'math'", result.stdout)

    def test_zero_arity_builtin_used_as_value_is_typed_as_return_type(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "input.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    let name be input.
                    show "Hello, " + name + "!".
                """).strip() + "\n")
            result = run_period([path], input_text="Ada\n")
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertIn("Hello, Ada!", result.stdout)

    def test_zero_arity_imported_function_used_as_value_auto_calls(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "random.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    import random.
                    let n be random.
                    show n.
                """).strip() + "\n")
            result = run_period([path])
            self.assertEqual(result.returncode, 0, result.stdout)
            # Should print a number, not a function representation.
            line = result.stdout.strip().splitlines()[-1]
            self.assertTrue(line.replace('.', '', 1).isdigit(), f"expected numeric output, got {line!r}")

    def test_zero_arity_function_called_with_args_is_type_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "random.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    import random.
                    let n be random with nothing.
                    show n.
                """).strip() + "\n")
            result = run_period([path])
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertIn("cannot call 'number'", result.stdout)

    def test_mixed_integer_number_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "cmp.period")
            with open(path, "w", encoding="utf-8") as f:
                f.write(textwrap.dedent("""
                    show (5.5 > 0).
                    show (0 > 5.5).
                    show (5.0 == 5).
                    show (5 == 5.0).
                    show (2.5 > 2).
                    show (2 > 2.5).
                """).strip() + "\n")
            result = run_period([path])
            self.assertEqual(result.returncode, 0, result.stdout)
            lines = [line for line in result.stdout.splitlines() if line]
            self.assertEqual(lines, ["true", "false", "true", "true", "true", "false"])


class TestLSP(unittest.TestCase):
    def _lsp_message(self, obj):
        body = json.dumps(obj)
        return f"Content-Length: {len(body)}\r\n\r\n{body}".encode("utf-8")

    def _lsp_read_message(self, proc):
        """Read one LSP message from proc.stdout; returns parsed JSON or None."""
        header = b""
        while b"\r\n\r\n" not in header:
            chunk = proc.stdout.read(1)
            if not chunk:
                return None
            header += chunk
        length = 0
        for line in header.decode("utf-8").split("\r\n"):
            if line.lower().startswith("content-length:"):
                length = int(line.split(":", 1)[1].strip())
                break
        if length <= 0:
            return None
        return json.loads(proc.stdout.read(length).decode("utf-8"))

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

    def test_lsp_no_completion_in_comment(self):
        # Regression: typing inside a '--' comment must not offer completions.
        proc = subprocess.Popen(
            [PERIOD, "--lsp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            uri = "file:///comment_test.period"
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"processId": None, "rootUri": None, "capabilities": {}},
            }))
            proc.stdin.flush()
            self.assertEqual(self._lsp_read_message(proc)["id"], 1)

            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {},
            }))
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": "period",
                        "version": 1,
                        "text": "-- a comment with let show\nshow 1. -- inline comment\n",
                    }
                },
            }))
            proc.stdin.flush()

            # Completion inside the comment line (0-based line 0).
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": {"uri": uri},
                    "position": {"line": 0, "character": 5},
                },
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 2:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No completion response received")
            self.assertEqual(msg["result"], [], f"Expected no completions in comment, got {msg['result']}")

            # Completion inside an inline comment after code (line 1).
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": {"uri": uri},
                    "position": {"line": 1, "character": 20},
                },
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 4:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No completion response received")
            self.assertEqual(msg["result"], [], f"Expected no completions in inline comment, got {msg['result']}")

            # Sanity: completion on the code part of the same line is non-empty.
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": {"uri": uri},
                    "position": {"line": 1, "character": 2},
                },
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 3:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No completion response received")
            self.assertTrue(len(msg["result"]) > 0, "Expected completions on a normal line")
        finally:
            proc.stdin.close()
            proc.stdout.close()
            proc.stderr.close()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()

    def test_lsp_semantic_tokens_functions_and_classes(self):
        # User-defined functions/classes/methods must be reported as semantic
        # tokens so the editor can highlight zero-argument calls like `show a.`.
        proc = subprocess.Popen(
            [PERIOD, "--lsp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            uri = "file:///sem_tokens_test.period"
            source = (
                "define a:\n"
                "    show \"hi\".\n"
                "\n"
                "class Point:\n"
                "    define move:\n"
                "        show \"moving\".\n"
                "\n"
                "show a.\n"
                "let p be new Point.\n"
                "show p.move.\n"
            )
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"processId": None, "rootUri": None, "capabilities": {}},
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 1:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No initialize response received")
            legend = msg["result"]["capabilities"]["semanticTokensProvider"]["legend"]["tokenTypes"]
            self.assertEqual(legend, ["function", "type", "method"])

            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {},
            }))
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": "period",
                        "version": 1,
                        "text": source,
                    }
                },
            }))
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/semanticTokens/full",
                "params": {"textDocument": {"uri": uri}},
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 2:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No semanticTokens response received")

            data = msg["result"]["data"]
            self.assertTrue(len(data) > 0 and len(data) % 5 == 0, f"Bad token data: {data}")
            lines = source.splitlines()
            line, col = 0, 0
            found = []
            for i in range(0, len(data), 5):
                dl, ds, length, ttype, _ = data[i:i + 5]
                line += dl
                col = col + ds if dl == 0 else ds
                found.append((lines[line][col:col + length], legend[ttype]))
            expected = [
                ("a", "function"), ("Point", "type"), ("move", "method"),
                ("a", "function"), ("Point", "type"), ("move", "method"),
            ]
            self.assertEqual(found, expected, f"Unexpected semantic tokens: {found}")
        finally:
            proc.stdin.close()
            proc.stdout.close()
            proc.stderr.close()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()

    def test_lsp_semantic_tokens_with_syntax_errors(self):
        # A document that fails to parse must still get semantic highlighting
        # via the token-stream fallback, including classes without a body.
        proc = subprocess.Popen(
            [PERIOD, "--lsp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            uri = "file:///sem_tokens_broken_test.period"
            source = (
                "define a:\n"
                "    show \"hi\".\n"
                "\n"
                "class Point:\n"
                "    define move:\n"
                "        show \"moving\".\n"
                "\n"
                "show a.\n"
                "let p be\n"
                "let q be new Point.\n"
                "show p.move.\n"
                "class Empty:\n"
                "define b:\n"
                "    show a.\n"
            )
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"processId": None, "rootUri": None, "capabilities": {}},
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 1:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No initialize response received")
            legend = msg["result"]["capabilities"]["semanticTokensProvider"]["legend"]["tokenTypes"]

            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {},
            }))
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": "period",
                        "version": 1,
                        "text": source,
                    }
                },
            }))
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/semanticTokens/full",
                "params": {"textDocument": {"uri": uri}},
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 2:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No semanticTokens response received")

            data = msg["result"]["data"]
            self.assertTrue(len(data) > 0 and len(data) % 5 == 0, f"Bad token data: {data}")
            lines = source.splitlines()
            line, col = 0, 0
            found = []
            for i in range(0, len(data), 5):
                dl, ds, length, ttype, _ = data[i:i + 5]
                line += dl
                col = col + ds if dl == 0 else ds
                found.append((lines[line][col:col + length], legend[ttype]))
            expected = [
                ("a", "function"), ("Point", "type"), ("move", "method"),
                ("a", "function"), ("Point", "type"), ("move", "method"),
                ("Empty", "type"), ("b", "function"), ("a", "function"),
            ]
            self.assertEqual(found, expected, f"Unexpected semantic tokens: {found}")
        finally:
            proc.stdin.close()
            proc.stdout.close()
            proc.stderr.close()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()

    def test_lsp_hover_infers_undeclared_return_type(self):
        # `define a: return 2.` has no `returns` annotation; hover must infer
        # the return type from the return statements.
        proc = subprocess.Popen(
            [PERIOD, "--lsp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            uri = "file:///hover_infer_test.period"
            source = (
                "define a:\n"
                "    return 2.\n"
                "\n"
                "define b:\n"
                "    if true, then:\n"
                "        return 1.\n"
                "    otherwise:\n"
                "        return 2.\n"
                "\n"
                "define mixed:\n"
                "    if true, then:\n"
                "        return 1.\n"
                "    otherwise:\n"
                "        return \"x\".\n"
                "\n"
                "show a.\n"
                "show b.\n"
                "show mixed.\n"
            )
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"processId": None, "rootUri": None, "capabilities": {}},
            }))
            proc.stdin.flush()
            msg = self._lsp_read_message(proc)
            while msg is not None and msg.get("id") != 1:
                msg = self._lsp_read_message(proc)
            self.assertIsNotNone(msg, "No initialize response received")

            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {},
            }))
            proc.stdin.write(self._lsp_message({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": "period",
                        "version": 1,
                        "text": source,
                    }
                },
            }))
            proc.stdin.flush()

            lines = source.splitlines()

            def hover_at(line_no, name):
                proc.stdin.write(self._lsp_message({
                    "jsonrpc": "2.0",
                    "id": 100 + line_no,
                    "method": "textDocument/hover",
                    "params": {
                        "textDocument": {"uri": uri},
                        "position": {"line": line_no, "character": lines[line_no].find(name)},
                    },
                }))
                proc.stdin.flush()
                msg = self._lsp_read_message(proc)
                while msg is not None and msg.get("id") != 100 + line_no:
                    msg = self._lsp_read_message(proc)
                self.assertIsNotNone(msg, f"No hover response for {name!r}")
                result = msg.get("result")
                return result["contents"]["value"] if result else None

            self.assertIn("define a -> integer", hover_at(15, "a"))
            self.assertIn("define b -> integer", hover_at(16, "b"))
            # Conflicting return types give up and fall back to nothing.
            self.assertIn("define mixed -> nothing", hover_at(17, "mixed"))
        finally:
            proc.stdin.close()
            proc.stdout.close()
            proc.stderr.close()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()


class TestStandardLibrary(unittest.TestCase):
    def test_string_module_functions(self):
        out = run_file("""
            import string.
            show upper with "hello".
            show trim with "  world  ".
            show contains with "hello", "ell".
            show starts_with with "hello", "he".
            show ends_with with "hello", "lo".
            show replace with "hello world", "world", "period".
            show slice with "hello", 1.
            show substring with "hello", 1, 4.
            show split with "a,b,c", ",".
        """, expected_lines=[
            "HELLO",
            "world",
            "true",
            "true",
            "true",
            "hello period",
            "ello",
            "ell",
            "[a, b, c]",
        ])

    def test_list_module_higher_order_functions(self):
        out = run_file("""
            import list.
            define double with x:
                return x * 2.
            define is_even with x:
                return x % 2 == 0.
            let xs be [1, 2, 3, 4].
            show map with xs, double.
            show filter with xs, is_even.
            show reverse with xs.
            show sort with [3, 1, 4, 1, 5].
            show contains with xs, 3.
        """, expected_lines=[
            "[2, 4, 6, 8]",
            "[2, 4]",
            "[4, 3, 2, 1]",
            "[1, 1, 3, 4, 5]",
            "true",
        ])

    def test_path_module_functions(self):
        out = run_file("""
            import path.
            show join with "a", "b".
            show join with "a/", "/b".
            show basename with "/usr/bin/period".
            show dirname with "/usr/bin/period".
            show extension with "file.period".
            show is_absolute with "/usr/bin".
        """, expected_lines=[
            "a/b",
            "a/b",
            "period",
            "/usr/bin",
            "period",
            "true",
        ])

    def test_test_module_asserts(self):
        out = run_file("""
            import test.
            assert with 1 + 1 == 2.
            assert_equal with 4, 2 + 2.
            define boom with _:
                error with "boom".
            assert_raises with boom.
            show "ok".
        """, expected_lines=["ok"])


class TestCompactSyntax(unittest.TestCase):
    def test_dot_property_access_and_parenthesized_calls(self):
        out = run_file("""
            import math.
            class Person:
                init with name, age:
                    set this.name to name.
                    set this.age to age.
                define greet with greeting:
                    show greeting + ", " + this.name + "!".
            let p be new Person("Ada", 42).
            p.greet("Hello").
            show p.age.
            show sqrt(16).
        """, expected_lines=[
            "Hello, Ada!",
            "42",
            "4",
        ])


if __name__ == "__main__":
    unittest.main(verbosity=2)
