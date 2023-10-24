import math
import time
import random
import threading
from emt import EnergyMeter
from emt.power_groups import IntelCPU
from itertools import product
from functools import reduce
import psutil
# Assuming you have the EnergyMeter class defined as in your initial code

# Create an instance of the EnergyMeter class
energy_meter = EnergyMeter(power_groups=[IntelCPU()])  # Provide your PowerGroup objects


def foo():
    a = [random.randint(1, 100) for _ in range(1000)]
    b = [random.randint(1, 10) for _ in range(1000)]
    return [math.factorial(x) for x in map(sum, product(a, b))]

# Create a separate thread and start it
energy_meter_thread = threading.Thread(target=lambda :energy_meter.run())
energy_meter_thread.start()
time.sleep(1)

# Now, the EnergyMeter will run in the background thread without blocking the main thread.
# You can continue doing other tasks in the main thread.

class TEST:

    def __init__(self):
        self.processes = [psutil.Process()]

    def main(self):    
        for _ in range(10):
            foo()
            print(energy_meter.consumed_energy)

        energy_meter.conclude()
        energy_meter_thread.join()

test = TEST()
test.main()
