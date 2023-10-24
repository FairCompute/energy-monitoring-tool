import os
import psutil
import logging
from typing import Optional, Mapping
from functools import cached_property


class PowerGroup:
    def __init__(self, pid: Optional[int] = None, rate: float = 1,
                 log_file:os.PathLike ="energy_meter.log",    
                 logging_level: int = logging.NOTSET,):
        """
        This creates a virtual container consisting of one or more devices, The power measurements
        are accumulated over all the devices represented by this virtual power group. For example,
        an 'nvidia-gpu' power-group represents all nvidia-gpus and accumulates their energy
        consumption weighted by their utilization by the `pid` process-tree.

        Args:

        pid:                        The pid to be monitored, when `None` the current process is monitored.

        rate:                       How often the energy consumption is readout from the devices and the running
                                    average in a second. The rate defines the number of measurements in a single
                                    second of wall-time.
        
        log_file (os.PathLike):     The file path where logs are written by the monitor.

        logging_level (int):        The log level determines what sort of information is
                                    logged, when not set indicates that ancestor loggers
                                    are to be consulted to determine the level. If that
                                    still resolves to NOTSET, then all events are logged.
        """

        self._process = psutil.Process(pid=pid)
        self._consumed_energy = 0.0
        self._rate = rate

        # Configure logging to write to a log file with a custom format
        log_format = (
            "%(asctime)s - %(name)s - %(threadName)s - %(levelname)s - %(message)s"
        )
        logging.basicConfig(filename=log_file, level=logging_level, format=log_format)
        self.logger = logging.getLogger(type(self).__name__)

    @cached_property
    def sleep_interval(self)->float:
        return (1.0/self._rate)
    
    @property
    def tracked_process(self):
        return self._process

    @property
    def devices():
        """
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
    def consumed_energy(self) -> float:
        """_summary_
        This provides the total consumed energy, attributed to the process for the whole power-group.
        """
        return self._consumed_energy
