import os
import re
from pathlib import Path
from typing import List, Tuple
from .group import EnergyGroup


class IntelCPU(EnergyGroup):
    # RAPL Literature:
    # https://www.researchgate.net/publication/322308215_RAPL_in_Action_Experiences_in_Using_RAPL_for_Power_Measurements

    RAPL_DIR = "/sys/class/powercap/"
    
    def __init__(self):
        # Get amount of intel-rapl folders
        zones = list(filter(lambda x: ":" in x, os.listdir(self.RAPL_DIR)))
        self.zones_count = len(zones)
        self._rapl_zones = []
        self._zones = [] # human readable names for _rapl_zones
        zones_pattern = re.compile("intel-rapl:.")

        for zone in zones:
            if re.fullmatch(zones_pattern, zone):
                with open(os.path.join(self.RAPL_DIR, zone, "name"), "r") as f:
                    name = f.read().strip()
                if name != "psys":
                    self._rapl_zones.append(zone)
        
        def convert_rapl_zone_name(zone_name):
            if re.match(zones_pattern, zone_name):
                zone_name = "cpu:" + zone_name[-1]
            return zone_name
            
        # transform rapl_modules names to human readable names
        self._zones = list(map(convert_rapl_zone_name, zones))

    
    @property
    def devices(self):
        """Returns the name of all RAPL sub-zones"""
        return self._zones

    def is_available(self):
        return os.path.exists(self.RAPL_DIR) and bool(os.listdir(self.RAPL_DIR))
    
    @staticmethod
    def _read_energy(path):
        with open(os.path.join(path, "energy_uj"), "r") as f:
            return int(f.read())

    def get_trace(self) -> Tuple[List[float], List[float]]:
        rapl_zones = [Path(self.RAPL_DIR, zone) for zone in self._rapl_zones]
        energy_trace:List[float] = [self._read_energy(zone) for zone in rapl_zones]
        return energy_trace

    def __str__(self) -> str:
        return str(self.zones)