import csv
import json
import io
from pathlib import Path


TOLERANCE = 0.02
FIXTURES_DIR = Path(__file__).parent / "fixtures" / "rapl_verification"
PYTHON_TRACE_CSV = """trace_num,timestamp,elapsed_s,proc_count,measurement_time,ps_util,cpu_util,norm_ps_util,total_energy_cpu,consumed_utilized_energy,consumed_utilized_energy_cumsum
0,1711900000.100,0.100,1,0.0012,4.17,6.35,0.66,18.20,12.00,12.00
1,1711900001.100,1.100,1,0.0011,4.17,6.20,0.67,17.30,11.50,23.50
2,1711900002.100,2.100,1,0.0012,4.17,6.10,0.68,18.40,12.30,35.80
"""


def _load_python_total_energy_from_trace(trace_content: str) -> float:
    rows = list(csv.DictReader(io.StringIO(trace_content)))
    return float(rows[-1]["consumed_utilized_energy_cumsum"]) if rows else 0.0


def _load_rust_total_energy(path: Path) -> float:
    with open(path) as file:
        payload = json.load(file)
    return float(payload["total_energy"])


def test_python_and_rust_fixture_totals_within_two_percent():
    python_total = _load_python_total_energy_from_trace(PYTHON_TRACE_CSV)
    rust_total = _load_rust_total_energy(FIXTURES_DIR / "rust_output_fixture.json")

    assert python_total > 0.0
    relative_error = abs(rust_total - python_total) / python_total
    assert relative_error <= TOLERANCE


def test_rust_fixture_contains_per_socket_devices_for_multi_socket_attribution():
    with open(FIXTURES_DIR / "rust_output_fixture.json") as file:
        payload = json.load(file)

    device_map = payload["devices"]
    socket_devices = [
        device for device in device_map.keys() if device.startswith("rapl:socket:")
    ]
    assert socket_devices
