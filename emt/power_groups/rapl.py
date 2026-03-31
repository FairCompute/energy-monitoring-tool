import os
import re
import time
import asyncio
import psutil
import logging
from pathlib import Path
from typing import Collection, Mapping
from functools import cached_property
from emt.power_groups.power_group import PowerGroup

logger = logging.getLogger(__name__)


def extract_components(zone_path: Path, all_zone_paths: list) -> list:
    """Extract sub-components of a zone from the list of all available zone paths.

    Sub-components are RAPL domains that belong to the given parent zone and
    whose names both (a) start with the parent zone name followed by a colon
    (e.g., a sub-component of ``intel-rapl:0`` starts with ``intel-rapl:0:``)
    and (b) contain more than one colon in total (e.g., ``intel-rapl:0:0``).

    This includes all descendant sub-domains, not just immediate children. For
    example, for the parent zone ``intel-rapl:0``, both ``intel-rapl:0:0`` and
    ``intel-rapl:0:0:0`` are considered sub-components as long as they appear in
    *all_zone_paths*.

    This function intentionally does **not** rely on filesystem traversal so
    that it works regardless of whether the powercap entries are represented as
    a flat collection of symlinks or as a nested directory tree.

    Args:
        zone_path:       Path to a top-level RAPL zone (e.g.,
                         ``/sys/class/powercap/intel-rapl:0``).
        all_zone_paths:  All available zone paths, including sub-components.

    Returns:
        A list of paths that are sub-components (descendants) of *zone_path*.
    """
    return [
        comp
        for comp in all_zone_paths
        if comp.name.count(":") > 1 and comp.name.startswith(zone_path.name + ":")
    ]


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
        self._previous_value = None
        self._file = file_path
        self.track_energy_traces = None

    def __call__(self):
        """
        This call method provides the delta between the previously
        recorded value and the new value read by RAPL from the MSR registers of the CPU.
        The delta is returned in joules.

        Returns:   The delta in consumed energy between the previously and the current call.
                   The delta energy is obtained by RAPL from the MSR registers of the CPU
                   (in micro-joules) and is scaled to joules for the return value.
        """
        value = None
        delta = 0.0  # Initialize delta before the loop
        for _ in range(self._num_trails):
            delta = 0.0
            with open(Path(self._file, "energy_uj"), "r") as f:
                value = int(f.read())

            # if there is a previous reference value, compute delta
            if self._previous_value is not None:
                _delta = float(value - self._previous_value) * 1e-6
                if _delta >= 0:
                    # break the loop of the new reading is greater or equal to the previous one
                    # else read again
                    delta = _delta
                    break
        # after first ever call this value is set and then for consecutive calls its used and reset
        self._previous_value = value
        if delta < 0:
            logger.warning(f"Energy counter overflow detected for: \n{self._file}")
            return 0.0
        return delta


