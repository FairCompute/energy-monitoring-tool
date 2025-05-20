import logging
import torch
import emt
from emt import EnergyMonitor


emt.setup_logger(
    log_dir="./logs/tensor_addition_torch/",
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
        name="tensor_addition",
    ) as monitor:
        add_tensors_gpu()

    print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
    print(f"energy consumption: {monitor.consumed_energy}")
