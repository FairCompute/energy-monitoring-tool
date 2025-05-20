import logging
import psutil
import logging
from typing import Optional
from collections import defaultdict
from functools import cached_property
from copy import deepcopy


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
        self.logger = logging.getLogger(__name__)
        self._energy_trace = defaultdict(list)

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
        set.logger.info(f"shutting down {type(self).__name__} ")

    @property
    def consumed_energy(self) -> float:
        """
        This provides the total consumed energy, attributed to the process for the whole power-group.
        """
        return self._consumed_energy

    @property
    def energy_trace(self) -> dict:
        """
        This provides the energy trace of the power group. The energy trace is a dictionary
        where the keys are the time-stamps and the values are the energy consumption at that time-stamp.
        On reading the energy trace, the buffer is flushed.
        """
        energy_trace = deepcopy(self._energy_trace)
        self._energy_trace = defaultdict(list)
        return energy_trace
