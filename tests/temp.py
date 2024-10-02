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

emt.setup_logger(Path(Path(), "emt.log"), logging_level=logging.DEBUG)

# def foo():
#     a = [random.randint(1, 100) for _ in range(1000)]
#     b = [random.randint(1, 10) for _ in range(1000)]
#     return [math.factorial(x) for x in map(sum, product(a, b))]


def foo(device="gpu"):
    with tf.device(device):
        # Generate random data
        a = tf.random.uniform(shape=(1000,), minval=1, maxval=100, dtype=tf.int32)
        b = tf.random.uniform(shape=(1000,), minval=1, maxval=10, dtype=tf.int32)
        return a + b

        # # Perform the map and sum the pairs using TensorFlow ops
        # map_sum_result = tf.map_fn(lambda x: tf.reduce_sum(x), tf.transpose(tf.stack([a, b])))
        # # Perform the factorial using TensorFlow ops
        # factorial_result = tf.map_fn(lambda x: tf.reduce_sum(x), map_sum_result)
        # return factorial_result


# with EnergyMetering() as metering:
#     execution_time = timeit.timeit(lambda:None, number=1)
#     time.sleep(0.6)
#     print(f'execution time of None is: {execution_time}')
#     print(f'energy consumption of None: {metering.total_consumed_energy}')
#     print(f'energy consumption of None: {metering.consumed_energy}')

# print('\n')

with EnergyMonitor() as Monitor:
    execution_time = timeit.timeit(foo, number=10000)
    print(f"execution time of foo is: {execution_time}")
    print(f"energy consumption of foo: {Monitor.total_consumed_energy}")
    print(    f"energy consumption of foo: {Monitor.consumed_energy}")
