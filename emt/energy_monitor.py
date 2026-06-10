import time
import asyncio
import logging
import os
import threading
from threading import RLock
from typing import Collection, Mapping, Optional
from emt.power_groups import PowerGroup, get_available_pgs, get_pg_table
from emt.utils import setup_logger, TraceRecorder

# Public API
__all__ = ["EnergyMonitorCore", "EnergyMonitor"]

logger = logging.getLogger(__name__)

setup_logger(
    logger,
    logging_level=logging.INFO,
    mode="w",
    to_std_streams=True,
)


class EnergyMonitorCore:

    def __init__(
        self,
        powergroups: Collection[PowerGroup],
        context_name: str,
        trace_recorders: Optional[Collection[TraceRecorder]] = None,
    ):
        """
        EnergyMonitor accepts a collection of PowerGroup objects and monitor them, logs their
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

    @property
    def energy_unit(self) -> str:
        """
        Returns the energy unit that total_consumed_energy is reported in.
        """
        # Assuming all power groups use the same energy unit
        if self._power_groups:
            return next(iter(self._power_groups)).energy_unit
        return "Joules"

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
            type(power_group).__name__: power_group.consumed_energy
            for power_group in self.power_groups
        }
        return consumed_energy


class EnergyMonitor:
    def __init__(
        self,
        *,
        name: str = "unnamed_context",
        trace_recorders: Optional[Collection[TraceRecorder] | TraceRecorder] = None,
        startup_delay_s: float = 1.0,
        pid: Optional[int] = None,
        rate: Optional[float] = None,
    ):
        self.context_name = name
        setup_logger(logger)
        self._startup_delay_s = max(0.0, float(startup_delay_s))
        self._pid = pid
        self._rate = rate

        self._trace_recorders = self._normalize_trace_recorders(trace_recorders)

        if not self._trace_recorders:
            logger.warning(
                "No trace emitters provided. Energy traces will not be saved."
            )
        else:
            self._validate_trace_recorders(self._trace_recorders)

        self.pg_objs = None
        self._rust_backend = None

    def _normalize_trace_recorders(self, trace_recorders):
        if trace_recorders is None:
            return []
        if isinstance(trace_recorders, TraceRecorder):
            return [trace_recorders]
        return trace_recorders

    def _validate_trace_recorders(self, trace_recorders):
        if not all(isinstance(tr, TraceRecorder) for tr in trace_recorders):
            raise ValueError("Invalid trace emitters provided.")

    def _try_rust_backend(self):
        """Attempt to create and start the Rust-based monitor backend."""
        if self._trace_recorders:
            logger.debug(
                "Trace recorders require the Python backend; skipping Rust backend."
            )
            return None

        try:
            from emt._rust import RustMonitor

            kwargs = {
                "name": self.context_name,
                "pid": self._pid if self._pid is not None else os.getpid(),
            }
            if self._rate is not None:
                kwargs["rate"] = self._rate
            backend = RustMonitor(**kwargs)
            backend.commence()
            return backend
        except ImportError:
            logger.debug("Rust backend not available, falling back to Python.")
            return None
        except Exception as e:
            logger.warning(
                f"Rust backend failed to start ({e}), falling back to Python."
            )
            return None

    def __enter__(self):
        logger.info(f"EMT context manager invoked - {self.context_name} ...")
        self.start_time = time.time()

        # Try Rust backend first (no Python threads needed)
        self._rust_backend = self._try_rust_backend()
        if self._rust_backend is not None:
            if self._startup_delay_s:
                time.sleep(self._startup_delay_s)
            return self

        # Pure-Python fallback
        pg_kwargs = {"pid": self._pid} if self._pid is not None else {}
        if self._rate is not None:
            pg_kwargs["rate"] = self._rate
        self.pg_objs = get_available_pgs(**pg_kwargs)
        logger.info("\n" + get_pg_table())

        for trace_emitter in self._trace_recorders:
            trace_emitter.power_groups = self.pg_objs

        energy_meter = EnergyMonitorCore(
            powergroups=self.pg_objs,
            context_name=self.context_name,
            trace_recorders=self._trace_recorders,
        )
        self.energy_meter = energy_meter
        self.energy_meter_thread = threading.Thread(
            name="EnergyMonitoringThread", target=energy_meter.run
        )
        self.energy_meter_thread.start()

        if self._startup_delay_s:
            time.sleep(self._startup_delay_s)
        return self

    def __exit__(self, *_):
        if self._rust_backend is not None:
            self._rust_backend.shutdown()
            execution_time = time.time() - self.start_time
            logger.info(
                f"{self.context_name}: Execution time: {execution_time:.2f} seconds"
            )
            logger.info(
                f"{self.context_name}: Total energy consumption: "
                f"{self._rust_backend.total_consumed_energy:.2f} Joules"
            )
            logger.info(
                f"{self.context_name}: Power group energy consumptions: {self.consumed_energy}"
            )
            return

        # Pure-Python path
        self.energy_meter.conclude()
        self.energy_meter_thread.join()
        execution_time = time.time() - self.start_time
        logger.info(
            f"{self.context_name}: Execution time: {execution_time:.2f} seconds"
        )
        logger.info(
            f"{self.context_name}: Total energy consumption: {self.energy_meter.total_consumed_energy:.2f} {self.energy_meter.energy_unit}"
        )
        logger.info(
            f"{self.context_name}: Power group energy consumptions: {self.energy_meter.consumed_energy}"
        )

    @property
    def total_consumed_energy(self) -> float:
        """Total energy consumed across all devices (Joules)."""
        if hasattr(self, "_rust_backend") and self._rust_backend is not None:
            return self._rust_backend.total_consumed_energy
        if hasattr(self, "energy_meter"):
            return self.energy_meter.total_consumed_energy
        return 0.0

    @property
    def consumed_energy(self) -> Mapping[str, float]:
        """Per-power-group energy breakdown."""
        if hasattr(self, "_rust_backend") and self._rust_backend is not None:
            return self._rust_consumed_energy_by_power_group()
        if hasattr(self, "energy_meter"):
            return self.energy_meter.consumed_energy
        return {}

    def _rust_consumed_energy_by_power_group(self) -> Mapping[str, float]:
        device_energy = dict(self._rust_backend.consumed_energy)
        consumed_energy = {
            "RAPLSoC": device_energy.get("cpu", 0.0) + device_energy.get("dram", 0.0)
        }
        gpu_energy = device_energy.get("gpu", 0.0)
        gpu_available = bool(
            getattr(self._rust_backend, "gpu_available", gpu_energy > 0.0)
        )
        if gpu_available or gpu_energy > 0.0:
            consumed_energy["NvidiaGPU"] = gpu_energy
        return consumed_energy

    @property
    def trace_recorders(self):
        """Expose trace recorders for backwards compatibility."""
        if hasattr(self, "energy_meter") and self.energy_meter is not None:
            return self.energy_meter.trace_recorders
        return self._trace_recorders

    @property
    def energy_unit(self) -> str:
        """Returns the energy unit."""
        if hasattr(self, "energy_meter") and self.energy_meter is not None:
            return self.energy_meter.energy_unit
        return "Joules"
