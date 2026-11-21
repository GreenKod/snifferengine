#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine Performance Regression Gate (120 Hz Tripwires)
# ==============================================================================
# Runs the criterion benchmark suites from Stage 6.3 in quick mode and fails
# when any measured mean crosses its ceiling. The ceilings are deliberately
# generous order-of-magnitude tripwires (roughly 10x the recorded baselines),
# so shared-runner noise cannot flake the gate while a real regression — a
# layout algorithm rewrite, a batching break, a transform pipeline leak —
# still turns the CI run red.
#
# Usage:
#     bash scripts/perf_gate.sh
#
# The memory-limit side of the gate lives in the stress suite
# (crates/sniffer_plugin/tests/plugin_stress_tests.rs), which the CI runs as
# an explicitly named step.
# ==============================================================================
set -euo pipefail

failures=0

# Runs one benchmark, extracts criterion's mean value, and compares it to
# the ceiling after normalizing the unit to nanoseconds.
check_bench() {
    local crate="$1"
    local bench_file="$2"
    local bench_name="$3"
    local ceiling_ns="$4"

    echo "[perf-gate] $bench_name (ceiling ${ceiling_ns} ns)"

    local raw mean unit mean_ns verdict
    raw=$(cargo bench -p "$crate" --bench "$bench_file" -- "$bench_name" \
        --warm-up-time 0.1 --measurement-time 1 --color never 2>&1)

    # Criterion prints:  time:   [lower mean upper]  with field-width
    # padding that shifts whitespace between the brackets and values, so
    # the mean is extracted positionally-free: the second number on the
    # line, and the first unit.
    local stats_line
    stats_line=$(printf '%s\n' "$raw" | grep "time:" | grep "\[" | head -1)
    mean=$(printf '%s\n' "$stats_line" | grep -oE '[0-9]+(\.[0-9]+)?' | sed -n '2p')
    unit=$(printf '%s\n' "$stats_line" | grep -oE 'µs|ms|ns|s' | head -1)

    if [ -z "$mean" ] || [ -z "$unit" ]; then
        echo "[FAIL] could not parse the criterion mean for '$bench_name'"
        failures=$((failures + 1))
        return
    fi

    mean_ns=$(awk -v mean="$mean" -v unit="$unit" 'BEGIN {
        factor = 1;
        if (unit == "ms") { factor = 1000000 }
        else if (unit == "\xc2\xb5s") { factor = 1000 }
        else if (unit == "s") { factor = 1000000000 }
        printf "%.0f", mean * factor
    }')

    verdict=$(awk -v mean="$mean_ns" -v ceiling="$ceiling_ns" 'BEGIN {
        print (mean <= ceiling) ? "ok" : "fail"
    }')

    if [ "$verdict" = "ok" ]; then
        echo "[OK] $bench_name mean ${mean}${unit} (${mean_ns} ns <= ${ceiling_ns} ns)"
    else
        echo "[FAIL] $bench_name mean ${mean}${unit} (${mean_ns} ns) exceeds the ${ceiling_ns} ns ceiling"
        failures=$((failures + 1))
    fi
}

# --- Layout phase -------------------------------------------------------------
check_bench sniffer_core frame_pipeline "layout/full_tree" 5000000
check_bench sniffer_core frame_pipeline "layout/window_culled_tree" 1500000

# --- Transform phase ----------------------------------------------------------
check_bench sniffer_core frame_pipeline "transform/200_springs_tick_120hz" 500000

# --- Render phase -------------------------------------------------------------
check_bench sniffer_render render_walk "render_walk/full_tree" 100000
check_bench sniffer_render render_walk "render_walk/hit_test_full_tree" 10000

if [ "$failures" -ne 0 ]; then
    echo "perf-gate: $failures benchmark(s) regressed past their ceilings."
    exit 1
fi

echo "perf-gate: all 120 Hz tripwires green."
