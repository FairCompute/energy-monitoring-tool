import time
import asyncio
import pynvml
import subprocess
import pandas as pd
import numpy as np
from typing import Mapping
from functools import cached_property
from emt.power_groups.power_group import PowerGroup

class PowerIntegrator:
    """
    Integrates the instantaneous power usages (W) over the time-delta between the previous call.
    This performs a definite integral of the instantaneous power, using a high-resolution timer,
    the timer measures the time passed since the last call and integrates the power using the
    trapezoidal rule.
    """

    def __init__(self):
        self._init_time = time.perf_counter()
        self._init_power = 0.0
        self._energy = 0

    def __call__(self, current_power):
        """
        Add an instantaneous power value (in watts) for a power zone and calculate the cumulative energy
        consumption in Joules.
        Args:
            power_watt (float): Instantaneous power usage in watts.
        Returns:
            float: Cumulative energy consumption in watt-seconds.
        """
        energy_delta = 0
        current_time = time.perf_counter()
        time_delta = current_time - self._init_time
        # Calculate the energy consumed during this time interval using the trapezoidal rule
        energy_delta = ((current_power + self._init_power) / 2.0) * time_delta
        self._energy += energy_delta
        # Update previous time and power for the next call
        self._init_time = current_time
        self._init_power = current_power

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
        power_integrators = []
        for index in range(pynvml.nvmlDeviceGetCount()):
            zone_handle = pynvml.nvmlDeviceGetHandleByIndex(index)
            zones.append(zone_handle)
            power_integrators.append(PowerIntegrator())
        self._zones = zones
        self._power_integrators = power_integrators

    @cached_property
    def pids(self):
        pids = [p.pid for p in self.processes]
        return pids

    @cached_property
    def zones(self):
        """
        Return unique IDs for each GPU in the system.
        """
        names = [ pynvml.nvmlDeviceGetIndex(zone) for zone in self._zones]
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
        Retrieves instantaneous power usages (W) of all GPUs in use by the tracked processes.
        Integrates the power using the corresponding power integrator for the zone, reports
        the cumulative energy fro each zone.
        """
        energy_zones = {zone: 0.0 for zone in self.zones}
        for zone, zone_handle, integrator in zip(
            self.zones, self._zones, self._power_integrators
        ):
            try:
                # Retrieves power usage in mW, divide by 1000 to get in W.
                power_usage = pynvml.nvmlDeviceGetPowerUsage(zone_handle) / 1000
                energy_zones[zone] = integrator(power_usage)
            except pynvml.NVMLError:
                raise Exception
            # get time elapsed since
        return energy_zones


    def _read_utilization(self) -> Mapping[int, float]:
        """
        This method provides utilization (per-zone) of the compute devices by the tracked
        processes.The is used to attribute a proportionate energy credit to the processes.

        """
        def _filter(pid):
            """
            The filter masks out the `pid` entries not tracked 
            by the energy monitor and returns the boolean mask.
            """
            keep = False
            if not np.isnan(pid):
                keep = True if int(pid) in self.pids else False
            return keep
        
        command = "nvidia-smi  pmon -c 1"
        output = subprocess.check_output(command, shell=True, text=True)
        lines = output.rstrip().split("\n")
        header = lines[0][1:].split()  # Extract field names from the header
        # the second line is units, data begins at the third line
        data = [line.split() for line in lines[2:] if line.strip()]
        df = pd.DataFrame(data, columns=header)[["gpu", "pid", "sm", "mem"]]
        df = df.apply(pd.to_numeric, errors="coerce")
        # filter out pids that are not relevant
        filter = df['pid'].apply(_filter)
        df_system = df.drop(columns=['pid'])
        df_processes  = df_system[filter]
        df_processes= df_processes.groupby('gpu').sum().fillna(0.0)
        return df_processes.to_dict(orient='index')

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
            utilization_trace = self._read_utilization()
            energy_trace = self._read_energy()

            self._count_trace_calls += 1
            self.logger.debug(
                f"Obtained energy trace no.{self._count_trace_calls} from {type(self).__name__ }:\n"
                f"utilization: {utilization_trace}\n"
                f"energy:     {energy_trace}"
            )

            for zone in utilization_trace:    
                #fmt: off
                self._consumed_energy += (
                    energy_trace[zone] * utilization_trace[zone]['sm']
                )
                # fmt: on

            await asyncio.sleep(self.sleep_interval)

    def shutdown(self):
        """
        The cleanup routine executed when the powergroup monitoring is finished
        or aborted by the user.
        """
        pynvml.nvmlShutdown()
