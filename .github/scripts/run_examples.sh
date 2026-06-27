#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../../period"

PERIOD="./target/debug/period"
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" || "$OSTYPE" == "cygwin" ]]; then
  PERIOD="./target/debug/period.exe"
fi

EXAMPLES_DIR="../examples"
# These examples intentionally contain errors and must fail.
EXPECTED_FAILURES=(errors.period multi_errors.period)

success_count=0
failure_count=0
unexpected_success=0

run_example() {
  local file="$1"
  local expect_fail="$2"
  local basename
  basename="$(basename "$file")"

  echo "==> Running $basename"
  if $PERIOD "$file" > /dev/null 2>&1; then
    if [[ "$expect_fail" == "yes" ]]; then
      echo "    ERROR: $basename was expected to fail but succeeded"
      ((unexpected_success++)) || true
    else
      echo "    OK"
      ((success_count++)) || true
    fi
  else
    if [[ "$expect_fail" == "yes" ]]; then
      echo "    OK (expected failure)"
      ((success_count++)) || true
    else
      echo "    FAILED"
      ((failure_count++)) || true
    fi
  fi
}

for file in "$EXAMPLES_DIR"/*.period; do
  basename="$(basename "$file")"
  expect_fail="no"
  for fail in "${EXPECTED_FAILURES[@]}"; do
    if [[ "$basename" == "$fail" ]]; then
      expect_fail="yes"
      break
    fi
  done
  run_example "$file" "$expect_fail"
done

echo ""
echo "Examples run: $((success_count + failure_count + unexpected_success))"
echo "Passed:       $success_count"
echo "Failed:       $failure_count"
echo "Unexpected:   $unexpected_success"

if (( failure_count > 0 || unexpected_success > 0 )); then
  exit 1
fi
