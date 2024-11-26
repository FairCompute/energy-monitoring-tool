import math
import time
import timeit
import random
import logging
from pathlib import Path
from itertools import product
import tensorflow as tf

import emt
from emt import EnergyMonitor

LOG_DIR = "./logs/tenosor_addition/"
LOG_TRACE_PATH = LOG_DIR + "traces/"
LOG_FILE_NAME = "emt.log"

emt.setup_logger(
    log_dir=LOG_DIR,
    log_file_name=LOG_FILE_NAME,
    logging_level=logging.DEBUG,
    mode="w",
)


def add_tensors_gpu(device="gpu"):
    with tf.device(device):
        # Generate random data
        a = tf.random.uniform(shape=(1000,), minval=1, maxval=100, dtype=tf.int32)
        b = tf.random.uniform(shape=(1000,), minval=1, maxval=100, dtype=tf.int32)
        return a + b


with EnergyMonitor(tracing_interval=20, log_trace_path=LOG_TRACE_PATH) as Monitor:
    # repeat the addition 10000 times
    execution_time = timeit.timeit(add_tensors_gpu, number=10000)
    print(f"execution time: {execution_time:.2f} Seconds.")
    print(f"energy consumption: {Monitor.total_consumed_energy:.2f} J")
    print(f"energy consumption: {Monitor.consumed_energy}")
