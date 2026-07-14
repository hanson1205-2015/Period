#!/usr/bin/env bash
set -euo pipefail

# Execution-path differential test.
#
# Period previously had three execution backends (Cranelift JIT, bytecode VM,
# and tree-walk interpreter) that this script compared. The runtime has since
# been unified to a single bytecode VM path, so the script now runs the
# examples once and confirms they still pass.

cd "$(dirname "$0")/../.."

# Make sure the debug binary is available.
cd period
cargo build 2>&1
cd ..

.github/scripts/run_examples.sh
