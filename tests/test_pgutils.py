import pytest
import unittest
from unittest.mock import MagicMock, patch
from emt.power_groups import PowerGroup, get_pg_types, get_available_pg_types, get_available_pgs, get_pg_table


class MockPowerGroup1(PowerGroup):
    @classmethod
    def is_available(cls):
        return True


class MockPowerGroup2(PowerGroup):
    @classmethod
    def is_available(cls):
        return False


def test_get_pg_types():
    # Mock the power_groups module
    with patch("emt.power_groups.utils.power_groups") as mock_module:
        mock_module.MockPowerGroup1 = MockPowerGroup1
        mock_module.MockPowerGroup2 = MockPowerGroup2

        pg_types = get_pg_types(mock_module)

        assert MockPowerGroup1 in pg_types
        assert MockPowerGroup2 in pg_types
        assert PowerGroup not in pg_types  # Base class should not be included


def test_get_available_pg_types():
    with patch("emt.power_groups.utils.get_pg_types") as mock_get_pg_types:
        mock_get_pg_types.return_value = [MockPowerGroup1, MockPowerGroup2]
        
        available_types = get_available_pg_types()
        
        assert len(available_types) == 1  # only one class is available
        assert MockPowerGroup1 in available_types
        assert MockPowerGroup2 not in available_types


def test_get_available_pgs():
    with patch("emt.power_groups.utils.get_available_pg_types") as mock_get_available_types:
        mock_get_available_types.return_value = [MockPowerGroup1]
        
        available_pgs = get_available_pgs()
        
        assert len(available_pgs) == 1
        assert isinstance(available_pgs[0], MockPowerGroup1)


def test_get_pg_table():
    with patch("emt.power_groups.utils.get_pg_types") as mock_get_pg_types:
        mock_get_pg_types.return_value = [MockPowerGroup1, MockPowerGroup2]
        
        table_output = get_pg_table()
        
        assert "MockPowerGroup1" in table_output
        assert "MockPowerGroup2" in table_output
        assert "Yes" in table_output
        assert "Tracked @ 10Hz" in table_output
        assert "No" in table_output
