import time
import tempfile
import unittest
import math, random
import logging
import asyncio
from pathlib import Path
from threading import Thread
from itertools import product
import unittest
from unittest.mock import Mock
from emt import EnergyMeter
from emt.power_groups import RAPLSoC


def foo():
    a = [random.randint(1, 100) for _ in range(1000)]
    b = [random.randint(1, 10) for _ in range(1000)]
    return [math.factorial(x) for x in map(sum, product(a, b))]


class TestEnergyMeter(unittest.TestCase):
    def setUp(self):
        # Create some mock PowerGroup instances for testing
        self.log_dir = tempfile.TemporaryDirectory()
        self.log_file = str(Path(self.log_dir.name, "energy_meter.log"))
        self.mock_power_group_1 = Mock()
        self.mock_power_group_2 = Mock()
        self.power_groups = [self.mock_power_group_1, self.mock_power_group_2]

    def test_init(self):
        # Test the __init__ method
        logging_interval = 900
        logging_level = logging.NOTSET
        energy_meter = EnergyMeter(
            self.power_groups, self.log_file, logging_interval, logging_level
        )
        self.assertEqual(energy_meter.power_groups, self.power_groups)
        self.assertEqual(energy_meter.monitoring, False)
        self.assertEqual(energy_meter.concluded, False)
        self.assertEqual(energy_meter._logging_interval, logging_interval)

    def test_asynchronous_shutdown(self):
        """
        The run method is run in the main thread while conclude is
        executed by a parallel thread asynchronously.
        """
        energy_meter = EnergyMeter(
            self.power_groups,
            self.log_file,
            logging_interval=1,
        )

        # Test calling conclude before commencement leads to
        # Runtime Error!
        energy_meter = EnergyMeter(self.power_groups)
        with self.assertRaises(RuntimeError):
            # concluding before commencement
            energy_meter.conclude()

        async def mock_commence():
            while True:
                await asyncio.sleep(1)

        def conclude_after_t_sec(t):
            time.sleep(t)
            energy_meter.conclude()

        # Mock the commence method to return a coroutine
        self.mock_power_group_1.commence = Mock()
        self.mock_power_group_2.commence = Mock()
        self.mock_power_group_1.commence.side_effect = mock_commence
        self.mock_power_group_2.commence.side_effect = mock_commence

        # Create a seperate thread to cancel the tracking aftert 1 sec.
        Thread(target=conclude_after_t_sec, args=(1,)).start()
        with self.assertLogs("EnergyMonitor", level="INFO"):
            energy_meter.run()

        self.assertTrue(self.mock_power_group_1.commence.called)
        self.assertTrue(self.mock_power_group_2.commence.called)
        self.assertTrue(self.mock_power_group_1.commence.awaited)
        self.assertTrue(self.mock_power_group_2.commence.awaited)

    def test_run_threaded(self):
        """
        The run method is run in a seperate thread while conclude is
        executed on the main thread after a while.
        """
        energy_meter = EnergyMeter(
            self.power_groups,
            self.log_file,
            logging_interval=1,
        )

        async def mock_commence():
            while True:
                await asyncio.sleep(1)

        def conclude_after_t_sec(t):
            time.sleep(t)
            energy_meter.conclude()

        # Mock the commence method to return a coroutine
        self.mock_power_group_1.commence = Mock()
        self.mock_power_group_2.commence = Mock()
        self.mock_power_group_1.commence.side_effect = mock_commence
        self.mock_power_group_2.commence.side_effect = mock_commence
        _thread = Thread(target=lambda: energy_meter.run())

        with self.assertLogs("EnergyMonitor", level="INFO"):
            _thread.start()
            conclude_after_t_sec(1)    
            _thread.join()
            
        self.assertTrue(self.mock_power_group_1.commence.called)
        self.assertTrue(self.mock_power_group_2.commence.called)
        self.assertTrue(self.mock_power_group_1.commence.awaited)
        self.assertTrue(self.mock_power_group_2.commence.awaited)

    def test_rapl_powergroup(self):
        intel_group = RAPLSoC()
        energy_meter = EnergyMeter((intel_group,))
        _thread = Thread(target=lambda: energy_meter.run())
        _thread.start()
        time.sleep(1)
        energy_meter.conclude()
        _thread.join

    def tearDown(self):
        # This will remove the temporary directory and all its contents
        self.log_dir.cleanup()


if __name__ == "__main__":
    unittest.main()
