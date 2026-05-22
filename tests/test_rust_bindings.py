import os

import pytest


def test_rust_bindings_create_energy_group():
    from emt._rust import EnergyGroup, RaplCollector

    group = EnergyGroup.create(
        collector=RaplCollector(),
        rate=10.0,
        pids=[os.getpid()],
    )

    assert group.is_running() is False
    assert group.total_energy() == pytest.approx(0.0)
    assert group.energy_trace() == {
        "pid": [],
        "device": [],
        "energy": [],
        "timestamp": [],
    }

    group.poll_data()
    group.shutdown()
