import timeit
import logging
import torch
import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder

emt.setup_logger(
    log_dir="./logs/tensor_addition_torch/",
    logging_level=logging.DEBUG,
    mode="w",
)


def add_tensors_gpu():
    device = torch.device(device if torch.cuda.is_available() else "cpu")
    # Generate random data
    a = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)
    b = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)

    return a + b


with EnergyMonitor(
    name="tensor_addition",
    trace_recorders=[CSVRecorder(), TensorboardRecorder()],
) as monitor:
    # repeat the addition 10000 times
    execution_time = timeit.timeit(add_tensors_gpu, number=1000000)
    print(f"execution time: {execution_time:.2f} Seconds.")
    print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
    print(f"energy consumption: {monitor.consumed_energy}")
