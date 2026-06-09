import os
import time
from pathlib import Path

from scripts import release_candidate_smoke as rc_smoke


def test_latest_wheel_prefers_newest_file(tmp_path):
    old_wheel = tmp_path / "emt-0.0.1a3-cp312-cp312-linux_x86_64.whl"
    new_wheel = tmp_path / "emt-0.0.1rc1-cp312-cp312-linux_x86_64.whl"
    old_wheel.write_text("", encoding="utf-8")
    new_wheel.write_text("", encoding="utf-8")
    old_time = time.time() - 10
    os.utime(old_wheel, (old_time, old_time))

    assert rc_smoke.latest_wheel(tmp_path) == new_wheel


def test_wheel_looks_like_release_candidate():
    assert rc_smoke.wheel_looks_like_release_candidate(
        Path("emt-0.0.1rc1-cp312-cp312-linux_x86_64.whl")
    )
    assert not rc_smoke.wheel_looks_like_release_candidate(
        Path("emt-0.0.1a3-cp312-cp312-linux_x86_64.whl")
    )


def test_write_energy_monitor_smoke_uses_public_context_manager(tmp_path):
    script = tmp_path / "smoke.py"

    rc_smoke.write_energy_monitor_smoke(script)

    contents = script.read_text(encoding="utf-8")
    assert "from emt import EnergyMonitor" in contents
    assert "with EnergyMonitor(" in contents
