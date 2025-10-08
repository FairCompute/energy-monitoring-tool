import timeit
import logging
from emt import EnergyMonitor

__NAME = "simple_addition"
logger = logging.getLogger(__NAME)
LOG_FILE_NAME = f"{__NAME}.log"

logging.basicConfig(level=logging.INFO)


def foo():
    return sum([i**30 for i in range(500)])


with EnergyMonitor(
    name=__NAME,
) as monitor:
    # repeat the addition 100000 times
    execution_time = timeit.timeit(foo, number=10000)
    logger.info(f"execution time: {execution_time:.2f} Seconds.")
    logger.info(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
    logger.info(f"energy consumption: {monitor.consumed_energy}")
