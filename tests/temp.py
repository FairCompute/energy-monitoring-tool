import math
import time
import timeit
import random
import logging
from pathlib import Path
from itertools import product

import emt
from emt import EnergyMetering

emt.setup_logger(Path(Path(), 'emt.log'), logging_level=logging.DEBUG)

def foo():
    a = [random.randint(1, 100) for _ in range(1000)]
    b = [random.randint(1, 10) for _ in range(1000)]
    return [math.factorial(x) for x in map(sum, product(a, b))]

with EnergyMetering() as metering:
    execution_time = timeit.timeit(lambda:None, number=1)
    time.sleep(0.6)
    print(f'execution time of None is: {execution_time}')
    print(f'energy consumption of None: {metering.total_consumed_energy}')
    print(f'energy consumption of None: {metering.consumed_energy}')

print('\n')

with EnergyMetering() as metering:
    execution_time = timeit.timeit(foo, number=1)
    print(f'execution time of foo is: {execution_time}')
    print(f'energy consumption of foo: {metering.total_consumed_energy}')
    print(f'energy consumption of fo: {metering.consumed_energy}')


