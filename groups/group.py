
class EnergyGroup:
    def __init__(self, pid=None, rate:float=1):
        """_summary_
        This creates a virtual container consisting of one or more devices, The power measurements
        are accumulated over all the devices represented by this virtual power group. For example,
        an 'nvidia-gpu' power-group represents all nvidia-gpus and accumulates their energy
        consumption weighted by their utilization by the `pid` process-tree.  

        Args:
        pid:    The pid to be monitored, when `None` the power consumption of the 
                underlying devices is not attributed to any particular process but
                is reported as is without weighting.   

        rate:   How often the energy consumption is readout from the devices and the running 
                average in a second. The rate defines the number of measurements in a single 
                second of wall-time. 
        
        """ 
        self._pid = pid
        self._running_mean = 0.0
        self._samples = 0
        self._rate = rate

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

    def get_trace()  -> float:
        """_summary_
        Provides the weighted accumulated energy of all the devices within the virtual component.
        The energy is weighted by the utilization by the `pid` process-tree, when the pid is not
        tracked utilization is
        cannot be 

        Returns:
            Union[float, Mapping[str, float]]: _description_
        """
        ...

    def update(self) -> None :
        """_summary_
        Updates the running average of the total consumed energy attributable to a process in question. 
        """
        ...

    def consumed_energy(self) -> float:
        return list(zip(self._time, self.power))
    
