#!/usr/bin/env python3
"""
Known CPU workload for energy measurement verification.
Performs matrix multiplication to generate consistent CPU load.
"""

import sys
import time
import os


def cpu_intensive_work(duration_seconds: float = 10.0):
    """Perform CPU-intensive matrix operations for a fixed duration."""
    import random

    print(f"PID: {os.getpid()}", flush=True)
    print(f"Starting CPU workload for {duration_seconds} seconds...", flush=True)

    start = time.perf_counter()
    iterations = 0

    # Matrix size - adjust for desired CPU load
    size = 200

    while time.perf_counter() - start < duration_seconds:
        # Create random matrices
        a = [[random.random() for _ in range(size)] for _ in range(size)]
        b = [[random.random() for _ in range(size)] for _ in range(size)]

        # Matrix multiplication (CPU intensive)
        result = [
            [sum(a[i][k] * b[k][j] for k in range(size)) for j in range(size)]
            for i in range(size)
        ]

        iterations += 1

        # Progress indicator
        elapsed = time.perf_counter() - start
        if iterations % 2 == 0:
            print(
                f"  Progress: {elapsed:.1f}s / {duration_seconds}s ({iterations} iterations)",
                flush=True,
            )

    elapsed = time.perf_counter() - start
    print(f"Completed {iterations} iterations in {elapsed:.2f} seconds", flush=True)
    return iterations


if __name__ == "__main__":
    duration = float(sys.argv[1]) if len(sys.argv) > 1 else 10.0
    cpu_intensive_work(duration)
