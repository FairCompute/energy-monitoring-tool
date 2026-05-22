#!/usr/bin/env python3
"""
Known CPU workload for energy measurement verification.
Performs matrix multiplication to generate consistent CPU load.
"""

import sys
import time
import os


def deterministic_value(row: int, col: int, iteration: int) -> float:
    """Return a stable pseudo-varying value without using a PRNG."""
    return ((row * 31 + col * 17 + iteration * 13) % 1000) / 1000.0


def cpu_intensive_work(duration_seconds: float = 10.0):
    """Perform CPU-intensive matrix operations for a fixed duration."""
    print(f"PID: {os.getpid()}", flush=True)
    print(f"Starting CPU workload for {duration_seconds} seconds...", flush=True)

    start = time.perf_counter()
    iterations = 0
    checksum = 0.0

    # Matrix size - adjust for desired CPU load
    size = 200

    while time.perf_counter() - start < duration_seconds:
        # Create deterministic matrices so verification is repeatable.
        a = [
            [deterministic_value(i, k, iterations) for k in range(size)]
            for i in range(size)
        ]
        b = [
            [deterministic_value(k, j, iterations + 1) for j in range(size)]
            for k in range(size)
        ]

        # Matrix multiplication (CPU intensive)
        result = [
            [sum(a[i][k] * b[k][j] for k in range(size)) for j in range(size)]
            for i in range(size)
        ]
        checksum += sum(sum(row) for row in result)

        iterations += 1

        # Progress indicator
        elapsed = time.perf_counter() - start
        if iterations % 2 == 0:
            print(
                f"  Progress: {elapsed:.1f}s / {duration_seconds}s ({iterations} iterations)",
                flush=True,
            )

    elapsed = time.perf_counter() - start
    print(
        f"Completed {iterations} iterations in {elapsed:.2f} seconds "
        f"(checksum {checksum:.2f})",
        flush=True,
    )
    return iterations


if __name__ == "__main__":
    duration = float(sys.argv[1]) if len(sys.argv) > 1 else 10.0
    cpu_intensive_work(duration)
