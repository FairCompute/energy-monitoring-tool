use crate::utils::errors::MonitoringError;
use std::collections::HashMap;
use sysinfo::System;
use users::{Users, UsersCache};

/// A group of processes belonging to the same user and application
#[derive(Debug)]
pub struct ProcessGroup {
    pub user: String,
    pub task: String,
    pub pids: Vec<usize>,
}

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
        return Err(MonitoringError::ProcessDiscoveryError(
            "No processes found on system".to_string(),
        ));
    }

    for (pid, process) in processes {
        let user = process
            .user_id()
            .map(|uid| resolve_username(**uid, &users_cache))
            .unwrap_or_else(|| "unknown".to_string());
        let app = resolve_group_name(&process.name().to_string_lossy());
        groups
            .entry((user, app))
            .or_default()
            .push(pid.as_u32() as usize);
    }

    Ok(groups)
}

/// Get all child PIDs of a given parent PID (recursive)
fn get_child_pids(system: &System, parent_pid: usize) -> Vec<usize> {
    let mut children = Vec::new();
    let parent_pid_obj = sysinfo::Pid::from_u32(parent_pid as u32);

    for (pid, process) in system.processes() {
        if let Some(ppid) = process.parent() {
            if ppid == parent_pid_obj {
                let child_pid = pid.as_u32() as usize;
                children.push(child_pid);
                // Recursively get grandchildren
                children.extend(get_child_pids(system, child_pid));
            }
        }
    }
    children
}

/// Expand PIDs to include all children (process tree)
fn expand_pids_with_children(system: &System, pids: &[usize]) -> Vec<usize> {
    let mut expanded = pids.to_vec();
    for &pid in pids {
        expanded.extend(get_child_pids(system, pid));
    }
    // Remove duplicates
    expanded.sort();
    expanded.dedup();
    expanded
}

/// Filters process groups to only include groups that have at least one of the specified PIDs.
fn filter_groups_by_pids(
    groups: &mut HashMap<(String, String), Vec<usize>>,
    selected_pids: &[usize],
) {
    groups.retain(|_, pids| {
        pids.retain(|pid| selected_pids.contains(pid));
        !pids.is_empty()
    });
}

// Collects process groups based on the provided PIDs, if not explicitly provided collect all.
// When PIDs are provided, also includes all child processes (process tree).
pub fn collect_process_groups(
    selected_pids: Option<Vec<usize>>,
) -> Result<Vec<ProcessGroup>, MonitoringError> {
    let system = System::new_all();

    let groups = match selected_pids {
        Some(ref pids) if pids.is_empty() => {
            // Explicitly requested no processes: return empty groups
            Ok(HashMap::new())
        }
        Some(pids) => {
            // Expand PIDs to include children (process tree)
            let expanded_pids = expand_pids_with_children(&system, &pids);
            log::info!(
                "Expanded {} PIDs to {} (including children)",
                pids.len(),
                expanded_pids.len()
            );

            let mut groups = collect_all()?;
            filter_groups_by_pids(&mut groups, &expanded_pids);
            Ok(groups)
        }
        None => collect_all(),
    }?;

    if groups.is_empty() {
        return Err(MonitoringError::ProcessDiscoveryError(
            "No process groups found".to_string(),
        ));
    }

    let tracked_processes: Vec<ProcessGroup> = groups
        .into_iter()
        .map(|((user, application), pids)| ProcessGroup {
            user,
            task: application,
            pids,
        })
        .collect();

    Ok(tracked_processes)
}
