import os
import re
import asyncio
import psutil
import numpy as np
from pathlib import Path
from typing import Collection, Mapping
from functools import cached_property, reduce
from emt import PowerGroup


class DeltaReader:
    """
    This class provides a method that provides the delta between the previously
    recorded value and the new value read by RAPL from the MSR registers of the CPU.
    The delta is returned in joules.
    """

    def __init__(self, file_path: os.PathLike, num_trails: int = 3) -> None:
        """
        Args:
            file_path (os.PathLike):    The file path to the rapl energy counter.
            num_trails (int):           The number of trails to read the energy counter.
                                        The multiple trials provide a mechanism to avoid reading
                                        from a overflown counter. When all attempts fail, the
                                        energy delta is set to zero and a warning is logged.

        Returns:
            float:  The delta between the previously recorded value and the new value read by
                    RAPL from the MSR registers of the CPU. The delta is returned in joules.
        """

        self._num_trails = num_trails
        self._previous_value = np.nan
        self._file = file_path

    def __call__(self):
        """
        This call method provides the delta between the previously
        recorded value and the new value read by RAPL from the MSR registers of the CPU.
        The delta is returned in joules.

        Returns:   The delta in consumed energy between the previously and the current call.
                   The delta energy is obtained by RAPL from the MSR registers of the CPU
                   (in micro-jouled) and is scaled to joules for the return value.
        """
        value = np.nan
        for k_trail in range(self._num_trails):
            delta = 0.0
            with open(Path(self._file, "energy_uj"), "r") as f:
                value = int(f.read())

            # if there is reference value, compute delta
            if not np.isnan(self._previous_value):
                _delta = float(value - self._previous_value) * 1e-6
                if _delta >= 0:
                    delta = _delta
                    break

            self.logger.warning(
                f"Energy counter overflow detected for: \n{self._file}"
            ) if k_trail >= (self._num_trails - 1) and delta < 0 else None
        self._previous_value = value
        return delta


