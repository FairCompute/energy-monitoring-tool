import os
import logging
from .energy_meter import *
from .power_group import *

# Configure logging to write to a log file with a custom format


_DEFAULT_FORMATTER = logging.Formatter(
    "%(asctime)s  - %(levelname)s  - %(name)s - %(threadName)s - %(message)s"
)


def setup_logger(
    log_file: os.PathLike = "emt.log",
    mode: str = "a",
    formatter: logging.Formatter = _DEFAULT_FORMATTER,
    logging_level: int = logging.DEBUG,
) -> None:
    """
    Configure a custom logger for the EMT package.

    Args:
        log_file (os.PathLike):         The log file path.
        mode (str):                     The mode for opening the log file ('w' for write, 'a' for append).
                                        Default mode is set to 'a'
        formatter (logging.Formatter):  The log message formatter.
        logging_level:                  The logging level: (DEBUG, INFO,ERROR,CRITICAL)
                                        defaults to `logging.DEBUG`
    Returns:
        None

    """
    logger = logging.getLogger(__name__)
    logger.setLevel(logging_level)
    handler = logging.FileHandler(log_file, mode=mode)
    handler.setFormatter(formatter)
    logger.addHandler(handler)
    logger.info("A logger is created for the energy monitoring tool.")
