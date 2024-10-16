import unittest
import asyncio
import math, random
from itertools import product
from typing import Collection
from emt.power_groups import RAPLSoC


def foo():
    a = [random.randint(1, 100) for _ in range(1000)]
    b = [random.randint(1, 10) for _ in range(1000)]
    return [math.factorial(x) for x in map(sum, product(a , b))]

async def cancel_after(delay, tasks:Collection[asyncio.Task]):
    await asyncio.sleep(delay)
    for task in tasks:
        task.cancel()

class TestRAPLGroup(unittest.IsolatedAsyncioTestCase):

    def test_object_creation(self):
        rapl_group = RAPLSoC()
        self.assertTrue(rapl_group.devices)
        
    async def test_power_group(self):
        power_groups = [IntelCPU()]
        tasks = [asyncio.create_task(pG.commence()) for pG in power_groups]
        cancel_task = asyncio.create_task(cancel_after(1, tasks))
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.gather(*tasks, cancel_task)

if __name__ == '__main__':
    unittest.main()