class IntelCPU(PowerGroup):
    """
    This is a specialized PowerGroup for Intel CPUs. It provides a mechanism to track the energy
    consumption of the CPU and its sub-components (cores, dram, igpu). The energy consumption is
    obtained from the RAPL (Running Average Power Limit) interface of the CPU. The RAPL interface
    is available on Intel CPUs since the Sandy Bridge micro-architecture.

    The energy consumption is reported in `joules` when the `consumed_energy` property is accessed.
    The energy consumption is accumulated over the duration of the monitoring period, which starts
    when the `commence` method is called and ends when the async task is cancelled externally.
    """

    RAPL_DIR = "/sys/class/powercap/"

    def __init__(
        self,
        zone_pattern: str = "intel-rapl",
        excluded_zones: Collection = ("psys",),
        **kwargs,
    ):
        """
        Args:
            zone_pattern (str):             The pattern to match the RAPL zone name.
                                            The default value is `intel-rapl`.
            excluded_zones (Collection):    A collection of zone names to be excluded from monitoring.
                                            The default is `("psys",)`, this excludes the power supply
                                            zone from the observed zones.
            **kwargs:                       Additional arguments be passed to the `PowerGroup`.
        """

        # by default a rate 5Hz is used to collect energy_trace.
        kwargs.update({"rate": kwargs.get("rate", 10)})
        super().__init__(**kwargs)

        # Get intel-rapl power zones/domains
        zones = [
            Path(self.RAPL_DIR, zone)
            for zone in filter(lambda x: ":" in x, os.listdir(self.RAPL_DIR))
        ]

        # filter out zones that do not match zone_pattern
        zones = list(
            filter(lambda zone: not re.fullmatch(zone_pattern, str(zone)), zones)
        )

        # Get components for each zone (if available);
        #  Not all processors expose components.
        components = [
            list(filter(lambda x: len(x.stem.split(":")) > 2, Path(zone).rglob("*")))
            for zone in zones
        ]

        self.zones_count = len(zones)
        self._zones = []
        self._devices = []

        for zone, devices in zip(zones, components):
            with open(Path(zone, "name"), "r") as f:
                name = f.read().strip()
            if name not in excluded_zones:
                self._zones.append(zone)
                self._devices.append(devices)

        self.processes = [self.tracked_process] + self.tracked_process.children(
            recursive=True
        )

        # create delta energy_readers for each types
        self.zone_readers = [DeltaReader(_zone) for _zone in self._zones]
        self.core_readers = [
            DeltaReader(_comp)
            for device in self._devices
            for _comp in device
            if any(keyword in _comp for keyword in ["ram", "dram"])
        ]
        self.dram_readers = [
            DeltaReader(_comp)
            for device in self._devices
            for _comp in device
            if any(keyword in _comp for keyword in ["cores", "cpu"])
        ]
        self.igpu_readers = [
            DeltaReader(_comp)
            for device in self._devices
            for _comp in device
            if "gpu" in _comp
        ]

    @cached_property
    def zones(self):
        """
        Get zone names, for all the tracked zones from RAPL
        """

        def get_zone_name(zone):
            with open(Path(zone, "name"), "r") as f:
                name = f.read().strip()
            return name

        return list(map(get_zone_name, self._zones))

    def __repr__(self) -> str:
        """
        The string representation of the IntelCPU PowerGroup
        """
        return str(self.zones)

    @cached_property
    def devices(self):
        """
        Get devices names, for all the tracked zones from RAPL
        """

        def get_device_name(zone, devices):
            with open(Path(zone, "name"), "r") as f:
                zone_name = f.read().strip()
            device_name = None
            for device in devices:
                with open(Path(device, "name"), "r") as f:
                    device_name = f.read().strip()
                device_name = f"{zone_name}/{device_name}"
            return device_name

        return list(map(get_device_name, self._zones, self._devices))

    def is_available(self):
        """A check for availability of RAPL interface"""
        return os.path.exists(self.RAPL_DIR) and bool(os.listdir(self.RAPL_DIR))

    def _read_energy(self) -> Mapping[str, float]:
        """
        Reports the accumulated energy consumption of the tracked devices types. The readers are
        created in the constructor and are called to obtain the energy delta. Reader of each type
        are called in a loop and the energy delta is accumulated.

        Returns (float):    A map of accumulated energy consumption (in joules) for each device
                            type since the last call to this method.
        """
        energy_zones = 0.0
        energy_cores = 0.0
        energy_dram = 0.0
        energy_igpu = 0.0

        # accumulate energy delta from zones
        for _reader in self.zone_readers:
            energy_zones += _reader()
        # accumulate energy delta from drams
        for _reader in self.dram_readers:
            energy_dram += _reader()
        # accumulate energy delta form cores
        for _reader in self.core_readers:
            energy_cores += _reader()
        # accumulate energy delta from igpus
        for _reader in self.igpu_readers:
            energy_igpu += _reader()

        return {
            "zones": energy_zones,
            "cores": energy_cores,
            "dram": energy_dram,
            "igpu": energy_igpu,
        }

    def _read_utilization(self) -> Mapping[str, float]:
        """
        Reports the  utilization of the CPUs and DRAM by the tracked processes. The utilization
        is obtained from the `psutil` library, which reports the utilization as a percentage of 
        the cpu_time offered to the processes compared to overall cpu_time.

        The cpu utilization is a number between 0 and 1, where 1 is 100%. Similarly, the dram
        utilization is a number between 0 and 1, where 1 is 100%.
        """
        cpu_utilization = np.nan
        memory_utilization = np.nan
        try:
            cpu_utilization = reduce(
                lambda x, y: x + y, (ps.cpu_percent() for ps in self.processes)
            )
            memory_utilization = reduce(
                lambda x, y: x + y, (ps.memory_percent() for ps in self.processes)
            )
        except (psutil.NoSuchProcess, psutil.ZombieProcess):
            pass
        
        return {
            "cpu": cpu_utilization/psutil.cpu_count(),
            "dram":memory_utilization,
        }

    async def commence(self) -> None:
        """
        This commence a periodic execution at a set rate:
            [get_energy_trace -> update_energy_consumption -> async_wait]

        The periodic execution is scheduled at the rate dictated by `self.sleep_interval`, during the
        instantiation. The energy consumption is updated using the `_read_energy` and `_read_utilization`
        methods. The method credits energy consumption to the tracked processes by weighting the energy
        trace, obtained from the zones and the devices, by the utilization of the devices by the processes.
        """

        while True:
            utilization_trace = self._read_utilization()
            energy_trace = self._read_energy()

            # system_compute_utilization = psutil.cpu_percent()
            # system_memory_utilization = psutil.virtual_memory().percent
            #  # relative utilization
            # relative_utilization_compute = utilization_trace / system_compute_utilization \
            #     if system_compute_utilization > 0.0 else 0.0
            # relative_utilization_memory = memory_utilization_process / system_memory_utilization \
            #    if system_memory_utilization > 0.0 else 0.0

            self._count_trace_calls += 1
            self.logger.debug(
                f"Obtained energy trace no.{self._count_trace_calls} from {type(self).__name__ }:\n"
                f"Utilization: {utilization_trace}\n"
                f"Energy:     {energy_trace}"
            )

            if self.dram_readers:
                # fmt:off
                self._consumed_energy += (
                    (energy_trace['zones'] - energy_trace['dram']) * utilization_trace['cpu'] +
                      energy_trace['dram'] * utilization_trace['dram']
                ) 
            else:
                self._consumed_energy += (
                    energy_trace["zones"] * utilization_trace["cpu"]
                )
                # fmt: on

            await asyncio.sleep(self.sleep_interval)
