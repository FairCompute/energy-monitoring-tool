import unittest
import math, random 
from itertools import product

def foo():
    a = [random.randint(1, 100) for _ in range(1000)]
    b = [random.randint(1, 10) for _ in range(1000)]
    return [math.factorial(x) for x in map(sum, product(a , b))]

class TestFooPowerTrace(unittest.TestCase):
    def setUp(self) -> None:
        return super().setUp()

    def test_intel_cpu_trace(self):
        foo()
        

if __name__ == '__main__':
    unittest.main()