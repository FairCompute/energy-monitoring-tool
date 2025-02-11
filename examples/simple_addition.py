import timeit
import logging
import emt
from emt import EnergyMonitor


emt.setup_logger(
    log_dir="./logs/simple_addition/",
    logging_level=logging.DEBUG,
    mode="w",
)


def foo():
    return sum([i**30 for i in range(500)])


with EnergyMonitor(
    name="simple_addition",
) as monitor:
    # repeat the addition 100000 times
    execution_time = timeit.timeit(foo, number=10000)
    print(f"execution time: {execution_time:.2f} Seconds.")
    print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
    print(f"energy consumption: {monitor.consumed_energy}")
