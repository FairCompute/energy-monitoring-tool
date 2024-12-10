import pytest
from unittest.mock import patch, MagicMock, call
import os
import pandas as pd
import dash
import plotly.subplots as sp
import plotly.graph_objects as go
from emt.utils import GUI


@pytest.fixture
def mock_csv_dir(tmp_path):
    """Fixture to create a temporary directory for mock CSV files."""
    return tmp_path


@pytest.fixture
def gui(mock_csv_dir):
    """Fixture to create a GUI instance."""
    return GUI(csv_dir=str(mock_csv_dir), refresh_interval=5)


def test_init(gui, mock_csv_dir):
    """Test GUI initialization."""
    assert gui.csv_dir == str(mock_csv_dir)
    assert gui.refresh_interval == 5000  # Converted to milliseconds
    assert gui.host == "127.0.0.1"
    assert gui.port == 8052
    assert gui.data == {}


def test_setup_layout(gui):
    """Test layout setup."""
    with (
        patch("dash.html.Div") as mock_div,
        patch("dash.dcc.Dropdown") as mock_dropdown,
        patch("dash.dcc.Graph") as mock_graph,
    ):
        gui._setup_layout()
        assert gui.app.layout is not None
        assert mock_div.called
        assert mock_dropdown.called
        assert mock_graph.called


def test_read_new_csvs(gui, mock_csv_dir):
    """Test reading new CSV files."""
    with (
        patch(
            "os.listdir",
            return_value=["NvidiaGPU_1.csv", "RAPLSoC_1.csv", "invalid.txt"],
        ) as mock_listdir,
        patch("pandas.read_csv") as mock_read_csv,
    ):
        # Mock CSV file reading
        gpu_data = pd.DataFrame(
            {"trace_num": [1, 2], "consumed_utilized_energy": [10, 20]}
        )
        cpu_data = pd.DataFrame(
            {"trace_num": [1, 2], "consumed_utilized_energy": [5, 15]}
        )
        mock_read_csv.side_effect = [gpu_data, cpu_data]

        # Create dummy files
        for filename in mock_listdir():
            (mock_csv_dir / filename).touch()

        gui._read_new_csvs()

        assert not gui.data["gpu"].empty
        assert not gui.data["cpu"].empty
        assert gui.data["gpu"].equals(gpu_data)
        assert gui.data["cpu"].equals(cpu_data)


def test_no_csv_data(gui):
    """Test behavior when no CSV data is available."""
    with patch("os.listdir", return_value=[]):
        gui._read_new_csvs()
        assert gui.data["cpu"].empty
        assert gui.data["gpu"].empty


def test_plot_data(gui):
    """Test plotting data."""
    with (
        patch("plotly.subplots.make_subplots") as mock_make_subplots,
        patch("plotly.graph_objects.Scatter") as mock_scatter,
    ):
        # Mock data
        gui.data["cpu"] = pd.DataFrame(
            {
                "trace_num": [1, 2],
                "consumed_utilized_energy": [10, 20],
                "norm_ps_util": [50, 60],
                "consumed_utilized_energy_cumsum": [10, 30],
            }
        )
        gui.data["gpu"] = pd.DataFrame(
            {
                "trace_num": [1, 2],
                "consumed_utilized_energy": [15, 25],
                "avg_utilization": [70, 80],
                "consumed_utilized_energy_cumsum": [15, 40],
            }
        )
        gui._plot_data("Both")

        assert mock_make_subplots.called
        assert mock_scatter.call_count == 6


def test_stop(gui):
    """Test stopping the server."""

    with (patch("werkzeug.serving.make_server") as mock_make_server,):
        mock_make_server = MagicMock()
        gui.server = mock_make_server
        gui.stop()
        mock_make_server.shutdown.assert_called_once()
