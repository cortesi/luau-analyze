#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ROOT}/target/release/lan"
ITERATIONS="${ITERATIONS:-5}"

echo "building release binary for latency benchmark..."
cargo build -p lan --release --manifest-path "${ROOT}/Cargo.toml" >/dev/null

python3 - "${ROOT}" "${BIN}" "${ITERATIONS}" <<'PY'
import os
import statistics
import subprocess
import sys
import time

root = sys.argv[1]
binary = sys.argv[2]
iterations = int(sys.argv[3])

cases = [
    (
        "examples/scripts/01_ok_builder.luau",
        float(os.environ.get("LATENCY_THRESH_SIMPLE_MS", "650")),
    ),
    (
        "examples/scripts/realtime/08_timeout_sensitive_types_ok.luau",
        float(os.environ.get("LATENCY_THRESH_COMPLEX_MS", "900")),
    ),
    (
        "examples/scripts/strict/11_generic_result_ok.luau",
        float(os.environ.get("LATENCY_THRESH_STRICT_MS", "700")),
    ),
]

print(f"running {iterations} iteration(s) per script")

violations = []
for relpath, threshold_ms in cases:
    script_path = os.path.join(root, relpath)
    durations_ms = []
    for _ in range(iterations):
        start = time.perf_counter()
        completed = subprocess.run(
            [binary, "check", "--default-definitions", script_path],
            cwd=root,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        elapsed_ms = (time.perf_counter() - start) * 1000
        durations_ms.append(elapsed_ms)
        if completed.returncode != 0:
            violations.append(
                f"{relpath}: command exited with status {completed.returncode}"
            )
            break

    if not durations_ms:
        continue

    durations_ms.sort()
    p95_index = max(0, int(round(0.95 * (len(durations_ms) - 1))))
    p95_ms = durations_ms[p95_index]
    avg_ms = statistics.fmean(durations_ms)
    print(
        f"{relpath}: avg={avg_ms:.2f}ms p95={p95_ms:.2f}ms threshold={threshold_ms:.2f}ms"
    )

    if p95_ms > threshold_ms:
        violations.append(
            f"{relpath}: p95 {p95_ms:.2f}ms exceeds threshold {threshold_ms:.2f}ms"
        )

if violations:
    print("latency benchmark failed:")
    for violation in violations:
        print(f"  - {violation}")
    sys.exit(1)

print("latency benchmark passed")
PY
