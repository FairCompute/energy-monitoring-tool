import pytest
import unittest
from unittest.mock import MagicMock, patch
from emt.power_groups import PowerGroup
from emt.utils import PGUtils


class MockPowerGroup1(PowerGroup):
    @staticmethod
    def is_available():
        return True


class MockPowerGroup2(PowerGroup):
    @staticmethod
    def is_available():
        return False


@pytest.fixture
def pg_utils_instance():
    return PGUtils()


def test_get_pg_types(pg_utils_instance):
    # Mock the power_groups module
    with patch("emt.utils.powergroup_utils.power_groups") as mock_module:
        mock_module.MockPowerGroup1 = MockPowerGroup1
        mock_module.MockPowerGroup2 = MockPowerGroup2

        pg_types = pg_utils_instance.get_pg_types(mock_module)

        assert MockPowerGroup1 in pg_types
        assert MockPowerGroup2 in pg_types
        assert PowerGroup not in pg_types  # Base class should not be included


def test_get_available_pgs(pg_utils_instance):
    with patch("emt.utils.powergroup_utils.power_groups") as mock_module:
        mock_module.MockPowerGroup1 = MockPowerGroup1
        mock_module.MockPowerGroup2 = MockPowerGroup2

        print(pg_utils_instance.powergroup_types)
        available_pgs = pg_utils_instance.get_available_pgs()

        assert len(available_pgs) == 1  # only one class of powergroup is available
        assert isinstance(available_pgs[0], MockPowerGroup1)


def test_get_pg_table(pg_utils_instance):
    with patch("emt.utils.powergroup_utils.power_groups") as mock_module:
        mock_module.MockPowerGroup1 = MockPowerGroup1
        mock_module.MockPowerGroup2 = MockPowerGroup2

        pg_utils_instance.get_available_pgs()  # Populate powergroup_types
        table_output = pg_utils_instance.get_pg_table()

        assert "MockPowerGroup1" in table_output
        assert "MockPowerGroup2" in table_output
        assert "Yes" in table_output
        assert "Tracked @ 10Hz" in table_output
        assert "No" in table_output
