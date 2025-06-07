import os
import logging
from pathlib import Path
import sys

# Configure logging to write to a log file with a custom format
_DEFAULT_FORMATTER = logging.Formatter(
    "%(asctime)s - %(levelname)s - %(name)s - %(threadName)s - %(message)s"
)
_DEFAULT_LOG_DIR = Path.cwd() / ".emt_logs"

def setup_logger(
    logger: logging.Logger,
    log_dir: os.PathLike = _DEFAULT_LOG_DIR,
    log_file_name: os.PathLike = "emt.log",
    mode: str = "a",
    formatter: logging.Formatter = _DEFAULT_FORMATTER,
    logging_level: int = logging.INFO,
    to_std_streams: bool = False,
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
        forward_to_console (bool):      Whether to forward logs to stdout/stderr.
    Returns:
        None

    """
    # reset any existing logger
    logger = reset_logger(logger)
    os.makedirs(log_dir, exist_ok=True)
    file_path = os.path.join(log_dir, log_file_name)
    # configure as root logger
    logger.setLevel(logging_level)  # Set logging level for the logger

    handler = logging.FileHandler(file_path, mode=mode)
    handler.setFormatter(formatter)
    handler.setLevel(logging_level)  # Set logging level for the handler

    logger.addHandler(handler)

    # Optional console handler (stdout and stderr)
    if to_std_streams:
        stdout_handler = logging.StreamHandler(sys.stdout)
        stdout_handler.setLevel(logging_level)
        stdout_handler.setFormatter(formatter)
        logger.addHandler(stdout_handler)

        stderror_handler = logging.StreamHandler(sys.stderr)
        stderror_handler.setLevel(logging.ERROR)
        stderror_handler.setFormatter(formatter)
        logger.addHandler(stderror_handler)

    logger.info("EMT logger created ...")


def reset_logger(logger: logging.Logger) -> logging.Logger:
    while logger.handlers:
        handler = logger.handlers[0]
        logger.removeHandler(handler)
    logger.propagate = False  # Prevent propagation to the root logger
    return logger
