import logging
import torch
import emt
from emt import EnergyMonitor

_NAME = "tensor_addition_torch"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

emt.setup_logger(
    logger,
    log_file_name=LOG_FILE_NAME,
    logging_level=logging.DEBUG,
    mode="w",
)


def add_tensors_gpu():
    device = "cuda" if torch.cuda.is_available() else "cpu"
    a = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)
    b = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)
    return a + b


if __name__ == "__main__":
    with EnergyMonitor(
        name=_NAME,
    ) as monitor:
        add_tensors_gpu()

    print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
    print(f"energy consumption: {monitor.consumed_energy}")
