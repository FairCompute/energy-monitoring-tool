use std::collections::HashMap;
use sysinfo::System;
use users::{Users, UsersCache};
use crate::energy_group::ProcessGroup;
use crate::utils::errors::MonitoringError;


pub fn resolve_username(uid: u32, users_cache: &UsersCache) -> String {
    users_cache
        .get_user_by_uid(uid)
        .map(|user| user.name().to_string_lossy().to_string())
        .unwrap_or_else(|| uid.to_string())
}

pub fn resolve_group_name(name: &str) -> String {
    name.split('/').next().unwrap_or("unknown").to_string()
}

/// Collects all process from the system and groups them by user and application
fn collect_all() -> Result<HashMap<(String, String), Vec<usize>>, MonitoringError> {
    let system = System::new_all();
    let users_cache = UsersCache::new();
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();

    // If there are no processes, treat as an error
    let processes = system.processes();
    if processes.is_empty() {
        return Err(MonitoringError::ProcessDiscoveryError("No processes found on system".to_string()));
    }

    for (pid, process) in processes {
        let user = process.user_id()
            .map(|uid| resolve_username(**uid, &users_cache))
            .unwrap_or_else(|| "unknown".to_string());
        let app = resolve_group_name(&process.name().to_string_lossy());
        groups.entry((user, app)).or_default().push(pid.as_u32() as usize);
    }
    
    Ok(groups)
}

/// Filters process groups to only include groups that have at least one of the specified PIDs.
fn filter_groups_by_pids(groups: &mut HashMap<(String, String), Vec<usize>>, selected_pids: &[usize]) {
    groups.retain(|_, pids| {
        pids.retain(|pid| selected_pids.contains(pid));
        !pids.is_empty()
    });
}

// Collects process groups based on the provided PIDs, if not explicitly provided collect all.
pub fn collect_process_groups(selected_pids: Option<Vec<usize>>) -> Result<Vec<ProcessGroup>, MonitoringError> {
    let groups = match selected_pids {
        Some(ref pids) if pids.is_empty() => {
            // Explicitly requested no processes: return empty groups
            Ok(HashMap::new())
        }
        Some(pids) => {
            let mut groups = collect_all()?;
            filter_groups_by_pids(&mut groups, &pids);
            Ok(groups)
        }
        None => {
            collect_all()
        }
    }?;

    if groups.is_empty() {
        return Err(MonitoringError::ProcessDiscoveryError("No process groups found".to_string()));
    }

    let tracked_processes: Vec<ProcessGroup> = groups
        .into_iter()
        .map(|((user, application), pids)| ProcessGroup { user, task: application, pids })
        .collect();

    Ok(tracked_processes)
}
