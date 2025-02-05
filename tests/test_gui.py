import pytest
from unittest.mock import patch, MagicMock, call
import os
import sys
import pandas as pd
from collections import defaultdict
import dash
import dash_bootstrap_components as dbc
import plotly.subplots as sp
import plotly.graph_objects as go
from emt.utils.gui import GUI, main


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
    assert gui._data == {}


def test_setup_layout(gui):
    """Test layout setup."""
    with (
        patch("dash.html.Div") as mock_div,
        patch("dash.dcc.Dropdown") as mock_dropdown,
        patch("dash_bootstrap_components.Accordion") as mock_accordion,
    ):
        gui._setup_layout()
        assert gui.app.layout is not None
        assert mock_div.called
        assert mock_dropdown.called
        assert mock_accordion.called


def test_read_new_csvs(gui, mock_csv_dir):
    """Test reading new CSV files and updating aggregated data."""

    # Mock GPU and CPU data
    gpu_data = pd.DataFrame({"trace_num": [1, 2], "consumed_utilized_energy": [10, 20]})
    cpu_data = pd.DataFrame({"trace_num": [1, 2], "consumed_utilized_energy": [5, 15]})

    # Mock file structure and data
    with (
        patch(
            "os.listdir",
            side_effect=[
                ["context_1"],
                ["NvidiaGPU_1.csv", "RAPLSoC_1.csv", "invalid.txt"],
            ],
        ),
        patch("pandas.read_csv", side_effect=[gpu_data, cpu_data]) as mock_read_csv,
    ):
        # Set the CSV directory for the GUI
        gui.csv_dir = mock_csv_dir

        # Run the function
        gui._read_new_csvs()

        # Assertions
        assert "context_1" in gui._data.keys()
        assert gui._data["context_1"]["gpu"].equals(gpu_data)
        assert gui._data["context_1"]["cpu"].equals(cpu_data)

        # Check if pandas.read_csv was called correctly
        assert mock_read_csv.call_count == 2


def test_no_csv_data(gui):
    """Test behavior when no CSV data is available."""
    with patch("os.listdir", return_value=[]):
        gui._read_new_csvs()
        # No data should be read
        assert gui._data == {}


def test_get_plot_name(gui):
    """Test getting plot names."""

    assert gui._get_plot_name("Both") == [
        "CPU Energy Traces and Utilization",
        "GPU Energy Traces and Utilization",
        "CPU Energy CumSum",
        "GPU Energy CumSum",
    ]
    assert gui._get_plot_name("CPU") == [
        "CPU Energy Traces and Utilization",
        "CPU Energy CumSum",
    ]
    assert gui._get_plot_name("GPU") == [
        "GPU Energy Traces and Utilization",
        "GPU Energy CumSum",
    ]


def test_plot_data(gui):
    """Test plotting data."""
    with (
        patch("plotly.subplots.make_subplots") as mock_make_subplots,
        patch("plotly.graph_objects.Scatter") as mock_scatter,
    ):
        # Mock data
        mock_data = defaultdict(pd.DataFrame)
        mock_data["cpu"] = pd.DataFrame(
            {
                "trace_num": [1, 2],
                "consumed_utilized_energy": [10, 20],
                "norm_ps_util": [50, 60],
                "consumed_utilized_energy_cumsum": [10, 30],
            }
        )
        mock_data["gpu"] = pd.DataFrame(
            {
                "trace_num": [1, 2],
                "consumed_utilized_energy": [15, 25],
                "ps_util": [70, 80],
                "consumed_utilized_energy_cumsum": [15, 40],
            }
        )
        gui._plot_data(mock_data, "Both")
        assert mock_make_subplots.called
        assert mock_scatter.call_count == 6


def test_plot_data_scopes(gui):
    with patch.object(GUI, "_plot_data") as mock_plot_data:
        gui._scope_names = ["context_1", "context_2"]
        gui._plot_data_scopes("Both")
    assert mock_plot_data.call_count == 2


def test_stop(gui):
    """Test stopping the server."""

    with (patch("werkzeug.serving.make_server") as mock_make_server,):
        mock_make_server = MagicMock()
        gui.server = mock_make_server
        gui.stop()
        mock_make_server.shutdown.assert_called_once()


def test_main():
    # Simulate command-line arguments
    test_args = [
        "app.py",
        "--csv_dir",
        "./test_logs/",
        "--refresh_interval",
        "10",
        "--host",
        "0.0.0.0",
        "--port",
        "8080",
    ]

    with (
        patch.object(sys, "argv", test_args),
        patch("emt.utils.gui.GUI") as mock_gui_class,
        # patch.object("GUI.run", return_value=None) as mock_gui_run,
    ):
        mock_gui_instance = MagicMock()
        mock_gui_class.return_value = mock_gui_instance
        mock_gui_class.return_value.run = MagicMock()
        main()  # Call the main function

        # Assertions to verify correct behavior
        mock_gui_class.assert_called_once_with(
            "./test_logs/", refresh_interval=10, host="0.0.0.0", port=8080
        )
        mock_gui_instance.run.assert_called_once()
