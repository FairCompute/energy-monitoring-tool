# RAPL Accuracy Verification

This note documents the verification path for comparing the Rust collector in
`src/collectors/rapl.rs` against the Python reference implementation in
`emt/power_groups/rapl.py`.

## Acceptance criteria

- Run a CPU-bound benchmark on a **physical host** for 30 seconds.
- Confirm the Rust-measured total energy is within **±2%** of the
  Python-measured total.
- Verify `DeltaReader` wrap-around handling with an automated unit test.
- Verify multi-socket package/component discovery so vCPU attribution is based
  on the correct socket topology.

## Automated checks committed in this repository

- Rust unit test:
  - `delta_reader_returns_zero_on_counter_wraparound`
  - `scan_powercap_entries_keeps_multi_socket_components_separate`
- Verification harness coverage:
  - The RAPL preflight checks and the ±2% acceptance analysis logic remain
    implemented in `verification/verify.py`.
  - These transition-only verification checks are intentionally kept localized
    to the `verification/` workflow instead of the permanent `tests/` suite.

## Physical-host verification command

Build the Rust CLI and run the verification harness on a machine that exposes
real RAPL counters under `/sys/class/powercap`:

```bash
cargo build --release
python verification/verify.py --iterations 3 --duration 30
```

The harness now fails fast when the host has no readable RAPL counters instead
of running indefinitely on an unsupported environment.

## How to read the output

`verification/verify.py` writes `verification/verification_results.json` with an
`analysis` section:

```json
{
  "analysis": {
    "tolerance_percent": 2.0,
    "python_vs_rust": {
      "python_mean_j": 0.0,
      "rust_mean_j": 0.0,
      "relative_diff_percent": 0.0,
      "within_tolerance": true,
      "iterations_compared": 3
    }
  }
}
```

The acceptance criterion is satisfied when
`analysis.python_vs_rust.within_tolerance` is `true`.

## Multi-socket verification notes

The Rust collector records package energy per socket and keeps socket
sub-components attached to their owning `intel-rapl:N` entry. The unit test for
`scan_powercap_entries()` checks that a multi-socket powercap layout does not
mix package, core, or uncore readers across sockets.

For manual host validation on a multi-socket machine, review the `devices`
section in the Rust JSON output and confirm that each `rapl:socket:<id>:package`
entry corresponds to a real package exposed by `/sys/class/powercap`.
