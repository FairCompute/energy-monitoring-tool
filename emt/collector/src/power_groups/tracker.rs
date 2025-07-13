use crate::power_groups::errors::TrackerError;
use async_trait::async_trait;
use std::collections::HashMap;
use sysinfo::{Pid, System};
use users::{Users, UsersCache};

#[derive(Debug)]
pub struct ProcessGroup {
    pub user: String,
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

    fn resolve_username(uid: u32, users_cache: &UsersCache) -> String {
        users_cache
            .get_user_by_uid(uid)
            .map(|user| user.name().to_string_lossy().to_string())
            .unwrap_or_else(|| uid.to_string())
    }
    
    pub fn new(rate: f64, provided_pids: Option<Vec<usize>>) -> Result<Self, TrackerError> {
        let system = System::new_all();
        let users_cache = UsersCache::new();
        let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();

        match provided_pids {
            Some(ref pids) if pids.is_empty() => {
                // Explicitly requested no processes: return empty groups
            }
            Some(pids) => {
                for pid in pids {
                    if let Some(process) = system.process(Pid::from(pid)) {
                        let user = process.user_id()
                            .map(|uid| Self::resolve_username(**uid, &users_cache))
                            .unwrap_or_else(|| "unknown".to_string());
                        let app = process.name().to_string_lossy().split('/').next().unwrap_or("unknown").to_string();
                        groups.entry((user, app)).or_default().push(pid);
                    } else {
                        groups.entry(("unknown".to_string(), "unknown".to_string())).or_default().push(pid);
                    }
                }
            }
            None => {
                for (pid, process) in system.processes() {
                    let user = process.user_id()
                        .map(|uid| Self::resolve_username(**uid, &users_cache))
                        .unwrap_or_else(|| "unknown".to_string());
                    let app = process.name().to_string_lossy().split('/').next().unwrap_or("unknown").to_string();
                    groups.entry((user, app)).or_default().push(pid.as_u32() as usize);
                }
            }
        }
        let tracked_processes: Vec<ProcessGroup> = groups
            .into_iter()
            .map(|((user, application), pids)| ProcessGroup { user, application, pids })
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
    // Test tracker with a nonexistent PID returns a unknown/unknown group
    fn test_tracker_with_nonexistent_pid() {
        let tracker = PowerGroupTracker::new(1.0, Some(vec![999999])).unwrap();
        let groups = tracker.processes();
        assert!(groups.iter().any(|g| g.user == "unknown" && g.application == "unknown"));
    }

    #[test]
    // Test tracker with PID 1 returns a group containing PID 1
    fn test_tracker_with_pid_1() {
        let tracker = PowerGroupTracker::new(1.0, Some(vec![1])).unwrap();
        let groups = tracker.processes();
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
    // Test to debug what users we actually see
    fn test_debug_users() {
        use std::collections::HashMap;
        
        let system = System::new_all();
        let users_cache = UsersCache::new();
        let mut user_counts: HashMap<String, usize> = HashMap::new();
        
        // Count processes by user
        for (_pid, process) in system.processes().take(100) {
            let user = process.user_id()
                .map(|uid| Self::resolve_username(**uid, &users_cache))
                .unwrap_or_else(|| "unknown".to_string());
            
            *user_counts.entry(user).or_insert(0) += 1;
        }
        
        // Print results
        let mut users: Vec<(String, usize)> = user_counts.into_iter().collect();
        users.sort_by(|a, b| b.1.cmp(&a.1));
        
        eprintln!("Users found (first 10):");
        for (user, count) in users.iter().take(10) {
            eprintln!("  {}: {} processes", user, count);
        }
        
        // The test should pass - we just want to see the debug output
        assert!(!users.is_empty());
    }
}
