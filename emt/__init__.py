"""
Energy Monitoring Tool (EMT) Package

A comprehensive tool for monitoring energy consumption in computing applications.
"""

from .energy_monitor import EnergyMonitorCore, EnergyMonitor
from .power_groups import *  # Keep * import for power groups for now since it's complex
from .utils import (
    # Logger utilities
    setup_logger,
    # Trace recorder utilities
    TensorBoardWriterType,
    TraceRecorder,
    CSVRecorder,
    TensorboardRecorder,
    # Power group utilities
    PGUtils,
    # Config module
    config,
)

# Export all public symbols
__all__ = [
    # Energy monitoring core
    "EnergyMonitorCore",
    "EnergyMonitor",
    # Logger utilities
    "setup_logger",
    # Trace recorder utilities
    "TensorBoardWriterType",
    "TraceRecorder",
    "CSVRecorder",
    "TensorboardRecorder",
    # Power group utilities
    "PGUtils",
    # Config module (as submodule)
    "config",
]
