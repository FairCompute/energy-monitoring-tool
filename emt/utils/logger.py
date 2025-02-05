import os
import logging
from pathlib import Path

# Configure logging to write to a log file with a custom format
_DEFAULT_FORMATTER = logging.Formatter(
    "%(asctime)s - %(levelname)s - %(name)s - %(threadName)s - %(message)s"
)
_DEFAULT_LOG_DIR = Path.cwd() / "emt_logs"


def setup_logger(
    log_dir: os.PathLike = _DEFAULT_LOG_DIR,
    log_file_name: os.PathLike = "emt.log",
    mode: str = "a",
    formatter: logging.Formatter = _DEFAULT_FORMATTER,
    logging_level: int = logging.INFO,
) -> None:
    """
    Configure a custom logger for the EMT package.

    Args:
        log_file (os.PathLike):         The log file path.
        mode (str):                     The mode for opening the log file ('w' for write, 'a' for append).
                                        Default mode is set to 'a'
        formatter (logging.Formatter):  The log message formatter.
        logging_level:                  The logging level: (DEBUG, INFO, ERROR, CRITICAL)
                                        defaults to `logging.DEBUG`
    Returns:
        None

    """
    # reset any existing logger
    logger = reset_logger()
    os.makedirs(log_dir, exist_ok=True)
    file_path = os.path.join(log_dir, log_file_name)
    # configure as root logger
    logger.setLevel(logging_level)  # Set logging level for the logger

    handler = logging.FileHandler(file_path, mode=mode)
    handler.setFormatter(formatter)
    handler.setLevel(logging_level)  # Set logging level for the handler

    logger.addHandler(handler)
    logger.info("EMT logger created ...")


def reset_logger() -> logging.Logger:
    logger = logging.getLogger()
    while logger.handlers:  # Remove all handlers
        handler = logger.handlers[0]
        logger.removeHandler(handler)
    logger.propagate = False  # Prevent propagation to the root logger
    return logger
