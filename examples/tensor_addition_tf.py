import timeit
import logging
import tensorflow as tf
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder, setup_logger

_NAME = "tensor_addition_tf"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

setup_logger(
    logger,
    logging_level=logging.DEBUG,
    log_file_name=LOG_FILE_NAME,
    mode="w",
)


def add_tensors_gpu(device="gpu"):
    with tf.device(device):
        # Generate random data
        a = tf.random.uniform(shape=(1000,), minval=1, maxval=100, dtype=tf.int32)
        b = tf.random.uniform(shape=(1000,), minval=1, maxval=100, dtype=tf.int32)
        return a + b


with EnergyMonitor(
    name=_NAME,
    trace_recorders=[
        CSVRecorder("./csv_traces"),
        TensorboardRecorder("./tensorboard_logs"),
    ],
) as monitor:
    # repeat the addition 100000 times
    execution_time = timeit.timeit(add_tensors_gpu, number=100000)

logger.info(f"\n\n{'*' * 20} Context name: {_NAME} {'*' * 20}")
logger.info(f"execution time: {execution_time:.2f} Seconds.")
logger.info(
    f"energy consumption: {monitor.total_consumed_energy} {monitor.energy_unit}"
)
logger.info(f"energy consumption: {monitor.consumed_energy} {monitor.energy_unit}")
