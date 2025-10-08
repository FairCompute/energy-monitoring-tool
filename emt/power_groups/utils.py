"""
Utility functions for power group management.

This module provides functional utilities for discovering, instantiating,
and managing power groups in the Energy Monitoring Tool.
"""

from emt import power_groups
from emt.power_groups import PowerGroup
from tabulate import tabulate
from typing import List, Type, Any, Optional

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
    return [pg_type for pg_type in all_pg_types if pg_type.is_available()]


def get_available_pgs(**kwargs) -> List[PowerGroup]:
    """
    Get instantiated available PowerGroup objects.

    Args:
        **kwargs: Keyword arguments to pass to PowerGroup constructors.

    Returns:
        List of instantiated PowerGroup objects for available power groups.
    """
    available_types = get_available_pg_types()
    return [pg_type(**kwargs) for pg_type in available_types]


def get_pg_table() -> str:
    """
    Get PowerGroup information in a tabular format.

    Returns:
        Formatted table string showing device types, availability, and tracking status.
    """
    all_pg_types = get_pg_types()

    table = []
    headers = ["Devices", "Available", "Tracked"]

    for pg_type in all_pg_types:
        is_available = pg_type.is_available()
        table.append(
            [
                pg_type.__name__,
                "Yes" if is_available else "No",
                "Tracked @ 10Hz" if is_available else "No",
            ]
        )

    return tabulate(table, headers, tablefmt="pretty")
