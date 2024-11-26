import asyncio
import pynvml
import time
import numpy as np
from functools import cached_property
from collections import defaultdict
from emt.power_groups.power_group import PowerGroup


class DeltaCalculator:
    """
    Calculates the difference between two energy readings in Joules. it is initialized with an initial energy of 0.0 .
    """

    def __init__(self, init_energy: float = 0.0):
        self._init_energy = init_energy

    def __call__(self, current_energy: float):
        """
        Add an instantaneous energy value (in joules) for a power zone and calculate the difference in energy
        consumption in Joules.
        Args:
            current_energy (float): total consumed energy till current point in J
        Returns:
            float: Energy consumption in J.
        """
        self._energy = (current_energy - self._init_energy) / 1000
        # Update previous energy for the next call
        self._init_energy = current_energy
        return self._energy


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
        delta_calculators = []
        for index in range(pynvml.nvmlDeviceGetCount()):
            zone_handle = pynvml.nvmlDeviceGetHandleByIndex(index)
            zone_current_energy = pynvml.nvmlDeviceGetTotalEnergyConsumption(
                zone_handle
            )
            zones.append(zone_handle)
            delta_calculators.append(DeltaCalculator(zone_current_energy))
        self._zones = zones
        self._delta_calculators = delta_calculators

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

    def _read_energy(self):
        """
        Retrieves the instantaneous consumed energy, and calculates the delta of energy for each zone.
        Returns:
            defaultdict: energy usage of each zone
        """
        # initialize energy_zones using defaultdict
        zone_consumed_energy = defaultdict(int)
        for zone, zone_handle, delta_calculator in zip(
            self.zones, self._zones, self._delta_calculators
        ):
            try:
                # Retrieve total energy consumption at this point in time
                current_total_energy = pynvml.nvmlDeviceGetTotalEnergyConsumption(
                    zone_handle
                )
                # get the zone level utilizations and delta energy
                zone_consumed_energy[zone] = delta_calculator(current_total_energy)
            except pynvml.NVMLError as e:
                self.logger.warning(f"Could not read energy due to NVML error: {e}")
        return zone_consumed_energy

    def _read_utilization(self):
        """ "
        Measures the process level memory utilization for each zone, and uses that as a proxy for GPU energy utilization.

        Returns:
            defaultdict: memory utilization of each zone
        """
        zone_process_utilization = defaultdict(int)
        for zone, zone_handle in zip(self.zones, self._zones):
            try:
                # get total available memory of a zone (zone: gpu 0,1,2 etc)
                zone_memory_total = pynvml.nvmlDeviceGetMemoryInfo(zone_handle).total
                # get the active processes in a zone
                processes = pynvml.nvmlDeviceGetComputeRunningProcesses(zone_handle)
                # filter processes based on self.pids and if the memory usage is not N/A
                filtered_processes = [
                    process
                    for process in processes
                    if (process.pid in self.pids) and (process.usedGpuMemory)
                ]
                zone_memory_use = 0.0
                for process in filtered_processes:
                    memory_used = (
                        process.usedGpuMemory
                    )  # Memory used by a specific process
                    # memory usage of all filtered processes in a zone
                    zone_memory_use += memory_used
                zone_process_utilization[zone] = zone_memory_use / zone_memory_total
            except pynvml.NVMLError as e:
                self.logger.warning(
                    f"Could not read utilizations due to NVML error: {e}"
                )
        return zone_process_utilization

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

            start_time = time.perf_counter()
            zone_consumed_energy = self._read_energy()
            measuremnet_time = time.perf_counter() - start_time

            zone_utilization = self._read_utilization()
            if zone_consumed_energy.keys() != zone_utilization.keys():
                raise ValueError("Dictionaries do not have the same zone_handle keys.")
            # get weighted sum of energy utilization
            consumed_utilized_energy = sum(
                zone_consumed_energy[zone] * zone_utilization[zone]
                for zone in zone_consumed_energy
            )
            # consume energy is sum of all the utilized consumed enrergies across the intervals
            self._consumed_energy += consumed_utilized_energy

            # add trace info
            self._energy_trace["trace_num"].append(self._count_trace_calls)
            self._energy_trace["total_energy"].append(
                round(np.sum(np.array(list(zone_consumed_energy.values()))), 2)
            )
            self._energy_trace["avg_utilization"].append(
                round(np.mean(np.array(list(zone_utilization.values()))), 2)
            )
            self._energy_trace["consumed_utilized_energy"].append(
                round(consumed_utilized_energy, 2)
            )
            self._energy_trace["consumed_utilized_energy_cumsum"].append(
                round(self._consumed_energy, 2)
            )
            self._energy_trace["measuremnet_time"].append(round(measuremnet_time, 4))
            # increase trace number by 1
            self._count_trace_calls += 1
            await asyncio.sleep(self.sleep_interval)

    def shutdown(self):
        """
        The cleanup routine executed when the powergroup monitoring is finished
        or aborted by the user.
        """
        pynvml.nvmlShutdown()
