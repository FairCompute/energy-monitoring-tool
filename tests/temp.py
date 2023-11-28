import math
import time
import timeit
import random
from emt import EnergyMetering
from itertools import product
from pathlib import Path
import logging
import emt

emt.setup_logger(Path(Path(), 'emt.log'), logging_level=logging.DEBUG)

def foo():
    a = [random.randint(1, 100) for _ in range(1000)]
    b = [random.randint(1, 10) for _ in range(1000)]
    return [math.factorial(x) for x in map(sum, product(a, b))]

with EnergyMetering() as metering:
    execution_time = timeit.timeit(lambda:None, number=10)
    time.sleep(6)
    print(f'execution time of None is: {execution_time}')
    print(f'energy consumption of None: {metering.consumed_energy}')


with EnergyMetering() as metering:
    execution_time = timeit.timeit(foo, number=10)
    print(f'execution time of foo is: {execution_time}')
    print(f'energy consumption of foo: {metering.consumed_energy}')


