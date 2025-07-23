use std::collections::HashMap;
use sysinfo::{Pid, System};
use users::{Users, UsersCache};
use crate::energy_monitor::ProcessGroup;

/// Utility function to resolve a user ID to a username
pub fn resolve_username(uid: u32, users_cache: &UsersCache) -> String {
    users_cache
        .get_user_by_uid(uid)
        .map(|user| user.name().to_string_lossy().to_string())
        .unwrap_or_else(|| uid.to_string())
}

/// Utility function for default process discovery behavior
/// This can be used by collectors that don't need special filtering logic
pub fn gather_process_groups(provided_pids: Option<Vec<usize>>) -> Result<Vec<ProcessGroup>, String> {
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
                        .map(|uid| resolve_username(**uid, &users_cache))
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
                    .map(|uid| resolve_username(**uid, &users_cache))
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

    Ok(tracked_processes)
}
