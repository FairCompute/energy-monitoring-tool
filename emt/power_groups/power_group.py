import logging
import os
import psutil
from typing import Optional, Dict
from collections import defaultdict
from functools import cached_property
from copy import deepcopy

logger = logging.getLogger(__name__)


class PowerGroup:
    def __init__(self, pid: Optional[int] = None, rate: float = 1):
        """
        This creates a virtual container consisting of one or more devices, The power measurements
        are accumulated over all the devices represented by this virtual power group. For example,
        an 'nvidia-gpu' power-group represents all nvidia-gpus and accumulates their energy
        consumption weighted by their utilization by the `pid` process-tree.

        Args
        pid:            The pid to be monitored, when `None` the current process is monitored.
        rate:           How often the energy consumption is readout from the devices and the running
                        average in a second. The rate defines the number of measurements in a single
                        second of wall-time.
        """
        self._count_trace_calls = 0
        self._process = psutil.Process(pid=pid)
        self._consumed_energy = 0.0
        self._rate = rate
        self._energy_trace = defaultdict(list)
        
        # Cache for Process objects when using EMT_RELOAD_PROCS
        # Key: PID, Value: psutil.Process object (reused for cpu_percent tracking)
        self._process_cache: Dict[int, psutil.Process] = {}

        # Load configuration to get target energy unit (lazy import to avoid circular dependency)
        self._config = None
        self._target_energy_unit: Optional[str] = None
        self._internal_energy_unit = (
            "Joules"  # Internal energy is stored in Joules (base unit)
        )

    def _get_target_energy_unit(self) -> str:
        """
        Lazily load the target energy unit from configuration.
        """
        if self._target_energy_unit is None:
            try:
                # Import here to avoid circular dependency
                from emt.utils.config import load_config

                self._config = load_config()
                unit = self._config.get("measurement_units", {}).get("energy", "Joules")
                self._target_energy_unit = unit if isinstance(unit, str) else "Joules"
            except Exception as e:
                logger.warning(
                    f"Could not load configuration for unit conversion: {e}. Using default unit 'Joules'."
                )
                self._target_energy_unit = "Joules"

        # Return with fallback to ensure we always return a string
        return self._target_energy_unit or "Joules"

    def _convert_energy_to_target_unit(self, energy_joules: float) -> float:
        """
        Convert energy from Joules to the configured target unit.
        """
        target_unit = self._get_target_energy_unit()
        try:
            # Import here to avoid circular dependency
            from emt.utils.units import UnitConverter

            return UnitConverter.convert_energy(
                energy_joules, self._internal_energy_unit, target_unit
            )
        except Exception as e:
            logger.warning(
                f"Could not convert energy from {self._internal_energy_unit} to {target_unit}: {e}. Returning raw value."
            )
            return energy_joules

    @cached_property
    def sleep_interval(self) -> float:
        return 1.0 / self._rate

    @property
    def tracked_process(self):
        return self._process

    @tracked_process.setter
    def tracked_process(self, value):
        """
        This setter is mostly created for testing purpose
        """
        self._tracked_process = value

    def get_processes(self):
        """
        Get all processes under the current one.
        
        When EMT_RELOAD_PROCS is set, this returns fresh process list each time.
        Process objects are cached and reused so that psutil.cpu_percent() works
        correctly (it needs same object instance to compute delta between calls).
        """
        if not os.getenv("EMT_RELOAD_PROCS"):
            # Standard behavior - return simple list
            return [self.tracked_process] + self.tracked_process.children(recursive=True)
        
        # EMT_RELOAD_PROCS mode - discover new children but reuse cached Process objects
        current_pids = {self.tracked_process.pid}
        try:
            for child in self.tracked_process.children(recursive=True):
                current_pids.add(child.pid)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass
        
        # Update cache: add new processes, remove dead ones
        # Remove PIDs that are no longer children
        dead_pids = [pid for pid in self._process_cache if pid not in current_pids]
        for pid in dead_pids:
            del self._process_cache[pid]
        
        # Add new PIDs to cache (this is where cpu_percent warmup starts)
        for pid in current_pids:
            if pid not in self._process_cache:
                try:
                    proc = psutil.Process(pid)
                    # Warmup cpu_percent - first call establishes baseline
                    proc.cpu_percent(interval=None)
                    self._process_cache[pid] = proc
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    pass
        
        # Return cached Process objects in order (tracked first, then children)
        result = []
        if self.tracked_process.pid in self._process_cache:
            result.append(self._process_cache[self.tracked_process.pid])
        for pid in current_pids:
            if pid != self.tracked_process.pid and pid in self._process_cache:
                result.append(self._process_cache[pid])
        
        return result

    if os.getenv("EMT_RELOAD_PROCS"):
        # Also account for new child processes (using caching for cpu_percent accuracy)
        processes = property(get_processes)
    else:
        processes = cached_property(get_processes)

    @classmethod
    def is_available(cls) -> bool:
        """
        A status flag, provides information if the virtual group is available for monitoring.
        When false a mechanism to trace a particular device type is not available.

        Returns:
            bool:   A status flag, provides information if the device is available for monitoring.
                    This includes if the necessary drivers for computing power and installed and
                    initialized. Each device class must provide a way to confirm this.
        """
        ...

    async def commence(self) -> None:
        """
        This commence a periodic execution at a set rate:
          [get_energy_trace -> update_energy_consumption -> async_wait]
        """
        ...

    def shutdown(self) -> None:
        """
        This performs the any cleanup required at the shutdown of the PowerGroup monitoring.
        This includes stopping the periodic execution and flushing the energy trace.
        The shutdown is called when the context manager exits.
        """
        logger.info(f"shutting down {type(self).__name__} ")

    @property
    def consumed_energy(self) -> float:
        """
        This provides the total consumed energy, attributed to the process for the whole power-group.
        The energy is converted to the unit specified in the configuration file.
        """
        return self._convert_energy_to_target_unit(self._consumed_energy)

    @property
    def energy_unit(self) -> str:
        """
        Returns the energy unit that consumed_energy is reported in.
        """
        return self._get_target_energy_unit()

    @property
    def energy_trace(self) -> dict:
        """
        This provides the energy trace of the power group. The energy trace is a dictionary
        where the keys are the time-stamps and the values are the energy consumption at that time-stamp.
        Energy values in the trace are converted to the configured unit.
        On reading the energy trace, the buffer is flushed.
        """
        energy_trace = deepcopy(self._energy_trace)

        # Convert energy values in trace to configured unit
        if "consumed_energy" in energy_trace:
            try:
                converted_energies = [
                    self._convert_energy_to_target_unit(energy)
                    for energy in energy_trace["consumed_energy"]
                ]
                energy_trace["consumed_energy"] = converted_energies
            except Exception as e:
                logger.warning(
                    f"Could not convert energy trace values: {e}. Returning raw values."
                )

        self._energy_trace = defaultdict(list)
        return energy_trace
