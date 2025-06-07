import timeit
import logging
import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder


def test_energy_monitor_sanity_check():
    # Simple CPU operation to generate activity
    def dummy_cpu_operation():
        return sum(range(1000))

    # Run EnergyMonitor with dummy workload
    try:
        with EnergyMonitor(
            name="sanity_check",
            trace_recorders=[CSVRecorder()],
        ) as monitor:
            execution_time = timeit.timeit(dummy_cpu_operation, number=100)
            assert monitor.total_consumed_energy >= 0.0
            assert execution_time >= 0.0
            assert isinstance(monitor.consumed_energy, dict)
            # check if csv file was created
            assert len(monitor.trace_recorders) == 1
    except Exception as e:
        assert False, f"EnergyMonitor sanity check failed with error: {e}"
