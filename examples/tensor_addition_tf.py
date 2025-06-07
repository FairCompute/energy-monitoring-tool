import timeit
import logging
import tensorflow as tf
import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder

_NAME = "tensor_addition_tf"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

emt.setup_logger(
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
    trace_recorders=[CSVRecorder(), TensorboardRecorder()],
) as monitor:
    # repeat the addition 100000 times
    execution_time = timeit.timeit(add_tensors_gpu, number=100000)
    print(f"execution time: {execution_time:.2f} Seconds.")
    print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
    print(f"energy consumption: {monitor.consumed_energy}")
