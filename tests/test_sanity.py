import timeit
import logging
import torch
import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder


def test_energy_monitor_sanity_check():
    # Setup logging
    emt.setup_logger(
        log_dir="./logs/sanity_check/",
        logging_level=logging.INFO,
        mode="w",
    )

    # Simple tensor addition to generate activity
    def dummy_tensor_operation():
        device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
        a = torch.randint(1, 10, (100,), device=device)
        b = torch.randint(1, 10, (100,), device=device)
        return a + b

    # Run EnergyMonitor with dummy workload
    try:
        with EnergyMonitor(
            name="sanity_check",
            trace_recorders=[CSVRecorder(), TensorboardRecorder()],
        ) as monitor:
            execution_time = timeit.timeit(dummy_tensor_operation, number=100)
            assert monitor.total_consumed_energy >= 0.0
            assert execution_time >= 0.0
            assert isinstance(monitor.consumed_energy, dict)
            # check if csv files were created
            assert len(monitor.trace_recorders) == 2
    except Exception as e:
        assert False, f"EnergyMonitor sanity check failed with error: {e}"
