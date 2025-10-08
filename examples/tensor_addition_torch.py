import logging
import torch
from emt import EnergyMonitor
from emt.utils import setup_logger

_NAME = "tensor_addition_torch"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

setup_logger(
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

    logger.info(f"\n\n{'*' * 20} Context name: {_NAME} {'*' * 20}")
    logger.info(f"energy consumption: {monitor.total_consumed_energy} {monitor.energy_unit}")
    logger.info(f"energy consumption: {monitor.consumed_energy} {monitor.energy_unit}")
