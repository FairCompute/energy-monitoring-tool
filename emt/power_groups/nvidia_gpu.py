import time
import asyncio
import pynvml
import subprocess
import pandas as pd
import numpy as np
from typing import Mapping
from functools import cached_property
from collections import defaultdict
from emt.power_groups.power_group import PowerGroup

class PowerCalculator:
    """
    Integrates the instantaneous power usages (W) over the time-delta between the previous call.
    This performs a definite integral of the instantaneous power, using a high-resolution timer,
    the timer measures the time passed since the last call and integrates the power using the
    trapezoidal rule.
    """

    def __init__(self, init_energy: float = 0.0):
        self._init_time = time.perf_counter()
        self._init_energy = init_energy

    def __call__(self, current_energy: float):
        """
        Add an instantaneous power value (in watts) for a power zone and calculate the cumulative energy
        consumption in Joules.
        Args:
            current_energy (float): total consumed energy till current point in J
        Returns:
            float: Power consumption in watts.
        """
        current_time = time.perf_counter()
        time_delta = current_time - self._init_time
        # Update previous time for the next call
        self._init_time = current_time
        # Calculate the energy consumed during this time interval
        self._power = ((current_energy - self._init_energy) / time_delta)
        # Update previous energy for the next call
        self._init_energy = current_energy
        return self._power


class NvidiaGPU(PowerGroup):
    """
    __summary__
    """

    def __init__(self, **kwargs):
        """
        __summary__
        Args:
                **kwargs:     The arguments be passed to the `PowerGroup`.
        """
        # by default a rate 5Hz is used to collect energy_trace.
        kwargs.update({"rate": kwargs.get("rate", 10)})
        super().__init__(**kwargs)
        # get the process tree for the tracked process
        self.processes = [self.tracked_process] + self.tracked_process.children(
            recursive=True
        )
        pynvml.nvmlInit()
        zones = []
        power_calculators = []
        for index in range(pynvml.nvmlDeviceGetCount()):
            zone_handle = pynvml.nvmlDeviceGetHandleByIndex(index)
            zone_current_energy = pynvml.nvmlDeviceGetTotalEnergyConsumption(zone_handle)
            zones.append(zone_handle)
            power_calculators.append(PowerCalculator(zone_current_energy))
        self._zones = zones
        self._power_calculators = power_calculators

    @cached_property
    def pids(self):
        pids = [p.pid for p in self.processes]
        return pids

    @cached_property
    def zones(self):
        """
        Return unique IDs for each GPU in the system.
        """
        names = [pynvml.nvmlDeviceGetIndex(zone) for zone in self._zones]
        return names
    
    @classmethod
    def is_available(cls):
        """
        Checks if the NVML is available.
        """
        try:
            pynvml.nvmlInit()
            return True
        except pynvml.NVMLError:
            return False
        
    def _read_utilized_energy(self):
        """
        """    
        # initialize energy_zones using defaultdict
        consumed_energy = 0.0
        for zone, zone_handle, power_calculator in zip(
            self.zones, self._zones, self._power_calculators
        ):
            try:
                # Retrieves power usage in mW, divide by 1000 to get in W.
                # Measure total energy consumption at this point in time
                current_total_energy = pynvml.nvmlDeviceGetTotalEnergyConsumption(zone_handle)
                # get the zone level utilizations
                zone_energy = power_calculator(current_total_energy)
                zone_gpu_utilization = pynvml.nvmlDeviceGetUtilizationRates(zone_handle)
                zone_memory_total = pynvml.nvmlDeviceGetMemoryInfo(zone_handle).total
                self.logger.debug(
                f"Zone: {zone}, energy: {zone_energy},"
                f" gpu_util: {zone_gpu_utilization}, memory_total: {zone_memory_total / (1024 ** 2)}"
                )
                # Get running processes on the GPU
                processes = pynvml.nvmlDeviceGetComputeRunningProcesses(zone_handle)
                # Filter processes based on self.pids and if the memory usage is not N/A
                filtered_processes = [
                    process for process in processes
                    if (process.pid in self.pids) and (process.usedGpuMemory) 
                ]
                self.logger.debug(f"Total # processes: {len(filtered_processes)}")
                zone_consumed_energy = 0.0
                for process in filtered_processes:
                    pid = process.pid
                    memory_used = process.usedGpuMemory  # Memory used by this specific process   
                    # Here you might estimate energy usage based on memory usage or other metrics
                    # This is a simplistic approach and might not be accurate
                    estimated_energy_usage = (memory_used / zone_memory_total) * zone_energy
                    self.logger.debug(f"PID: {pid}, Memory Used: {memory_used / (1024 ** 2)} MB," 
                                      f" Estimated Energy Used: {estimated_energy_usage:.2f} J"
                                     )
                    zone_consumed_energy += estimated_energy_usage
            except pynvml.NVMLError:
                raise Exception
            consumed_energy += zone_consumed_energy
            # get time elapsed since
        return consumed_energy

    
    async def commence(self) -> None:
        """
        This commence a periodic execution at a set rate:
            [get_energy_trace -> update_energy_consumption -> async_wait]

        The periodic execution is scheduled at the rate dictated by `self.sleep_interval`, during the
        instantiation. The energy consumption is updated using the `_read_energy` and `_read_utilization`
        methods. The method credits energy consumption to the tracked processes by weighting the energy
        trace, obtained from each zone, by the utilization of the zone by the processes.
        """
        while True:
            consumed_energy = self._read_utilized_energy()
            self._count_trace_calls += 1
            self.logger.debug(
                f"Obtained energy trace no.{self._count_trace_calls} from {type(self).__name__ }:\n"
                f"consumed utilized energy: {consumed_energy}"
            )
            self._consumed_energy = consumed_energy
            await asyncio.sleep(self.sleep_interval)

    def shutdown(self):
        """
        The cleanup routine executed when the powergroup monitoring is finished
        or aborted by the user.
        """
        pynvml.nvmlShutdown()
