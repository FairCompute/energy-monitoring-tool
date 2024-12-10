import os
import logging

# Configure logging to write to a log file with a custom format
_DEFAULT_FORMATTER = logging.Formatter(
    "%(asctime)s - %(levelname)s - %(name)s - %(threadName)s - %(message)s"
)


def setup_logger(
    log_dir: os.PathLike = "logs",
    log_file_name: os.PathLike = "new_emt.log",
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
    reset_logger()
    os.makedirs(log_dir, exist_ok=True)
    file_path = os.path.join(log_dir, log_file_name)

    logger = logging.getLogger(__name__)
    logger.setLevel(logging_level)  # Set logging level for the logger

    handler = logging.FileHandler(file_path, mode=mode)
    handler.setFormatter(formatter)
    handler.setLevel(logging_level)  # Set logging level for the handler

    logger.addHandler(handler)
    logger.info("EMT logger created ...")


def reset_logger():
    logger = logging.getLogger(__name__)
    while logger.handlers:  # Remove all handlers
        handler = logger.handlers[0]
        logger.removeHandler(handler)
    logger.propagate = False  # Prevent propagation to the root logger
