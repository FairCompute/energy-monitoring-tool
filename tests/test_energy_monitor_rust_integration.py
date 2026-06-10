import math
import os
import time
from pathlib import Path

import pytest

from emt import EnergyMonitor

RAPL_ROOT = Path("/sys/class/powercap")


def _can_read(path: Path) -> bool:
    try:
        with path.open("r", encoding="utf-8") as file:
            file.read(1)
        return True
    except OSError:
        return False


def _readable_rapl_entries(root: Path = RAPL_ROOT) -> list[Path]:
    try:
        entries = root.iterdir()
    except OSError:
        return []

    return sorted(
        entry
        for entry in entries
        if "rapl" in entry.name and _can_read(entry / "energy_uj")
    )


def _read_energy_uj(entry: Path) -> int | None:
    try:
        return int((entry / "energy_uj").read_text(encoding="utf-8").strip())
    except (OSError, ValueError):
        return None


def _any_counter_advanced(entries: list[Path], duration_s: float = 0.5) -> bool:
    before = {entry: _read_energy_uj(entry) for entry in entries}
    _run_cpu_bound_workload(duration_s)
    after = {entry: _read_energy_uj(entry) for entry in entries}

    for entry in entries:
        before_value = before.get(entry)
        after_value = after.get(entry)
        if (
            before_value is not None
            and after_value is not None
            and after_value > before_value
        ):
            return True
    return False


@pytest.fixture(scope="module")
def rust_module():
    module = pytest.importorskip(
        "emt._rust",
        reason=(
            "PyO3 extension emt._rust is not importable; build/install it with "
            "`maturin develop` before running Rust-backed integration tests."
        ),
    )
    if not hasattr(module, "RustMonitor"):
        pytest.fail(
            "emt._rust imported but does not expose RustMonitor; rebuild the PyO3 "
            "extension from current sources."
        )
    if not hasattr(module.RustMonitor, "gpu_available"):
        pytest.fail(
            "emt._rust.RustMonitor does not expose gpu_available; rebuild the PyO3 "
            "extension from current sources."
        )
    return module


@pytest.fixture(scope="module")
def readable_rapl_entries():
    entries = _readable_rapl_entries()
    if not entries:
        pytest.skip(
            f"No readable RAPL energy counters found under {RAPL_ROOT}; this "
            "integration test requires Linux powercap/RAPL access."
        )
    if not _any_counter_advanced(entries):
        pytest.skip(
            "Readable RAPL energy counters were found, but none advanced during "
            "a CPU warmup; skipping strict positive-energy integration test."
        )
    return entries


def _run_cpu_bound_workload(duration_s: float) -> int:
    deadline = time.perf_counter() + duration_s
    value = 0
    while time.perf_counter() < deadline:
        value = ((value * 1_664_525) + 1_013_904_223) & 0xFFFFFFFF
    return value


def test_energy_monitor_reports_energy_from_real_rust_backend(
    monkeypatch, rust_module, readable_rapl_entries
):
    monkeypatch.setenv("EMT_DISABLE_GPU", "1")

    with EnergyMonitor(
        name="RustIntegrationCPU",
        pid=os.getpid(),
        rate=20.0,
        startup_delay_s=0.1,
    ) as monitor:
        assert monitor._rust_backend is not None, (
            "EnergyMonitor fell back to the Python backend even though emt._rust "
            "and readable RAPL counters are available."
        )
        assert isinstance(monitor._rust_backend, rust_module.RustMonitor)

        _run_cpu_bound_workload(2.5)

    total_energy = monitor.total_consumed_energy
    consumed_energy = monitor.consumed_energy

    assert total_energy > 0.0, (
        "Rust-backed EnergyMonitor reported no energy for a CPU-bound workload "
        f"with readable RAPL counters: {[entry.name for entry in readable_rapl_entries]}"
    )
    assert consumed_energy
    assert "RAPLSoC" in consumed_energy
    assert "cpu" not in consumed_energy
    assert "dram" not in consumed_energy
    assert "gpu" not in consumed_energy
    assert sum(consumed_energy.values()) == pytest.approx(
        total_energy, rel=1e-6, abs=1e-9
    )

    for device_name, joules in consumed_energy.items():
        assert math.isfinite(joules), f"{device_name} energy is not finite"
        assert joules >= 0.0, f"{device_name} energy is negative"
