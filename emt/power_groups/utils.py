"""
Utility functions for power group management.

This module provides functional utilities for discovering, instantiating,
and managing power groups in the Energy Monitoring Tool.
"""

import os

from emt import power_groups
from emt.power_groups import PowerGroup
from tabulate import tabulate
from typing import List, Type, Any, Optional, Dict

from emt.utils.config import load_config

# Public API
__all__ = [
    "get_pg_types",
    "get_available_pg_types",
    "get_available_pgs",
    "get_pg_table",
]


def get_pg_types(module: Optional[Any] = None) -> List[Type[PowerGroup]]:
    """
    Get all PowerGroup subclasses from the given module.

    Args:
        module: The module to search for PowerGroup types.
                If None, uses the power_groups module.

    Returns:
        List of PowerGroup subclass types found in the module.
    """
    if module is None:
        module = power_groups

    candidates = [
        getattr(module, name)
        for name in dir(module)
        if isinstance(getattr(module, name), type)
    ]
    pg_types = filter(
        lambda x: issubclass(x, PowerGroup) and x is not PowerGroup, candidates
    )
    return list(pg_types)


def get_available_pg_types() -> List[Type[PowerGroup]]:
    """
    Get available PowerGroup types (those that pass the is_available() check).

    Returns:
        List of available PowerGroup subclass types.
    """
    all_pg_types = get_pg_types()
    return [
        pg_type
        for pg_type in all_pg_types
        if not _is_disabled_by_environment(pg_type) and pg_type.is_available()
    ]


def _is_disabled_by_environment(pg_type: Type[PowerGroup]) -> bool:
    if pg_type.__name__ == "NvidiaGPU" and os.getenv("EMT_DISABLE_GPU"):
        return True
    return False


def _get_pg_rate(pg_type: Type[PowerGroup], config: Dict[str, Any]) -> Optional[int]:
    pg_map = {
        "RAPLSoC": "rapl",
        "NvidiaGPU": "nvidia_gpu",
    }
    key = pg_map.get(pg_type.__name__)
    if not key:
        return None
    rate = config.get("power_groups", {}).get(key, {}).get("rate")
    if isinstance(rate, int) and rate > 0:
        return rate
    return None


def get_available_pgs(**kwargs) -> List[PowerGroup]:
    """
    Get instantiated available PowerGroup objects.

    Args:
        **kwargs: Keyword arguments to pass to PowerGroup constructors.

    Returns:
        List of instantiated PowerGroup objects for available power groups.
    """
    available_types = get_available_pg_types()
    try:
        config = load_config()
    except Exception:
        config = {}

    power_groups = []
    for pg_type in available_types:
        pg_kwargs = dict(kwargs)
        rate = _get_pg_rate(pg_type, config)
        if rate is not None and "rate" not in pg_kwargs:
            pg_kwargs["rate"] = rate
        power_groups.append(pg_type(**pg_kwargs))
    return power_groups


def get_pg_table() -> str:
    """
    Get PowerGroup information in a tabular format.

    Returns:
        Formatted table string showing device types, availability, and tracking status.
    """
    all_pg_types = get_pg_types()

    try:
        config = load_config()
    except Exception:
        config = {}

    table = []
    headers = ["Devices", "Available", "Tracked"]

    for pg_type in all_pg_types:
        if _is_disabled_by_environment(pg_type):
            table.append([pg_type.__name__, "No", "Disabled"])
            continue

        is_available = pg_type.is_available()
        rate = _get_pg_rate(pg_type, config)
        tracked = "No"
        if is_available:
            tracked = f"Tracked @ {rate}Hz" if rate is not None else "Tracked"
        table.append([pg_type.__name__, "Yes" if is_available else "No", tracked])

    return tabulate(table, headers, tablefmt="pretty")
