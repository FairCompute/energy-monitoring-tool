import os
import pandas as pd
from collections import defaultdict
import threading
import dash
from dash import Dash, dcc, html
from dash.dependencies import Input, Output
import dash_bootstrap_components as dbc
import plotly.express as px
import plotly.subplots as sp
import plotly.graph_objects as go
from flask import Flask
import argparse
from werkzeug.serving import make_server


class GUI:
    def __init__(self, csv_dir, refresh_interval=5, host="127.0.0.1", port=8052):
        """
        Initialize the UI class.

        Args:
        - csv_dir (str): Directory containing CSV files.
        - refresh_interval (int): Interval (seconds) to refresh plots.
        """
        self.csv_dir = csv_dir
        self.refresh_interval = refresh_interval * 1000  # convert to miliseconds
        self.app = Dash(
            __name__,
            server=Flask(__name__),
            external_stylesheets=[dbc.themes.BOOTSTRAP],
        )
        self.host = host
        self.port = port
        self.server = None
        self._data = defaultdict(dict)
        self._shutdown_event = threading.Event()
        # Incase gui, is invoked before the directory is created
        os.makedirs(self.csv_dir, exist_ok=True)
        self._scope_names = [scope_name for scope_name in os.listdir(self.csv_dir)]

        self._setup_layout()

    def _setup_layout(self):
        """Define the Dash app layout."""

        # Device selection dropdown
        device_dropdown_section = html.Div(
            [
                dcc.Dropdown(
                    options=[
                        {"label": "CPU", "value": "CPU"},
                        {"label": "GPU", "value": "GPU"},
                        {"label": "Both", "value": "Both"},
                    ],
                    value="Both",  # Default device
                    id="device-dropdown",
                    style={
                        "width": "50%",
                        "marginBottom": "20px",
                        "backgroundColor": "#8a8988",
                    },
                )
            ]
        )
        # Dynamic Accordions of Graphs for each scope
        scope_graphs = dbc.Accordion(
            [
                dbc.AccordionItem(
                    [
                        dcc.Graph(
                            id=f"{scope}-graph",  # Unique ID for each scope's graph
                            responsive=True,
                            style={"height": "800px", "margin": "0 auto"},
                        )
                    ],
                    title=f"EMT Scope: {scope}",
                    style={"backgroundColor": "#8a8988", "color": "black"},
                )
                for scope in self._scope_names
            ],
            flush=True,
            always_open=True,
        )
        # Define overall layout
        self.app.layout = html.Div(
            children=[
                html.H1(
                    "EMT Energy Traces",
                    style={"textAlign": "center", "backgroundColor": "#f5be3d"},
                ),
                html.Hr(),
                # Device selection dropdown
                device_dropdown_section,
                # Vertical gap matching the dropdown menu height
                html.Div(style={"height": "40px"}),
                # Interval for updating
                dcc.Interval(
                    id="interval-update", interval=self.refresh_interval, n_intervals=0
                ),
                html.Div(
                    id="status",
                    children="Waiting for data...",
                    style={"textAlign": "right", "marginBottom": "5px"},
                ),
                scope_graphs,
            ],
            style={"backgroundColor": "#8a8988", "minHeight": "100vh"},
        )

        # Callbacks
        @self.app.callback(
            # Dynamically generate an Output for each scope's graph + a status message
            [Output(f"{scope}-graph", "figure") for scope in self._scope_names]
            + [Output("status", "children")],
            [
                Input("interval-update", "n_intervals"),  # Interval for refreshing data
                Input("device-dropdown", "value"),  # Dropdown device selection
            ],
        )
        def update_plot(n, device_option):
            """Update the plot with new data."""

            # Read new CSVs and append data
            self._read_new_csvs()

            # Check for empty data
            if not any(
                self._data[scope][device].size
                for scope in self._data
                for device in ["cpu", "gpu"]
            ):
                # If all scope data is empty
                return [{} for _ in self._scope_names] + ["No data available yet."]

            # Generate plots
            figs = self._plot_data_scopes(device_option)
            return figs + [f"Data refreshed at iteration {n}"]

    def _read_new_csvs(self):
        """Read new CSV files and update the aggregated data."""

        self._scope_names = os.listdir(self.csv_dir)
        for scope_name in self._scope_names:
            file_dir = os.path.join(self.csv_dir, scope_name)
            scope_data = defaultdict(pd.DataFrame)
            for file_name in sorted(os.listdir(file_dir)):
                file_path = os.path.join(file_dir, file_name)
                if file_name.endswith(".csv"):
                    try:
                        if file_name.startswith("NvidiaGPU"):
                            gpu_df = pd.read_csv(file_path)
                            scope_data["gpu"] = pd.concat(
                                [scope_data["gpu"], gpu_df],
                                ignore_index=True,
                            )
                        elif file_name.startswith("RAPLSoC"):
                            cpu_df = pd.read_csv(file_path)
                            scope_data["cpu"] = pd.concat(
                                [scope_data["cpu"], cpu_df],
                                ignore_index=True,
                            )
                    except Exception as e:
                        print(f"Error reading {file_path}: {e}")
            self._data[scope_name] = scope_data

    def _get_plot_name(self, device_option="Both"):
        """
        Returns plot names based on selected devices and number of plots

        Args:
            device_option (str, optional): Defaults to "Both".
        """
        if device_option == "Both":
            plot_names = [
                "CPU Energy Traces and Utilization",
                "GPU Energy Traces and Utilization",
                "CPU Energy CumSum",
                "GPU Energy CumSum",
            ]
        else:
            plot_names = [
                f"{device_option} Energy Traces and Utilization",
                f"{device_option} Energy CumSum",
            ]
        return plot_names

    def _plot_data(self, data, device_option="Both"):
        """
        Generate GPU and CPU energy trace plots using Plotly with layout based on device option.
        Args:
        - device_option (str):  one of 'CPU', 'GPU', or 'Both'
        """

        # Determine layout
        cols = 1 if device_option in ["CPU", "GPU"] else 2
        rows = 2
        # Create subplots
        fig = sp.make_subplots(
            rows=rows,
            cols=cols,
            horizontal_spacing=0.05,
            vertical_spacing=0.1,
            subplot_titles=self._get_plot_name(device_option),
        )
        plot_mode = "lines+markers"
        # plot CPU data
        if device_option in ["CPU", "Both"]:
            fig.add_trace(
                go.Scatter(
                    x=data["cpu"]["trace_num"],
                    y=data["cpu"]["consumed_utilized_energy"],
                    mode=plot_mode,
                    name="CPU: Consumed Energy (J)",
                    showlegend=True,
                ),
                row=1,
                col=1,
            )
            fig.add_trace(
                go.Scatter(
                    x=data["cpu"]["trace_num"],
                    y=data["cpu"]["norm_ps_util"],
                    mode=plot_mode,
                    name="CPU: Normalized Process Utilization (%)",
                    showlegend=True,
                ),
                row=1,
                col=1,
            )
            fig.add_trace(
                go.Scatter(
                    x=data["cpu"]["trace_num"],
                    y=data["cpu"]["consumed_utilized_energy_cumsum"],
                    mode=plot_mode,
                    name="CPU: CumSum Energy (J)",
                    marker_color="green",
                    showlegend=True,
                ),
                row=2,
                col=1,
            )
        # plot CPU data
        if device_option in ["GPU", "Both"]:
            fig.add_trace(
                go.Scatter(
                    x=data["gpu"]["trace_num"],
                    y=data["gpu"]["consumed_utilized_energy"],
                    mode=plot_mode,
                    name="GPU: Consumed Energy (J)",
                    showlegend=True,
                ),
                row=1,
                col=2 if device_option == "Both" else 1,
            )
            fig.add_trace(
                go.Scatter(
                    x=data["gpu"]["trace_num"],
                    y=data["gpu"]["ps_util"],
                    mode=plot_mode,
                    name="GPU: Process Utilization (%)",
                    showlegend=True,
                ),
                row=1,
                col=2 if device_option == "Both" else 1,
            )
            fig.add_trace(
                go.Scatter(
                    x=data["gpu"]["trace_num"],
                    y=data["gpu"]["consumed_utilized_energy_cumsum"],
                    mode=plot_mode,
                    name="GPU: CumSum Energy (J)",
                    marker_color="green",
                    showlegend=True,
                ),
                row=2,
                col=2 if device_option == "Both" else 1,
            )
        # Add axes labels
        x_axis_label = "Trace Number"
        y_axis_label = "Values"
        fig["layout"]["xaxis"]["title"] = x_axis_label
        fig["layout"]["xaxis2"]["title"] = x_axis_label
        fig["layout"]["yaxis"]["title"] = y_axis_label
        fig["layout"]["yaxis2"]["title"] = y_axis_label
        if device_option == "Both":
            fig["layout"]["xaxis3"]["title"] = x_axis_label
            fig["layout"]["xaxis4"]["title"] = x_axis_label
            fig["layout"]["yaxis3"]["title"] = y_axis_label
            fig["layout"]["yaxis4"]["title"] = y_axis_label

        # Customize layout
        fig.update_layout(
            height=1000,  # Define overall plot height
            width=1600,  # Define overall plot width
            # title_text="4x4 Subplots with Enhanced Styling",
            # title_x=0.5,  # Center title
            showlegend=True,  # Enable legend
        )

        return fig

    def _plot_data_scopes(self, device_option="Both") -> list:
        """
        Generate plots for all scopes_names
        Args:
        - device_option (str):  one of 'CPU', 'GPU', or 'Both'
        Returns:
        - list of figs
        """

        # Validate device option
        if device_option not in ["CPU", "GPU", "Both"]:
            raise ValueError(
                f"Invalid option- {device_option} ! Choose from 'CPU', 'GPU', or 'Both'."
            )
        scope_figs = []
        for scope_name in self._scope_names:
            # generate figures for each scope_name
            fig = self._plot_data(self._data[scope_name], device_option)
            scope_figs.append(fig)
        return scope_figs

    def run(self):
        """Run the Dash server."""
        # Define the host and port
        self.server = make_server(self.host, self.port, self.app.server)

        # Print the server address explicitly
        print(f"Dash server is running at http://{self.host}:{self.port}")

        # Start serving requests
        self.server.serve_forever()

    def stop(self):
        """Stop the Dash server gracefully."""
        if self.server:
            self.server.shutdown()  # Werkzeug server shutdown


def main():
    parser = argparse.ArgumentParser(description="Run the GUI for CSV traces.")
    parser.add_argument(
        "--csv_dir",
        type=str,
        default="./logs/exp_name/csv_traces/",
        help="Directory containing CSV trace files.",
    )
    parser.add_argument(
        "--refresh_interval",
        type=int,
        default=5,
        help="Refresh interval for the GUI in seconds.",
    )
    parser.add_argument(
        "--host", type=str, default="127.0.0.1", help="Host address for the GUI."
    )
    parser.add_argument(
        "--port", type=int, default=8052, help="Port number for the GUI."
    )

    args = parser.parse_args()

    gui = GUI(
        args.csv_dir,
        refresh_interval=args.refresh_interval,
        host=args.host,
        port=args.port,
    )
    gui.run()


if __name__ == "__main__":
    main()
