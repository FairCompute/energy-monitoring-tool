import os
import re
import asyncio
import psutil
import numpy as np
from pathlib import Path
from typing import Collection, Mapping
from functools import cached_property, reduce
from .power_group import PowerGroup


class IntelCPU(PowerGroup):
    # RAPL Literature:
    # https://www.researchgate.net/publication/322308215_RAPL_in_Action_Experiences_in_Using_RAPL_for_Power_Measurements

    RAPL_DIR = "/sys/class/powercap/"

    def __init__(
        self,
        zone_pattern: str = "intel-rapl",
        excluded_zones: Collection = ("psys",),
        **kwargs,
    ):
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

    @cached_property
    def zones(self):
        """Get zone names, for all the tracked zones from RAPL"""

        def get_zone_name(zone):
            with open(Path(zone, "name"), "r") as f:
                name = f.read().strip()
            return name

        return list(map(get_zone_name, self._zones))

    def __str__(self) -> str:
        return str(self.zones)

    @cached_property
    def devices(self):
        """Get devices names, for all the tracked zones from RAPL"""

        def get_device_name(zone, devices):
            with open(Path(zone, "name"), "r") as f:
                zone_name = f.read().strip()
            device_name = None
            for device in devices:
                with open(Path(device, "name"), "r") as f:
                    device_name = f.read().strip()
                device_name  = f"{zone_name}/{device_name}"
            return device_name

        return list(map(get_device_name, self._zones, self._devices))

    def is_available(self):
        return os.path.exists(self.RAPL_DIR) and bool(os.listdir(self.RAPL_DIR))

    def _read_energy(self):
        """_summary_
        Reports the energy consumption since the last reaadout for each package.
        When subcomponents/devices level tracking available it is reported under
        the `devices` key of the parent package.
        """
        energy_zones = 0
        energy_cores = np.nan
        energy_dram = np.nan
        energy_igpu = np.nan

        for zone_path in self._zones:
            with open(Path(zone_path, "energy_uj"), "r") as f:
                energy_zones += float(f.read())*1E-6

        for component in self._devices:
            if component:
                with open(Path(component, "energy_uj"), "r") as f:
                    value = float(f.read())*1E-6
                    if any(keyword in component for keyword in ["ram", "dram"]):
                        energy_dram = (
                            value if np.isnan(energy_dram) else (value + energy_dram)
                        )
                    if any(keyword in component for keyword in ["cores", "cpu"]):
                        energy_cores = (
                            value if np.isnan(energy_cores) else (value + energy_cores)
                        )
                    if any(keyword in component for keyword in ["gpu"]):
                        energy_igpu = (
                            value if np.isnan(energy_igpu) else (energy_igpu + value)
                        )
        return {
            "zones": energy_zones,
            "cores": energy_cores,
            "dram": energy_dram,
            "igpu": energy_igpu,
        }

    def _read_utilization(self) -> Mapping[str, float]:
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
            "cpu": (cpu_utilization / psutil.cpu_count())/100.0,
            "dram": memory_utilization/100.0,
        }

    async def commence(self) -> None:
        # tasks = self._measurement_tasks()
        while True:
            utilization_trace = self._read_utilization()
            energy_trace = self._read_energy()

            if np.isnan(energy_trace['dram']):
                self._consumed_energy += (energy_trace['zones'] * utilization_trace['cpu'])
            else:
                # fmt:off
                self._consumed_energy += (
                    (energy_trace['zones'] - energy_trace['dram']) * utilization_trace['cpu'] +
                      energy_trace['dram'] * utilization_trace['dram']
                )
                # fmt: on
            await asyncio.sleep(self.sleep_interval)
