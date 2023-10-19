import os
import asyncio
import logging
from threading import RLock
from typing import Collection
from emt.groups import PowerGroup


class EnergyMeter:
    def __init__(
        self,
        power_groups: Collection[PowerGroup],
        log_file: os.PathLike = "energy_meter.log",
        logging_interval: int = 900,
        logging_level: int = logging.NOTSET,
    ):
        """
        EnergyMeter accepts a collection of PowerGroup objects and monitor them, logs their
        energy consumption at regular intervals. Each PowerGroup provides a set a task or a
        set of tasks, exposed via `commence` method of the powerGroup.  All such tasks are
        gathered and asynchronouly awaited by the energyMeter. Ideally, the run method shoud
        be executed in a sepearate background thread, so the asyncronous loop is not blocked
        by the cpu intesive work going on in the main thread.

        Args:
            power_groups (PowerGroup):  All power groups to be tracked by the energy meter.

            log_file (os.PathLike):     The file path where logs are written by the monitor.

            logging_interval (int):     The energy reporting interval in secods, by default
                                        the meter writes the logs everyu 15 mins.

            logging_level (int):        The log level determines what sort of information is
                                        logged, when not set indicates that ancestor loggers
                                        are to be consulted to determine the level. If that
                                        still resolves to NOTSET, then all events are logged.
        """
        super().__init__()
        self._lock = RLock()
        self._monitoring = False
        self._concluded = False
        self._logging_interval = logging_interval
        self._power_groups = power_groups
        self._shutdown_event = asyncio.Event()

        # Configure logging to write to a log file with a custom format
        log_format = (
            "%(asctime)s - %(name)s - %(threadName)s - %(levelname)s - %(message)s"
        )
        logging.basicConfig(filename=log_file, level=logging_level, format=log_format)
        self.logger = logging.getLogger("EnergyMonitor")

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
        Waits asyncronously for the shutdown envent. Once the event is set, a
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
        `commmence` method for each PowerGroup object.  All commenced tasks are executed
        asynchronously, i.e. the task are scheduled to execute at the earliest possibility.
        However, when the main thread is performing a cpu intensive task, the asynchronous
        loop might get blocked, therefore it is recommended to execute this method in a
        seperate independent thread.
        """
        with self._lock:
            self._shutdown_event.clear()
            self._monitoring = True
        try:
            asyncio.run(self._run_tasks_asynchronous())
        except asyncio.CancelledError:
            self.logger.info('Monitoring is shutdown & concluded by the EnergyMeter')
        return 0

    def conclude(self):
        """
         The entrypoint for the monitoring routines. This method collects and spins off the
        `commmence` method for each PowerGroup object.  All commenced tasks are executed
        asynchronously, i.e. the task are scheduled to execute at the earliest possibility.
        However, when the main thread is performing a cpu intensive task, the asynchronous
        loop might get blocked, therefore it is recommended to execute this method in a
        seperate independent thread.
        """
        if not self.monitoring:
            raise RuntimeError("cannot conclude monitoring before commencement!")

        with self._lock:
            self._concluded = True
            self._shutdown_event.set()
            self._monitoring = False
