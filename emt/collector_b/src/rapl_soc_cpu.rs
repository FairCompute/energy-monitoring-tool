use crate::power_groups::PowerGroup;
use async_trait::async_trait;
use log::info;
use psutil::process::{Process, ProcessResult};

pub struct RaplSocCpuGroup {
    base: PowerGroup,
    rapl_path: String,
}

impl RaplSocCpuGroup {
    pub fn new(pid: Option<i32>, rate: f64, rapl_path: String) -> ProcessResult<Self> {
        let base = PowerGroup::new(pid, rate)?;
        Ok(Self { base, rapl_path })
    }
}
