use thiserror::Error;

#[derive(Error, Debug)]
pub enum MonitoringError {
    #[error("Sysinfo error: {0}")]
    SysinfoError(String),
    #[error("Process discovery error: {0}")]
    ProcessDiscoveryError(String),
    #[error("Other error: {0}")]
    Other(String),
}