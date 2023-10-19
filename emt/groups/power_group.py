import psutil
from typing import Optional, Mapping
from functools import cached_property


class PowerGroup:
    def __init__(self, pid: Optional[int] = None, rate: float = 1):
        """_summary_
        This creates a virtual container consisting of one or more devices, The power measurements
        are accumulated over all the devices represented by this virtual power group. For example,
        an 'nvidia-gpu' power-group represents all nvidia-gpus and accumulates their energy
        consumption weighted by their utilization by the `pid` process-tree.

        Args:

        pid:    The pid to be monitored, when `None` the current process is monitored.

        rate:   How often the energy consumption is readout from the devices and the running
                average in a second. The rate defines the number of measurements in a single
                second of wall-time.

        """
        self._process = psutil.Process(pid=pid)
        self._running_mean = 0.0
        self._samples = 0
        self._rate = rate

    @cached_property
    def sleep_interval(self)->float:
        return (1.0/self._rate)
    
    @property
    def tracked_process(self):
        return self._process

    @property
    def devices():
        """_summary_
        List all devices/components tracked by this EnergyGroup
        """
        ...

    def is_available(self) -> bool:
        """_summary_
        A status flag, provides information if the virtual group is available for monitoring.
        When false a mechanism to trace a particular device type is not available.

        Returns:
            bool:   A status flag, provides information if the device is available for monitoring.
                    This includes if the necessary drivers for computing power and installed and
                    initialized. Each device class must provide a way to confirm this.
        """
        ...

    def commence() -> None:
        """_summary_
        This commence a periodic execution at the set rate:
          [energy_trace -> update_running_metric -> async_wait]
        """
        ...

    @property
    def consumed_energy(self) -> Mapping[str, float]:
        """_summary_
        This provides the total consumed energy, attributed to the process, per power-group.
        """
        ...
