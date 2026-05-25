use crate::utils::errors::MonitoringError;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use sysinfo::System;
use users::{Users, UsersCache};

// ─── New process discovery model (consumed by Monitor) ───────────────────────

/// Represents a root process (tree top) identified during a full scan.
/// A root is a process whose parent PID is not present in the system snapshot.
#[derive(Debug, Clone)]
pub struct ProcessRoot {
    pub pid: u32,
    pub user: String,
    pub name: String,
}

/// Fast child-walk that reads /proc directly (no sysinfo overhead).
/// Returns all descendant PIDs of the given root PIDs, including the roots themselves.
/// Designed to be called at 10 Hz — must be sub-millisecond for ~20-50 roots.
pub fn walk_child_pids(roots: &[u32]) -> Vec<u32> {
    if roots.is_empty() {
        return Vec::new();
    }

    match walk_child_pids_from_children_files(roots) {
        Ok(pids) => return pids,
        Err(_) => return walk_child_pids_by_scanning_proc(roots),
    }
}

fn walk_child_pids_from_children_files(roots: &[u32]) -> Result<Vec<u32>, std::io::Error> {
    let mut result: Vec<u32> = Vec::new();
    let mut visited: HashSet<u32> = HashSet::new();
    let mut queue: VecDeque<u32> = VecDeque::new();

    for &root in roots {
        if visited.insert(root) {
            result.push(root);
            queue.push_back(root);
        }
    }

    while let Some(current) = queue.pop_front() {
        let proc_path = format!("/proc/{}", current);
        if fs::metadata(&proc_path).is_err() {
            continue;
        }

        let children_path = format!("/proc/{}/task/{}/children", current, current);
        let children = fs::read_to_string(children_path)?;
        for child in children.split_whitespace() {
            if let Ok(child_pid) = child.parse::<u32>() {
                if visited.insert(child_pid) {
                    result.push(child_pid);
                    queue.push_back(child_pid);
                }
            }
        }
    }

    Ok(result)
}

fn walk_child_pids_by_scanning_proc(roots: &[u32]) -> Vec<u32> {
    let mut parent_to_children: HashMap<u32, Vec<u32>> = HashMap::new();

    let proc_dir = match fs::read_dir("/proc") {
        Ok(dir) => dir,
        Err(_) => return roots.to_vec(),
    };

    for entry in proc_dir.flatten() {
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        let pid: u32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let status_path = format!("/proc/{}/status", pid);
        let contents = match fs::read_to_string(&status_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in contents.lines() {
            if let Some(ppid_str) = line.strip_prefix("PPid:\t") {
                if let Ok(ppid) = ppid_str.trim().parse::<u32>() {
                    parent_to_children.entry(ppid).or_default().push(pid);
                }
                break;
            }
        }
    }

    let mut result: Vec<u32> = Vec::new();
    let mut visited: HashSet<u32> = HashSet::new();
    let mut queue: VecDeque<u32> = VecDeque::new();

    for &root in roots {
        if visited.insert(root) {
            result.push(root);
            queue.push_back(root);
        }
    }

    while let Some(current) = queue.pop_front() {
        if let Some(children) = parent_to_children.get(&current) {
            for &child in children {
                if visited.insert(child) {
                    result.push(child);
                    queue.push_back(child);
                }
            }
        }
    }

    result
}

/// Full process scan to identify root PIDs (tree tops).
/// A root is a process whose parent PID is NOT in the discovered set of all PIDs.
/// Uses sysinfo for full metadata (user, name).
pub fn scan_roots() -> Vec<ProcessRoot> {
    let system = System::new_all();
    let users_cache = UsersCache::new();

    let all_pids: HashSet<u32> = system.processes().keys().map(|pid| pid.as_u32()).collect();

    let mut roots = Vec::new();
    for (pid, process) in system.processes() {
        let is_root = match process.parent() {
            Some(ppid) => !all_pids.contains(&ppid.as_u32()),
            None => true,
        };

        if is_root {
            let user = process
                .user_id()
                .map(|uid| resolve_username(**uid, &users_cache))
                .unwrap_or_else(|| "unknown".to_string());
            let name = process.name().to_string_lossy().to_string();

            roots.push(ProcessRoot {
                pid: pid.as_u32(),
                user,
                name,
            });
        }
    }

    roots
}

// ─── Legacy functions (still used by main.rs) ────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_child_pids_includes_roots() {
        // Current process should appear in results when passed as root
        let my_pid = std::process::id();
        let result = walk_child_pids(&[my_pid]);
        assert!(result.contains(&my_pid));
    }

    #[test]
    fn walk_child_pids_empty_roots_returns_empty() {
        let result = walk_child_pids(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn walk_child_pids_nonexistent_pid_returns_just_root() {
        // A PID that almost certainly doesn't exist
        let result = walk_child_pids(&[999_999_999]);
        // The root is included even if it has no children in the system
        assert!(result.contains(&999_999_999));
    }

    #[test]
    fn walk_child_pids_pid_1_has_descendants() {
        // PID 1 (init/systemd) should have many descendants
        let result = walk_child_pids(&[1]);
        assert!(result.contains(&1));
        // init should have at least some children on any running Linux system
        assert!(result.len() > 1);
    }

    #[test]
    fn scan_roots_returns_nonempty() {
        let roots = scan_roots();
        // There should always be at least one root process (init/systemd)
        assert!(!roots.is_empty());
    }

    #[test]
    fn scan_roots_contains_init() {
        let roots = scan_roots();
        // PID 1 should always be a root
        assert!(roots.iter().any(|r| r.pid == 1));
    }

    #[test]
    fn scan_roots_have_metadata() {
        let roots = scan_roots();
        // All roots should have non-empty names
        for root in &roots {
            assert!(
                !root.name.is_empty(),
                "Root PID {} has empty name",
                root.pid
            );
        }
    }
}
