"""
Energy Monitoring Tool (EMT) Package

A comprehensive tool for monitoring energy consumption in computing applications.
"""

__all__ = [
    "EnergyMonitor",
]


def __getattr__(name):
    if name == "EnergyMonitor":
        from .energy_monitor import EnergyMonitor

        return EnergyMonitor
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
