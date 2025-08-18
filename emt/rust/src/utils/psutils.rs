use std::collections::HashMap;
use sysinfo::System;
use users::{Users, UsersCache};
use crate::energy_monitor::ProcessGroup;


pub fn resolve_username(uid: u32, users_cache: &UsersCache) -> String {
    users_cache
        .get_user_by_uid(uid)
        .map(|user| user.name().to_string_lossy().to_string())
        .unwrap_or_else(|| uid.to_string())
}


/// Collects all process from the system and groups them by user and application
fn collect_all() -> HashMap<(String, String), Vec<usize>> {
    let system = System::new_all();
    let users_cache = UsersCache::new();
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    
    for (pid, process) in system.processes() {
        let user = process.user_id()
            .map(|uid| resolve_username(**uid, &users_cache))
            .unwrap_or_else(|| "unknown".to_string());
        let app = process.name().to_string_lossy().split('/').next().unwrap_or("unknown").to_string();
        groups.entry((user, app)).or_default().push(pid.as_u32() as usize);
    }
    groups
}

/// Filters process groups to only include the specified PIDs
fn filter_groups_by_pids(groups: &mut HashMap<(String, String), Vec<usize>>, selected_pids: &[usize]) {
    groups.values_mut().for_each(|pids_vec| {
        pids_vec.retain(|pid| selected_pids.contains(pid));
    });
    // Remove empty groups after filtering
    groups.retain(|_, pids| !pids.is_empty());
}

// Collects process groups based on the provided PIDs, if not explicitly provided collect all.
pub fn collect_process_groups(selected_pids: Option<Vec<usize>>) -> Result<Vec<ProcessGroup>, String> {
    let groups = match selected_pids {
        Some(ref pids) if pids.is_empty() => {
            // Explicitly requested no processes: return empty groups
            HashMap::new()
        }
        Some(pids) => {
            let mut groups: HashMap<(String, String), Vec<usize>> = collect_all();
            filter_groups_by_pids(&mut groups, &pids);
            groups
        }
        None => {
            collect_all()
        }
    };

    let tracked_processes: Vec<ProcessGroup> = groups
        .into_iter()
        .map(|((user, application), pids)| ProcessGroup { user, application, pids })
        .collect();

    Ok(tracked_processes)
}
