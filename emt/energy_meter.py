import time
import os
import asyncio
import logging
import threading
from pathlib import Path
from threading import RLock
from typing import Collection, Mapping
from emt.power_groups import PowerGroup
from emt.utils import setup_logger, TraceRecorder, PGUtils


logger = logging.getLogger(__name__)


class EnergyMeter:

    def __init__(
        self,
        powergroups: Collection[PowerGroup],
        context_name: str,
        trace_recorders: Collection[TraceRecorder] = None,
    ):
        """
        EnergyMeter accepts a collection of PowerGroup objects and monitor them, logs their
        energy consumption at regular intervals. Each PowerGroup provides a set a task or a
        set of tasks, exposed via `commence` method of the powerGroup.  All such tasks are
        # gathered and asynchronously awaited by the energyMeter. Ideally, the run method
        should be executed in a separate background thread, so the asynchronous loop is not
        blocked by the cpu intensive work going on in the main thread.

        Args:
            power_groups (PowerGroup):      All power groups to be tracked by the energy meter.
            log_trace_path (os.PathLike):   The path to save the energy traces.
            context_name (str):             The name of the context, used for logging.
        """
        super().__init__()
        self._lock = RLock()
        self._monitoring = False
        self._concluded = False
        self._power_groups = powergroups
        self._shutdown_event = asyncio.Event()
        self._context_name = context_name
        self.trace_recorders = trace_recorders or []

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
        tasks = [asyncio.create_task(pg.commence()) for pg in self.power_groups]
        for trace_emitter in self.trace_recorders:
            tasks.append(asyncio.create_task(trace_emitter()))
        tasks.append(asyncio.create_task(self._shutdown_asynchronous()))
        await asyncio.gather(*tasks)

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
            logger.info(f"Initiated Energy Monitoring -- {self._context_name}.")
            asyncio.run(self._run_tasks_asynchronous())
        except asyncio.CancelledError:
            logger.info(
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
            logger.error(
                "Attempting to conclude monitoring before commencement.\n"
                "It is illegal to conclude before commencement. Shutting Down!"
            )
            raise RuntimeError("Cannot conclude monitoring before commencement!")

        logger.info(f"ShutDown requested -- _{self._context_name}.")
        with self._lock:
            self._concluded = True
            self._shutdown_event.set()
            self._monitoring = False
            # after the shutdown event is set, request all trace
            #  emitters to emit any remaining traces.
            for trace_emitter in self.trace_recorders:
                trace_emitter.write_traces()

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
    def __init__(
        self,
        *,
        name: str = "unnamed_context",
        trace_recorders: Collection[TraceRecorder] | TraceRecorder = None,
    ):
        self.context_name = name
        setup_logger(logger)

        self._trace_recorders = self._normalize_trace_recorders(trace_recorders)

        if not self._trace_recorders:
            logger.warning(
                "No trace emitters provided. Energy traces will not be saved."
            )
        else:
            self._validate_trace_recorders(self._trace_recorders)

        self.pg_objs = None

    def _normalize_trace_recorders(self, trace_recorders):
        if trace_recorders is None:
            return []
        if isinstance(trace_recorders, TraceRecorder):
            return [trace_recorders]
        return trace_recorders

    def _validate_trace_recorders(self, trace_recorders):
        if not all(isinstance(tr, TraceRecorder) for tr in trace_recorders):
            raise ValueError("Invalid trace emitters provided.")

    def __enter__(self):
        logger.info(f"EMT context manager invoked - {self.context_name} ...")
        self.start_time = time.time()
        pg_utils = PGUtils()
        # get available powergroups
        self.pg_objs = pg_utils.get_available_pgs()
        # log powergroup info in a tabular format
        logger.info("\n" + pg_utils.get_pg_table())
        # set trace emitters
        for trace_emitter in self._trace_recorders:
            trace_emitter.power_groups = self.pg_objs
            # get log file handler location
            location = Path(logger.handlers[0].baseFilename).parent
            if trace_emitter.__class__.__name__ == "CSVRecorder":
                location = os.path.join(location, "csv_traces", self.context_name)
            elif trace_emitter.__class__.__name__ == "TensorboardRecorder":
                location = os.path.join(location, "tfevents", self.context_name)

            trace_emitter.trace_location = trace_emitter.trace_location or location

        energy_meter = EnergyMeter(
            powergroups=self.pg_objs,
            context_name=self.context_name,
            trace_recorders=self._trace_recorders,
        )
        self.energy_meter = energy_meter
        # run EnergyMonitoring as a separate thread
        self.energy_meter_thread = threading.Thread(
            name="EnergyMonitoringThread", target=energy_meter.run
        )
        self.energy_meter_thread.start()

        time.sleep(1)
        return self.energy_meter

    def __exit__(self, *_):
        self.energy_meter.conclude()
        self.energy_meter_thread.join()
        execution_time = time.time() - self.start_time
        logger.info(
            f"{self.context_name}: Execution time: {execution_time:.2f} seconds"
        )
        logger.info(
            f"{self.context_name}: Total energy consumption: {self.energy_meter.total_consumed_energy:.2f} J"
        )
        logger.info(
            f"{self.context_name}: Power group energy consumptions: {self.energy_meter.consumed_energy}"
        )
