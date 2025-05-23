use std::fmt;

#[derive(Debug)]
pub enum TrackerError {
    InvalidPid,
    SysinfoError(String),
    Other(String),
}

impl fmt::Display for TrackerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrackerError::InvalidPid => write!(f, "Invalid PID"),
            TrackerError::SysinfoError(e) => write!(f, "Sysinfo error: {}", e),
            TrackerError::Other(e) => write!(f, "Other error: {}", e),
        }
    }
}

impl std::error::Error for TrackerError {}
