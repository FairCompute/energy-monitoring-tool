import os
import numpy as np
import pandas as pd
from collections import defaultdict
from datetime import datetime
import threading

import dash
from dash import Dash, dcc, html
from dash.dependencies import Input, Output
import plotly.express as px
import plotly.subplots as sp
import plotly.graph_objects as go
from flask import Flask
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
        self.app = Dash(__name__, server=Flask(__name__))
        self.host = host
        self.port = port
        self.server = None
        self.data = defaultdict(pd.DataFrame)
        self._shutdown_event = threading.Event()
        self._setup_layout()

    def _setup_layout(self):
        """Define the Dash app layout."""
        self.app.layout = html.Div(
            children=[
                html.H1("EMT Energy Traces", style={"textAlign": "center"}),
                html.Hr(),
                # Drop down menu
                html.Div(
                    [
                        dcc.Dropdown(
                            options=["CPU", "GPU", "Both"],
                            value="Both",
                            id="device-dropdown",
                            style={"width": "50%"},
                        )
                    ]
                ),
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
                dcc.Graph(
                    id="dynamic-plot",
                    responsive=True,
                    style={"height": "800px", "margin": "0 auto"},
                ),
            ]
        )

        # Callbacks
        @self.app.callback(
            [Output("dynamic-plot", "figure"), Output("status", "children")],
            [
                Input("interval-update", "n_intervals"),
                Input("device-dropdown", "value"),
            ],
        )
        def update_plot(n, device_option):
            """Update the plot with new data."""
            # Read new CSVs and append data
            self._read_new_csvs()

            if self.data["cpu"].empty and self.data["gpu"].empty:
                return {}, "No data available yet."

            # Generate plots
            fig = self._plot_data(device_option)
            return fig, f"Data refreshed at iteration {n}"

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

    def _plot_data(self, device_option="Both"):
        """
        Generate GPU and CPU energy trace plots using Plotly with layout based on device option.

        :param gpu_data: DataFrame for GPU data
        :param cpu_data: DataFrame for CPU data
        :param device_option: str, one of 'CPU', 'GPU', or 'Both'
        """
        # Validate device option
        if device_option not in ["CPU", "GPU", "Both"]:
            raise ValueError(
                f"Invalid option- {device_option} ! Choose from 'CPU', 'GPU', or 'Both'."
            )

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
                    x=self.data["cpu"]["trace_num"],
                    y=self.data["cpu"]["consumed_utilized_energy"],
                    mode=plot_mode,
                    name="CPU: Consumed Energy (J)",
                    showlegend=True,
                ),
                row=1,
                col=1,
            )
            fig.add_trace(
                go.Scatter(
                    x=self.data["cpu"]["trace_num"],
                    y=self.data["cpu"]["norm_ps_util"],
                    mode=plot_mode,
                    name="CPU: Normalized Process Utilization (%)",
                    showlegend=True,
                ),
                row=1,
                col=1,
            )
            fig.add_trace(
                go.Scatter(
                    x=self.data["cpu"]["trace_num"],
                    y=self.data["cpu"]["consumed_utilized_energy_cumsum"],
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
                    x=self.data["gpu"]["trace_num"],
                    y=self.data["gpu"]["consumed_utilized_energy"],
                    mode=plot_mode,
                    name="GPU: Consumed Energy (J)",
                    showlegend=True,
                ),
                row=1,
                col=2 if device_option == "Both" else 1,
            )
            fig.add_trace(
                go.Scatter(
                    x=self.data["gpu"]["trace_num"],
                    y=self.data["gpu"]["avg_utilization"],
                    mode=plot_mode,
                    name="GPU: Avg. Utilization (%)",
                    showlegend=True,
                ),
                row=1,
                col=2 if device_option == "Both" else 1,
            )
            fig.add_trace(
                go.Scatter(
                    x=self.data["gpu"]["trace_num"],
                    y=self.data["gpu"]["consumed_utilized_energy_cumsum"],
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

    def _read_new_csvs(self):
        """Read new CSV files and update the aggregated data."""
        for file_name in sorted(os.listdir(self.csv_dir)):
            file_path = os.path.join(self.csv_dir, file_name)
            if file_name.endswith(".csv"):
                try:
                    if file_name.startswith("NvidiaGPU"):
                        gpu_df = pd.read_csv(file_path)
                        self.data["gpu"] = pd.concat(
                            [self.data["gpu"], gpu_df], ignore_index=True
                        )
                    elif file_name.startswith("RAPLSoC"):
                        cpu_df = pd.read_csv(file_path)
                        self.data["cpu"] = pd.concat(
                            [self.data["cpu"], cpu_df], ignore_index=True
                        )
                except Exception as e:
                    print(f"Error reading {file_path}: {e}")

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