class RAPLSoC(PowerGroup):
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

        # by default a rate 10Hz is used to collect energy_trace.
        kwargs.update({"rate": kwargs.get("rate", 10)})
        super().__init__(**kwargs)

        # Get intel-rapl power zones/domains
        # Only include top-level package zones (e.g., intel-rapl:0, intel-rapl:1)
        # Skip subdomains (e.g., intel-rapl:0:0, intel-rapl:0:1) as package already includes them
        all_zones = [
            Path(self.RAPL_DIR, zone)
            for zone in filter(lambda x: ":" in x, os.listdir(self.RAPL_DIR))
        ]

        # Filter to only keep package-level zones (exactly one colon, e.g., intel-rapl:0)
        # This prevents double-counting as package energy = core + uncore
        zones = [
            zone
            for zone in all_zones
            if zone.name.count(":") == 1  # Only top-level: intel-rapl:N
        ]

        # Get components for each zone (if available);
        #  Not all processors expose components.
        # Use all_zones (not zones) so that sub-components, which reside as
        # siblings in the flat powercap directory, are found correctly.
        components = [extract_components(zone, all_zones) for zone in zones]

        self.zones_count = len(zones)
        self._zones = []
        self._components = []

        for zone, zone_comps in zip(zones, components):
            with open(Path(zone, "name"), "r") as f:
                name = f.read().strip()
            if name not in excluded_zones:
                self._zones.append(zone)
                self._components.append(zone_comps)

        # create delta energy_readers for each types
        self.zone_readers = [DeltaReader(_zone) for _zone in self._zones]
        self.core_readers = [
            DeltaReader(_comp)
            for device in self._components
            for _comp in device
            if any(keyword in str(_comp) for keyword in ["cores", "cpu"])
        ]
        self.dram_readers = [
            DeltaReader(_comp)
            for device in self._components
            for _comp in device
            if any(keyword in str(_comp) for keyword in ["ram", "dram"])
        ]
        self.igpu_readers = [
            DeltaReader(_comp)
            for device in self._components
            for _comp in device
            if "gpu" in str(_comp)
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

    def __str__(self) -> str:
        """
        The string representation of the PowerGroup
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

        return list(map(get_device_name, self._zones, self._components))

    @classmethod
    def is_available(cls):
        """A check for availability of RAPL interface"""
        try:
            return bool(os.path.exists(cls.RAPL_DIR) and bool(os.listdir(cls.RAPL_DIR)))
        except OSError:
            return False

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
        calculates the relative utilization of the CPUs and DRAM of the tracked processes.
        process_utilization = (process_cpu_percent / cpu_count * total_cpu_percent).
        The utilization is obtained from the `psutil` library, which reports the utilization as a percentage of
        the cpu_time offered to the processes compared to overall cpu_time.
        It is normalized to get the utilization wrt. other active processes that contribute to energy consumption.

        Returns:
            dict: cpu and memory utilizations


        """
        utilizations = {
            "dram": 0.0,
            "ps_util": 0.0,
            "cpu_util": 0.0,
            "norm_ps_util": 0.0,
        }
        try:
            # Get system CPU first (this establishes a consistent time reference)
            # Using interval=None returns delta since last call
            utilizations["cpu_util"] = psutil.cpu_percent(interval=None)

            # Process level cpu utilization for all processes
            # Note: cpu_percent(interval=None) uses time since last call for that Process object
            ps_cpu_util = sum(ps.cpu_percent(interval=None) for ps in self.processes)
            ps_mem_util = sum(ps.memory_percent() for ps in self.processes)

            # Dividing by cpu count normalizes the utilization to [0-100]% for each process
            cpu_count = (
                psutil.cpu_count() or 1
            )  # Default to 1 if cpu_count() returns None
            utilizations["ps_util"] = ps_cpu_util / cpu_count

            # Handle the case when total CPU usage is zero
            if utilizations["cpu_util"] > 0:
                norm_ps_util = utilizations["ps_util"] / utilizations["cpu_util"]
                # Cap at 1.0 to prevent over-attribution due to timing mismatches
                # between process and system CPU measurements
                utilizations["norm_ps_util"] = min(norm_ps_util, 1.0)
            else:
                # When total CPU usage is zero, set normalized process utilization to 0
                # This prevents division by zero and handles idle system scenarios
                utilizations["norm_ps_util"] = 0.0
                logger.debug(
                    "Total CPU usage is zero, setting normalized process utilization to 0."
                )

            utilizations["dram"] = ps_mem_util
        except psutil.NoSuchProcess:
            logger.error("Process utilization could not be found.")

        return utilizations

    async def commence(self) -> None:
        """
        This commence a periodic execution at a set rate:
            [get_energy_trace -> update_energy_consumption -> async_wait]

        The periodic execution is scheduled at the rate dictated by `self.sleep_interval`, during the
        instantiation. The energy consumption is updated using the `_read_energy` and `_read_utilization`
        methods. The method credits energy consumption to the tracked processes by weighting the energy
        trace, obtained from the zones and the devices, by the utilization of the devices by the processes.
        """

        # Warm up psutil counters so first sample is not artificially zero.
        try:
            psutil.cpu_percent(interval=None)
            for ps in self.processes:
                ps.cpu_percent(interval=None)
        except psutil.NoSuchProcess:
            logger.warning("Warmup failed: process not found.")

        start_wall = time.time()

        while True:
            start_time = time.perf_counter()
            energy_trace = self._read_energy()
            measurement_time = time.perf_counter() - start_time

            utilization_trace = self._read_utilization()

            if self.dram_readers:
                # fmt:off
                consumed_utilized_energy = (
                    (energy_trace['zones'] - energy_trace['dram']) * utilization_trace['norm_ps_util'] +
                      energy_trace['dram'] * utilization_trace['dram']
                ) 
            else:
                consumed_utilized_energy = (
                    energy_trace["zones"] * utilization_trace["norm_ps_util"]
                )
                # fmt: on
            # consume energy is sum of all the utilized consumed energies across the intervals
            self._consumed_energy += consumed_utilized_energy

            # add trace info
            now = time.time()
            self._energy_trace["trace_num"].append(self._count_trace_calls)
            self._energy_trace["timestamp"].append(round(now, 3))
            self._energy_trace["elapsed_s"].append(round(now - start_wall, 3))
            self._energy_trace["proc_count"].append(len(self.processes))
            self._energy_trace["measurement_time"].append(round(measurement_time, 4))
            self._energy_trace["ps_util"].append(round(utilization_trace["ps_util"], 2))
            self._energy_trace["cpu_util"].append(
                round(utilization_trace["cpu_util"], 4)
            )
            self._energy_trace["norm_ps_util"].append(
                round(utilization_trace["norm_ps_util"], 2)
            )
            if self.dram_readers:
                self._energy_trace["total_energy_cpu"].append(
                    round(energy_trace["zones"] - energy_trace["dram"], 2)
                )
                self._energy_trace["total_energy_dram"].append(
                    round(energy_trace["dram"], 2)
                )
                self._energy_trace["utilization_dram"].append(
                    round(utilization_trace["dram"], 2)
                )
            else:
                self._energy_trace["total_energy_cpu"].append(
                    round(energy_trace["zones"], 2)
                )
            self._energy_trace["consumed_utilized_energy"].append(
                round(consumed_utilized_energy, 2)
            )
            self._energy_trace["consumed_utilized_energy_cumsum"].append(
                round(self._consumed_energy, 2)
            )
            self._count_trace_calls += 1
            await asyncio.sleep(self.sleep_interval)
