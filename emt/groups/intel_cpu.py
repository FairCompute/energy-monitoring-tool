import os
import re
import asyncio
import logging
from pathlib import Path
import numpy as np
from typing import  Collection, Mapping
from functools import cached_property
from .power_group import PowerGroup

logging.basicConfig()
logger = logging.getLogger(__name__)
logger.setLevel(logging.DEBUG)



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
        self._consumed_energy = np.nan

        for zone, devices in zip(zones, components):
            with open(Path(zone, "name"), "r") as f:
                name = f.read().strip()
            if name not in excluded_zones:
                self._zones.append(zone)
                self._devices.append(devices)

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
            device_name = "cpu"
            for device in devices:
                with open(Path(device, "name"), "r") as f:
                    device_name = f.read().strip()
            return f"{zone_name}/{device_name}"

        return list(map(get_device_name, self._zones, self._devices))

    def is_available(self):
        return os.path.exists(self.RAPL_DIR) and bool(os.listdir(self.RAPL_DIR))

    async def _read_energy(self):
        """_summary_
        Reports the energy consumption since the last reaadout for each package.
        When subcomponents/devices level tracking available it is reported under
        the `devices` key of the parent package. 
        """
        for zone_path, components in zip(self.zones, self.devices):
            pass
            # with open(Path(zone_path, "energy_uj"), "r") as f:
            #     package_energy = int(f.read())

            # for component_path in components:
            #     with open(Path(component_path, "energy_uj"), "r") as f:
            #         component_energy = int(f.read())
        return {}

    async def _read_utilization(self) -> Mapping[str, float]:
        ps_list = self.tracked_process.children(recursive=True)
        return {}

    def _measurement_tasks(self) -> Collection[asyncio.Task]:
        # get all active child processes
       
        task_utilization = asyncio.create_task(self._read_utilization())
        task_metering = asyncio.create_task(self._read_energy()
        )
        return (task_utilization, task_metering)
    
    async def commence(self) -> None:
        tasks = self._measurement_tasks()
        while True:
            [utilization_trace, energy_trace] = await asyncio.gather(*tasks)
            print(f'utilization_trace : {utilization_trace}')
            print(f'energy_trace: {energy_trace}')
            await asyncio.sleep(self.sleep_interval)

    async def conclude(self):
        print("something")