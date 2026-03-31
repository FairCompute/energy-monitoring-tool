import csv
import json
from pathlib import Path


TOLERANCE = 0.02
FIXTURES_DIR = Path(__file__).parent / "fixtures" / "rapl_verification"


def _load_python_total_energy_from_trace(path: Path) -> float:
    with open(path, newline="") as file:
        rows = list(csv.DictReader(file))
    return float(rows[-1]["consumed_utilized_energy_cumsum"]) if rows else 0.0


def _load_rust_total_energy(path: Path) -> float:
    with open(path) as file:
        payload = json.load(file)
    return float(payload["total_energy"])


def test_python_and_rust_fixture_totals_within_two_percent():
    python_total = _load_python_total_energy_from_trace(
        FIXTURES_DIR / "python_trace_fixture.csv"
    )
    rust_total = _load_rust_total_energy(FIXTURES_DIR / "rust_output_fixture.json")

    relative_error = abs(rust_total - python_total) / python_total
    assert relative_error <= TOLERANCE


def test_rust_fixture_contains_per_socket_devices_for_multi_socket_attribution():
    with open(FIXTURES_DIR / "rust_output_fixture.json") as file:
        payload = json.load(file)

    socket_devices = [
        device for device in payload.get("devices", {}) if device.startswith("rapl:socket:")
    ]
    assert socket_devices
