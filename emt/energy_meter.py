import time
import asyncio
import logging
import threading
from threading import RLock
from typing import Collection, Mapping

# from emt import setup_logger
import emt
from emt.power_groups import PowerGroup
from emt import power_groups


class EnergyMeter:
    def __init__(
        self,
        powergroups: Collection[PowerGroup],
        logging_interval: int = 900,
    ):
        """
        EnergyMeter accepts a collection of PowerGroup objects and monitor them, logs their
        energy consumption at regular intervals. Each PowerGroup provides a set a task or a
        set of tasks, exposed via `commence` method of the powerGroup.  All such tasks are
        # gathered and asynchronously awaited by the energyMeter. Ideally, the run method
        should be executed in a separate background thread, so the asynchronous loop is not
        blocked by the cpu intensive work going on in the main thread.

        Args:
            power_groups (PowerGroup):  All power groups to be tracked by the energy meter.
            logging_interval (int):     The energy reporting interval in seconds, by default
                                        the meter writes the logs every 15 mins.
        """
        super().__init__()
        self._lock = RLock()
        self._monitoring = False
        self._concluded = False
        self._logging_interval = logging_interval
        self._power_groups = powergroups
        self._shutdown_event = asyncio.Event()
        self.logger = logging.getLogger(__name__)

    @property
    def power_groups(self):
        return self._power_groups

    @property
    def monitoring(self):
        with self._lock:
            return self._monitoring

    @property
    def concluded(self):
        with self._lock:
            return self._concluded

    async def _shutdown_asynchronous(self):
        """
        Waits asynchronously for the shutdown event. Once the event is set, a
        `asyncio.CancelledError` exception is raised. The exception  is handled
        by the `run` method to breakout of the asyncio.run loop.
        """
        await self._shutdown_event.wait()
        raise asyncio.CancelledError

    async def _run_tasks_asynchronous(self):
        """
        This creates tasks, schedule them for asynchronous execution, and the
        wait until all tasks are completed. These tasks are commonly designed
        to run infinitely at a given rate.
        """
        tasks = [asyncio.create_task(pG.commence()) for pG in self.power_groups]
        task_shutdown = asyncio.create_task(self._shutdown_asynchronous())
        await asyncio.gather(*tasks, task_shutdown)

    def run(self):
        """
        The entrypoint for the monitoring routines. This method collects and spins off the
        `commence` method for each PowerGroup object.  All commenced tasks are executed
        asynchronously, i.e. the task are scheduled to execute at the earliest possibility.
        However, when the main thread is performing a cpu intensive task, the asynchronous
        loop might get blocked, therefore it is recommended to execute this method in a
        seperate independent thread.
        """
        with self._lock:
            self._shutdown_event.clear()
            self._monitoring = True
        try:
            self.logger.info("Initiated Energy Monitoring.")
            asyncio.run(self._run_tasks_asynchronous())
        except asyncio.CancelledError:
            self.logger.info(
                " Shutting Down! \nMonitoring Concluded by the EnergyMeter.\n\n"
            )
        return 0

    def conclude(self):
        """
         The entrypoint for the monitoring routines. This method collects and spins off the
        `commence` method for each PowerGroup object.  All commenced tasks are executed
        asynchronously, i.e. the task are scheduled to execute at the earliest possibility.
        However, when the main thread is performing a cpu intensive task, the asynchronous
        loop might get blocked, therefore it is recommended to execute this method in a
        seperate independent thread.
        """
        if not self.monitoring:
            self.logger.error(
                "Attempting to conclude monitoring before commencement.\n"
                "It is illegal to conclude before commencement. Shutting Down!"
            )
            raise RuntimeError("Cannot conclude monitoring before commencement!")

        self.logger.info("ShutDown requested.")
        with self._lock:
            self._concluded = True
            self._shutdown_event.set()
            self._monitoring = False

    @property
    def total_consumed_energy(self) -> float:
        total_consumed_energy = 0.0
        for power_group in self.power_groups:
            total_consumed_energy += power_group.consumed_energy
        return total_consumed_energy

    @property
    def consumed_energy(self) -> Mapping[str, float]:
        consumed_energy = {
            type(power_group).__name__: round(power_group.consumed_energy, 2)
            for power_group in self.power_groups
        }
        return consumed_energy


class EnergyMonitor:

    def get_powergroup_types(self, module):
        candidates = [
            getattr(module, name)
            for name in dir(module)
            if isinstance(getattr(module, name), type)
        ]
        pg_types = filter(lambda x: issubclass(x, PowerGroup), candidates)
        return list(pg_types)

    def __enter__(self):
        if not logging.getLogger("emt").hasHandlers():
            emt.setup_logger()
        powergroup_types = self.get_powergroup_types(power_groups)
        # check for available power_groups
        available_powergroups = list(
            filter(lambda x: x.is_available(), powergroup_types)
        )
        # instantiate only available powergroups
        powergroups = [pgt() for pgt in available_powergroups]
        # TODO: Check if no power groups are selected then raise warning and exit

        # Create a separate thread and start it.
        energy_meter = EnergyMeter(powergroups=powergroups)
        self.energy_meter_thread = threading.Thread(
            name="EnergyMonitoringThread", target=lambda: energy_meter.run()
        )
        self.energy_meter_thread.start()
        self.energy_meter = energy_meter
        time.sleep(1)
        return self.energy_meter

    def __exit__(self, *_):
        self.energy_meter.conclude()
        self.energy_meter_thread.join()
