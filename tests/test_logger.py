import pytest
import logging
from unittest.mock import patch
from emt.utils import setup_logger


@pytest.fixture
def temp_log_dir(tmp_path):
    """Fixture to provide a temporary log directory."""
    return tmp_path


def test_setup_logger_creates_log_dir(temp_log_dir):
    """Test that setup_logger creates the log directory if it doesn't exist."""
    logger = logging.getLogger("test_logger")
    log_dir = temp_log_dir / "logs"
    log_file_name = "test.log"
    setup_logger(logger, log_dir=log_dir, log_file_name=log_file_name)
    assert log_dir.exists() and log_dir.is_dir(), "Log directory was not created."


def test_setup_logger_writes_to_log_file(temp_log_dir):
    """Test that setup_logger writes to the specified log file."""
    logger = logging.getLogger("test_logger")
    log_dir = temp_log_dir
    log_file_name = "test.log"
    setup_logger(logger,log_dir=log_dir, log_file_name=log_file_name)
    log_file_path = log_dir / log_file_name

    assert log_file_path.exists(), "Log file was not created."
    with open(log_file_path, "r") as f:
        content = f.read()
        assert (
            "EMT logger created ..." in content
        ), "Log message not found in the log file."


def test_setup_logger_custom_formatter(temp_log_dir):
    """Test that setup_logger uses a custom formatter."""
    logger = logging.getLogger("test_logger")
    log_dir = temp_log_dir
    log_file_name = "test.log"

    setup_logger(
        logger,
        log_dir=log_dir,
        log_file_name=log_file_name,
    )
    log_file_path = log_dir / log_file_name

    assert log_file_path.exists(), "Log file was not created."
    with open(log_file_path, "r") as f:
        content = f.read()
        print(content)
        assert (
            "INFO - test_logger - MainThread - EMT logger created ..." in content
        ), "Custom formatter was not applied."


def test_setup_logger_calls_reset_logger():
    """Test that setup_logger calls reset_logger."""
    logger = logging.getLogger("test_logger")
    with patch(
        "emt.utils.logger.reset_logger"
    ) as mock_reset_logger:  # Mock reset_logger
        # Call setup_logger
        setup_logger(logger, log_dir="logs", log_file_name="test.log")
        # Assert reset_logger was called once
        mock_reset_logger.assert_called_once()


if __name__ == "__main__":
    pytest.main([__file__])