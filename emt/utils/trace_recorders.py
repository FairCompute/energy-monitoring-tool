import os
import csv
import asyncio
from enum import Enum
from typing import Collection
from datetime import datetime
from emt.power_groups import PowerGroup


class TensorBoardWriterType(Enum):
    TF = "tf"
    PYTORCH = "pytorch"


class TraceRecorder:
    def __init__(self, location: os.PathLike = None, write_interval: int = 50):
        """
        This class is responsible for recording the energy traces of the power groups. The traces are
        saved in the specified location at regular intervals, defined by `write_interval`.
        """
        self._power_groups = []
        # This is set in the __enter__ method of energy monitor
        self._location = location
        self.write_interval = write_interval

    @property
    def power_groups(self) -> Collection[PowerGroup]:
        return self._power_groups

    @power_groups.setter
    def power_groups(self, value: Collection[PowerGroup]):
        self._power_groups = value

    @property
    def trace_location(self):
        return self._location

    @trace_location.setter
    def trace_location(self, value: os.PathLike):
        self._location = value
        if not os.path.exists(self._location):
            os.makedirs(self._location)

    async def __call__(self):
        while True:
            await asyncio.sleep(self.write_interval)
            self.write_traces()

    def write_traces(self):
        raise NotImplementedError


class CSVRecorder(TraceRecorder):
    def __init__(self, location=None, write_interval=50):
        self.write_interval = write_interval
        super().__init__(location, write_interval)

    def write_traces(self):
        current_time = datetime.now().strftime("%Y%m%d_%H%M%S")
        for pg in self.power_groups:
            # Read log traces from the power group
            pg_name = pg.__class__.__name__
            energy_trace = pg.energy_trace

            filename = f"{pg_name}_{current_time}.csv"
            file_path = os.path.join(self._location, filename)
            # write data in csv format
            with open(file_path, mode="w", newline="") as file:
                writer = csv.writer(file)
                # Write the header
                writer.writerow(energy_trace.keys())
                # Write the data rows
                rows = zip(*energy_trace.values())  # Transpose the values
                writer.writerows(rows)


class TensorboardRecorder(TraceRecorder):
    def __init__(self, writer=None, location=None, write_interval=50):
        self.writer = writer
        self.writer_type = None
        self.add_scalar = None
        super().__init__(location, write_interval)
        if self.writer is not None:
            # if writer is passed then determine the writer type and add_scalar function
            self._determine_writer_type()

    def _setup_a_default_writer(self):
        try:
            import tensorflow as tf

            self.writer = tf.summary.create_file_writer(str(self.trace_location))
            self.writer_type = TensorBoardWriterType.TF
            self.add_scalar = tf.summary.scalar
        except ImportError:
            try:
                import torch
                from torch.utils.tensorboard import SummaryWriter

                self.writer = SummaryWriter(str(self.trace_location))
                self.writer_type = TensorBoardWriterType.PYTORCH
                self.add_scalar = self.writer.add_scalar
            except ImportError:
                self.writer = None
                raise ImportError(
                    """No suitable tensorboard library found (Tensorflow or Pytorch). 
                    try EMT without TensorboardRecorder()
                    """
                )

    def _determine_writer_type(self):
        # seu up the writer type and add_scalar dunctoin based on the type of writer passed
        if "tensorflow" in str(type(self.writer)).lower():
            import tensorflow as tf

            self.writer_type = TensorBoardWriterType.TF
            self.add_scalar = tf.summary.scalar
        elif "torch" in str(type(self.writer)).lower():
            self.writer_type = TensorBoardWriterType.PYTORCH
            self.add_scalar = self.writer.add_scalar
            print("torch add-scalar setup.")
        else:
            raise ValueError("Unsupported writer type provided.")

    def write_traces(self):
        # if no writer is passed then create a default writer based on the available libraries
        if self.writer is None:
            # this executes once, even if the writer is not passed initially,
            # from the second call onwards it will be set
            # necessary to setup here as otherwise the self.trace_location will not be set
            self._setup_a_default_writer()

        for pg in self.power_groups:
            pg_name = pg.__class__.__name__
            energy_trace = pg.energy_trace
            ps_util_var = "ps_util" if pg_name == "NvidiaGPU" else "norm_ps_util"
            plot_vars = [
                "consumed_utilized_energy",
                "consumed_utilized_energy_cumsum",
                ps_util_var,
            ]
            trace_num = energy_trace["trace_num"]
            for var in plot_vars:  # iterate over the variables
                for step, value in zip(
                    trace_num, energy_trace[var]
                ):  # iterate time steps

                    if self.writer_type == TensorBoardWriterType.TF:
                        with self.writer.as_default():
                            self.add_scalar(f"EMT: {pg_name}/{var}", value, step=step)
                    else:
                        self.add_scalar(
                            f"EMT: {pg_name}/{var}", value, global_step=step
                        )
        self.writer.flush()
