import pytest
import os
import csv
import asyncio
from collections import defaultdict
from datetime import datetime

try:
    import tensorflow as tf

    HAS_TF = True
except ImportError:
    HAS_TF = False
try:
    import torch

    HAS_TORCH = True
except ImportError:
    HAS_TORCH = False

from unittest.mock import MagicMock, patch, mock_open, PropertyMock
from emt.energy_meter import PowerGroup
from emt.utils import (
    TraceRecorder,
    CSVRecorder,
    TensorboardRecorder,
    TensorBoardWriterType,
)

TOLERANCE = 1e-9


class MockPowerGroup(PowerGroup):
    """Mock implementation of PowerGroup for testing."""

    def __init__(self, name):
        self.name = name
        self._energy_trace = {
            "trace_num": [1, 2, 3],
            "consumed_utilized_energy": [10, 20, 30],
            "consumed_utilized_energy_cumsum": [10, 30, 60],
            "norm_ps_util": [0.1, 0.2, 0.3],
        }

    @classmethod
    def is_available(cls):
        return True


@pytest.fixture
def mock_power_groups():
    return [MockPowerGroup("MockCPU"), MockPowerGroup("MockGPU")]


@pytest.fixture
def mock_csv_dir(tmp_path):
    """Fixture to create a temporary directory for mock CSV files."""
    return tmp_path


def test_trace_recorder_initialization(mock_csv_dir):
    recorder = TraceRecorder(location=mock_csv_dir, write_interval=60)
    assert recorder.trace_location == mock_csv_dir
    assert recorder.write_interval == 60
    assert recorder.power_groups == []


def test_trace_recorder_power_groups(mock_power_groups):
    recorder = TraceRecorder()
    recorder._power_groups = mock_power_groups
    assert recorder.power_groups == mock_power_groups


def test_csv_recorder_write_traces(mock_power_groups, mock_csv_dir):
    recorder = CSVRecorder(location=mock_csv_dir)
    recorder._power_groups = mock_power_groups

    mock_csv_data = mock_open()
    with patch("builtins.open", mock_csv_data), patch("os.makedirs"):
        recorder.write_traces()

    mock_csv_data.assert_called()
    handle = mock_csv_data()
    handle.write.assert_called()


def test_tensorboard_recorder_initialization():
    recorder = TensorboardRecorder()
    assert recorder.writer is None
    assert recorder.writer_type is None
    assert recorder.add_scalar is None


@pytest.mark.skipif(not HAS_TF, reason="Tensorflow not installed")
@patch("tensorflow.summary.create_file_writer")
def test_tensorboard_recorder_setup_tf(mock_tf_writer, mock_csv_dir):
    recorder = TensorboardRecorder()
    recorder.trace_location = mock_csv_dir
    recorder._setup_a_default_writer()

    assert recorder.writer_type.value == "tf"


@pytest.mark.skipif(not HAS_TORCH, reason="Pytorch not installed")
@patch("torch.utils.tensorboard.SummaryWriter")
def test_tensorboard_recorder_setup_torch(mock_torch_writer, mock_csv_dir):
    recorder = TensorboardRecorder()
    recorder.trace_location = mock_csv_dir
    with patch("importlib.import_module", side_effect=[ImportError, MagicMock()]):
        recorder._setup_a_default_writer()

    assert recorder.writer_type.value == "pytorch"


@pytest.mark.skipif(not HAS_TORCH, reason="Pytorch not installed")
def test_tensorboard_recorder_determine_writer_type(mock_csv_dir):
    from torch.utils.tensorboard import SummaryWriter

    writer = SummaryWriter(mock_csv_dir)
    recorder = TensorboardRecorder(writer)
    recorder._determine_writer_type()
    assert recorder.writer_type == TensorBoardWriterType.PYTORCH
    assert type(recorder.add_scalar) == type(writer.add_scalar)


@pytest.mark.skipif(not HAS_TORCH, reason="Pytorch not installed")
def test_tensorboard_recorder_write_traces(mock_power_groups, mock_csv_dir):
    from torch.utils.tensorboard import SummaryWriter

    writer = SummaryWriter(mock_csv_dir)
    mock_recorder = TensorboardRecorder(writer=writer)
    mock_recorder.power_groups = mock_power_groups
    with (
        patch.object(mock_recorder, "writer_type", TensorBoardWriterType.PYTORCH),
        patch.object(mock_recorder, "add_scalar") as mock_add_scalar,
    ):
        assert mock_recorder.writer_type == TensorBoardWriterType.PYTORCH
        mock_recorder.write_traces()
    assert mock_add_scalar.called
