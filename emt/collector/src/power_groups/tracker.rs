use crate::power_groups::errors::TrackerError;
use async_trait::async_trait;
use std::collections::HashMap;
use sysinfo::{Pid, System};

#[derive(Debug)]
pub struct ProcessGroup {
    pub category: String,
    pub application: String,
    pub pids: Vec<usize>,
}

pub struct PowerGroupTracker {
    rate: f64,
    count_trace_calls: usize,
    tracked_processes: Vec<ProcessGroup>,
    consumed_energy: Vec<f64>,
    energy_trace: HashMap<u64, Vec<f64>>,
}

impl PowerGroupTracker {
    pub fn new(rate: f64, provided_pids: Option<Vec<usize>>) -> Result<Self, TrackerError> {
        let system = System::new_all();
        let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
        match provided_pids {
            Some(ref pids) if pids.is_empty() => {
                // Explicitly requested no processes: return empty groups
            }
            Some(pids) => {
                for pid in pids {
                    if let Some(process) = system.process(Pid::from(pid)) {
                        let user_id = process.user_id().map(|u| u.to_string()).unwrap_or_else(|| "unknown".to_string());
                        let category = if user_id == "0" || user_id == "root" {
                            "system".to_string()
                        } else {
                            format!("user@{}", user_id)
                        };
                        let app = process.name().to_string_lossy().split('/').next().unwrap_or("unknown").to_string();
                        groups.entry((category, app)).or_default().push(pid);
                    } else {
                        groups.entry(("system".to_string(), "unknown".to_string())).or_default().push(pid);
                    }
                }
            }
            None => {
                for (pid, process) in system.processes() {
                    let user_id = process.user_id().map(|u| u.to_string()).unwrap_or_else(|| "unknown".to_string());
                    let category = if user_id == "0" || user_id == "root" {
                        "system".to_string()
                    } else {
                        format!("user@{}", user_id)
                    };
                    let app = process.name().to_string_lossy().split('/').next().unwrap_or("unknown").to_string();
                    groups.entry((category, app)).or_default().push(pid.as_u32() as usize);
                }
            }
        }
        let tracked_processes: Vec<ProcessGroup> = groups
            .into_iter()
            .map(|((category, application), pids)| ProcessGroup { category, application, pids })
            .collect();
        let total_pids = tracked_processes.iter().map(|g| g.pids.len()).sum();
        let consumed_energy = vec![0.0; total_pids];
        Ok(Self {
            rate,
            count_trace_calls: 0,
            tracked_processes,
            energy_trace: HashMap::new(),
            consumed_energy,
        })
    }

    pub fn sleep_interval(&self) -> f64 {
        1.0 / self.rate
    }
    pub fn processes(&self) -> &Vec<ProcessGroup> {
        &self.tracked_processes
    }
    pub fn consumed_energy(&self) -> &Vec<f64> {
        &self.consumed_energy
    }
    pub fn energy_trace(&self) -> HashMap<u64, Vec<f64>> {
        self.energy_trace.clone()
    }
}

#[async_trait]
pub trait AsyncEnergyCollector {
    fn get_trace(&self) -> Result<HashMap<u64, Vec<f64>>, String>;
    fn is_available() -> bool {
        unimplemented!()
    }
    async fn commence(&mut self) -> Result<(), String>;
    async fn shutdown(&mut self) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Test tracker with empty PID list returns no groups
    fn test_tracker_empty() {
        let tracker = PowerGroupTracker::new(1.0, Some(vec![])).unwrap();
        assert_eq!(tracker.processes().len(), 0);
    }

    #[test]
    // Test tracker with a nonexistent PID returns a system/unknown group
    fn test_tracker_with_nonexistent_pid() {
        let tracker = PowerGroupTracker::new(1.0, Some(vec![999999])).unwrap();
        let groups = tracker.processes();
        assert!(groups.iter().any(|g| g.category == "system" && g.application == "unknown"));
    }

    #[test]
    // Test tracker with PID 1 returns a system group containing PID 1
    fn test_tracker_with_pid_1() {
        let tracker = PowerGroupTracker::new(1.0, Some(vec![1])).unwrap();
        let groups = tracker.processes();
        assert!(groups.iter().any(|g| g.category == "system"));
        assert!(groups.iter().any(|g| g.pids.contains(&1)));
    }

    #[test]
    // Test tracker with all processes returns at least one group
    fn test_tracker_all_processes_not_empty() {
        let tracker = PowerGroupTracker::new(1.0, None).unwrap();
        let groups = tracker.processes();
        assert!(!groups.is_empty());
    }

    #[test]
    // Test all PIDs are unique across all groups
    fn test_tracker_pid_grouping() {
        let tracker = PowerGroupTracker::new(1.0, None).unwrap();
        let groups = tracker.processes();
        let mut all_pids: Vec<usize> = Vec::new();
        for group in groups {
            all_pids.extend(&group.pids);
        }
        all_pids.sort();
        all_pids.dedup();
        assert_eq!(all_pids.len(), tracker.consumed_energy().len());
    }

    #[test]
    // Test all groups have non-empty category and application
    fn test_tracker_category_and_application() {
        let tracker = PowerGroupTracker::new(1.0, None).unwrap();
        let groups = tracker.processes();
        for group in groups {
            assert!(!group.category.is_empty());
            assert!(!group.application.is_empty());
        }
    }
}
