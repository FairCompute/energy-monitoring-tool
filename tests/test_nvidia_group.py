import unittest
import asyncio
import math, random 
from itertools import product
from typing import Collection
from emt.power_groups import NvidiaGPU

try:
    import pynvml

    pynvml.nvmlInit()
    nvml_available = True
except (ImportError, pynvml.NVMLError):
    nvml_available = False


def foo():
    a = [random.randint(1, 100) for _ in range(1000)]
    b = [random.randint(1, 10) for _ in range(1000)]
    return [math.factorial(x) for x in map(sum, product(a , b))]

async def cancel_after(delay, tasks:Collection[asyncio.Task]):
    await asyncio.sleep(delay)
    for task in tasks:
        task.cancel()

@unittest.skipUnless(nvml_available, "NVML library is not available!")
class TestNvidiaGroup(unittest.IsolatedAsyncioTestCase):

    def test_object_creation(self):
        nvidia_group = NvidiaGPU()
        print(nvidia_group._read_utilization())
        print(nvidia_group._read_energy())
        self.assertTrue(nvidia_group.zones)
        
    async def test_power_group(self):
        power_groups = [NvidiaGPU()]
        tasks = [asyncio.create_task(pG.commence()) for pG in power_groups]
        cancel_task = asyncio.create_task(cancel_after(1, tasks))
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.gather(*tasks, cancel_task)

if __name__ == '__main__':
    unittest.main()