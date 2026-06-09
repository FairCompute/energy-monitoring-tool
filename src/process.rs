use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use users::{Users, UsersCache};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub process_group_id: Option<u32>,
    pub session_id: Option<u32>,
    pub command: String,
    pub user: String,
    pub cgroup_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessGroup {
    pub id: String,
    pub name: String,
    pub user: String,
    pub pids: Vec<u32>,
    pub representative_pid: u32,
}

pub trait GroupingStrategy {
    fn group(&self, processes: &[ProcessInfo]) -> Vec<ProcessGroup>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CgroupGrouping;

#[derive(Debug, Clone, Copy, Default)]
pub struct ParentLineageGrouping;

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessGroupIdGrouping;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CgroupLabel {
    key: String,
    name: String,
    priority: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcStatIds {
    parent_pid: Option<u32>,
    process_group_id: Option<u32>,
    session_id: Option<u32>,
}

impl GroupingStrategy for CgroupGrouping {
    fn group(&self, processes: &[ProcessInfo]) -> Vec<ProcessGroup> {
        let mut groups: BTreeMap<String, (String, Vec<&ProcessInfo>)> = BTreeMap::new();

        for process in processes {
            let label = cgroup_label(&process.cgroup_path).unwrap_or_else(unscoped_label);
            groups
                .entry(label.key)
                .or_insert_with(|| (label.name, Vec::new()))
                .1
                .push(process);
        }

        groups
            .into_iter()
            .filter_map(|(key, (name, processes))| {
                build_group(format!("cgroup:{key}"), name, &processes, None)
            })
            .collect()
    }
}

impl GroupingStrategy for ParentLineageGrouping {
    fn group(&self, processes: &[ProcessInfo]) -> Vec<ProcessGroup> {
        let by_pid = process_index(processes);
        let mut groups: BTreeMap<u32, Vec<&ProcessInfo>> = BTreeMap::new();

        for process in processes {
            if process.pid == 1 || process.pid == 2 {
                continue;
            }

            let root_pid = lineage_representative(process.pid, &by_pid);
            groups.entry(root_pid).or_default().push(process);
        }

        groups
            .into_iter()
            .filter_map(|(root_pid, processes)| {
                let name = by_pid
                    .get(&root_pid)
                    .map(|process| command_name(&process.command, process.pid))
                    .unwrap_or_else(|| format!("pid {root_pid}"));
                build_group(
                    format!("lineage:{root_pid}"),
                    name,
                    &processes,
                    Some(root_pid),
                )
            })
            .collect()
    }
}

impl GroupingStrategy for ProcessGroupIdGrouping {
    fn group(&self, processes: &[ProcessInfo]) -> Vec<ProcessGroup> {
        let mut groups: BTreeMap<u32, Vec<&ProcessInfo>> = BTreeMap::new();

        for process in processes {
            if process.pid == 1 || process.pid == 2 {
                continue;
            }

            let group_id = process.process_group_id.unwrap_or(process.pid);
            groups.entry(group_id).or_default().push(process);
        }

        groups
            .into_iter()
            .filter_map(|(group_id, processes)| {
                let representative_pid = processes
                    .iter()
                    .find(|process| process.pid == group_id)
                    .map(|process| process.pid)
                    .or_else(|| processes.first().map(|process| process.pid));
                let name = representative_pid
                    .and_then(|pid| {
                        processes
                            .iter()
                            .find(|process| process.pid == pid)
                            .map(|process| command_name(&process.command, process.pid))
                    })
                    .unwrap_or_else(|| format!("process group {group_id}"));
                build_group(
                    format!("pgrp:{group_id}"),
                    name,
                    &processes,
                    representative_pid,
                )
            })
            .collect()
    }
}

pub fn scan_processes() -> Vec<ProcessInfo> {
    let users_cache = UsersCache::new();
    let mut processes: Vec<ProcessInfo> = fs::read_dir("/proc")
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let stat_ids = read_proc_stat_ids(pid);
            Some(ProcessInfo {
                pid,
                parent_pid: stat_ids.and_then(|ids| ids.parent_pid),
                process_group_id: stat_ids.and_then(|ids| ids.process_group_id),
                session_id: stat_ids.and_then(|ids| ids.session_id),
                command: read_process_command(pid),
                user: read_process_user(pid, &users_cache),
                cgroup_path: read_cgroup_path(pid).unwrap_or_default(),
            })
        })
        .collect();

    processes.sort_by_key(|process| process.pid);
    processes
}

pub fn scan_process_groups<S: GroupingStrategy + ?Sized>(strategy: &S) -> Vec<ProcessGroup> {
    let processes = scan_processes();
    group_processes_with_strategy(&processes, strategy)
}

pub fn group_processes_with_strategy<S: GroupingStrategy + ?Sized>(
    processes: &[ProcessInfo],
    strategy: &S,
) -> Vec<ProcessGroup> {
    strategy.group(processes)
}

pub fn group_processes(processes: &[ProcessInfo]) -> Vec<ProcessGroup> {
    let cgroup_groups = group_processes_with_strategy(processes, &CgroupGrouping);
    if cgroup_grouping_is_degenerate(processes, &cgroup_groups) {
        let lineage_groups = group_processes_with_strategy(processes, &ParentLineageGrouping);
        if !lineage_groups.is_empty() {
            return lineage_groups;
        }
    }

    let split_groups = split_low_signal_cgroup_groups(processes, &cgroup_groups);
    if let Some(groups) = split_groups {
        return groups;
    }

    cgroup_groups
}

pub fn pid_to_group_map(groups: &[ProcessGroup]) -> HashMap<u32, String> {
    let mut map = HashMap::new();

    for group in groups {
        for pid in &group.pids {
            map.entry(*pid).or_insert_with(|| group.id.clone());
        }
    }

    map
}

pub fn tracked_pids(groups: &[ProcessGroup]) -> Vec<u32> {
    let mut pids: Vec<u32> = groups
        .iter()
        .flat_map(|group| group.pids.iter().copied())
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn read_cgroup_path(pid: u32) -> Option<String> {
    let contents = fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = select_cgroup_path(&contents);
    if path.is_empty() { None } else { Some(path) }
}

fn select_cgroup_path(contents: &str) -> String {
    let mut best: Option<(u8, usize, String)> = None;

    for (index, line) in contents.lines().enumerate() {
        let Some(path) = parse_cgroup_line(line) else {
            continue;
        };

        let priority = cgroup_label(&path).map_or(0, |label| label.priority);
        let replace_best = match &best {
            Some((best_priority, _, _)) => priority > *best_priority,
            None => true,
        };

        if replace_best {
            best = Some((priority, index, path));
        }
    }

    best.map(|(_, _, path)| path).unwrap_or_default()
}

fn parse_cgroup_line(line: &str) -> Option<String> {
    let mut fields = line.splitn(3, ':');
    fields.next()?;
    fields.next()?;
    let path = fields.next()?.trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn read_proc_stat_ids(pid: u32) -> Option<ProcStatIds> {
    let contents = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_proc_stat_ids(&contents)
}

#[cfg(test)]
fn parse_parent_pid_from_stat(contents: &str) -> Option<u32> {
    parse_proc_stat_ids(contents).and_then(|ids| ids.parent_pid)
}

fn parse_proc_stat_ids(contents: &str) -> Option<ProcStatIds> {
    let comm_end = contents.rfind(')')?;
    let fields: Vec<&str> = contents[comm_end + 2..].split_whitespace().collect();
    Some(ProcStatIds {
        parent_pid: fields.get(1).and_then(|value| value.parse().ok()),
        process_group_id: fields.get(2).and_then(|value| value.parse().ok()),
        session_id: fields.get(3).and_then(|value| value.parse().ok()),
    })
}

fn read_process_command(pid: u32) -> String {
    let command = fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|contents| command_from_cmdline(&contents))
        .unwrap_or_default();
    if !command.is_empty() {
        return command;
    }

    fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|name| name.trim().to_string())
        .unwrap_or_else(|_| format!("pid {pid}"))
}

fn command_from_cmdline(contents: &[u8]) -> String {
    contents
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_process_user(pid: u32, users_cache: &UsersCache) -> String {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|contents| parse_uid_from_status(&contents))
        .map(|uid| resolve_username(uid, users_cache))
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_uid_from_status(contents: &str) -> Option<u32> {
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != "Uid:" {
            return None;
        }
        fields.next()?.parse().ok()
    })
}

fn resolve_username(uid: u32, users_cache: &UsersCache) -> String {
    users_cache
        .get_user_by_uid(uid)
        .map(|user| user.name().to_string_lossy().to_string())
        .unwrap_or_else(|| uid.to_string())
}

fn cgroup_grouping_is_degenerate(processes: &[ProcessInfo], groups: &[ProcessGroup]) -> bool {
    if processes.is_empty() {
        return false;
    }

    if groups.is_empty() {
        return true;
    }

    if groups.len() != 1 {
        return false;
    }

    let labels: Vec<CgroupLabel> = processes
        .iter()
        .filter_map(|process| cgroup_label(&process.cgroup_path))
        .collect();

    if labels.is_empty() {
        return true;
    }

    let unique_paths: HashSet<&str> = processes
        .iter()
        .map(|process| process.cgroup_path.trim())
        .filter(|path| !path.is_empty())
        .collect();
    let only_low_signal_labels = labels.iter().all(|label| label.priority < 3);

    if unique_paths.len() <= 1 && only_low_signal_labels {
        return true;
    }

    unique_paths.len() <= 1 && ParentLineageGrouping.group(processes).len() > 1
}

fn split_low_signal_cgroup_groups(
    processes: &[ProcessInfo],
    groups: &[ProcessGroup],
) -> Option<Vec<ProcessGroup>> {
    let by_pid: HashMap<u32, &ProcessInfo> = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect();
    let mut split_any = false;
    let mut output = Vec::new();

    for group in groups {
        let group_processes: Vec<ProcessInfo> = group
            .pids
            .iter()
            .filter_map(|pid| by_pid.get(pid).map(|process| (*process).clone()))
            .collect();

        if cgroup_group_should_split_by_process_group(&group_processes) {
            let process_group_groups = ProcessGroupIdGrouping.group(&group_processes);
            if process_group_groups.len() > 1 {
                split_any = true;
                output.extend(process_group_groups);
                continue;
            }
        }

        output.push(group.clone());
    }

    if split_any {
        output.sort_by(|left, right| left.id.cmp(&right.id));
        Some(output)
    } else {
        None
    }
}

fn cgroup_group_should_split_by_process_group(processes: &[ProcessInfo]) -> bool {
    if processes.len() <= 1 {
        return false;
    }

    let labels: Vec<CgroupLabel> = processes
        .iter()
        .filter_map(|process| cgroup_label(&process.cgroup_path))
        .collect();
    if labels.is_empty() || !labels.iter().all(cgroup_label_is_low_signal) {
        return false;
    }

    ProcessGroupIdGrouping.group(processes).len() > 1
}

fn cgroup_label_is_low_signal(label: &CgroupLabel) -> bool {
    label.priority < 3 || is_interactive_user_scope_name(&label.name)
}

fn is_interactive_user_scope_name(name: &str) -> bool {
    let stem = name.strip_suffix(".scope").unwrap_or(name);
    const PREFIXES: [&str; 4] = ["app-", "session-", "vte-spawn-", "tmux-spawn-"];
    PREFIXES.iter().any(|prefix| stem.starts_with(prefix))
}

fn cgroup_label(path: &str) -> Option<CgroupLabel> {
    let segments = cgroup_segments(path);
    if segments.is_empty() {
        return None;
    }

    for segment in segments.iter().rev() {
        if is_systemd_unit_or_scope_segment(segment) && !is_generic_systemd_segment(segment) {
            let name = normalize_systemd_segment(segment);
            return Some(CgroupLabel {
                key: name.clone(),
                name,
                priority: 3,
            });
        }
    }

    for segment in segments.iter().rev() {
        if segment.ends_with(".slice") && !is_generic_systemd_segment(segment) {
            let name = normalize_systemd_segment(segment);
            return Some(CgroupLabel {
                key: name.clone(),
                name,
                priority: 2,
            });
        }
    }

    segments
        .iter()
        .rev()
        .find(|segment| meaningful_path_segment(segment))
        .map(|segment| {
            let name = normalize_path_segment(segment);
            CgroupLabel {
                key: name.clone(),
                name,
                priority: 1,
            }
        })
}

fn cgroup_segments(path: &str) -> Vec<&str> {
    path.trim_matches('/')
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn unscoped_label() -> CgroupLabel {
    CgroupLabel {
        key: "unscoped".to_string(),
        name: "unscoped".to_string(),
        priority: 0,
    }
}

fn is_systemd_unit_or_scope_segment(segment: &str) -> bool {
    const SUFFIXES: [&str; 9] = [
        ".service", ".scope", ".socket", ".timer", ".mount", ".target", ".path", ".device", ".swap",
    ];

    SUFFIXES.iter().any(|suffix| segment.ends_with(suffix))
}

fn is_generic_systemd_segment(segment: &str) -> bool {
    matches!(
        segment,
        "-.slice" | "system.slice" | "user.slice" | "machine.slice" | "init.scope"
    ) || (segment.starts_with("user-") && segment.ends_with(".slice"))
        || (segment.starts_with("user@") && segment.ends_with(".service"))
}

fn meaningful_path_segment(segment: &str) -> bool {
    !matches!(segment, "." | "..") && !is_generic_systemd_segment(segment)
}

fn normalize_systemd_segment(segment: &str) -> String {
    let Some((base, suffix)) = split_systemd_suffix(segment) else {
        return normalize_path_segment(segment);
    };
    let base = strip_volatile_suffixes(base);
    if base.is_empty() {
        segment.to_string()
    } else {
        format!("{base}.{suffix}")
    }
}

fn normalize_path_segment(segment: &str) -> String {
    let segment = segment.trim();
    let normalized = strip_volatile_suffixes(segment);
    if normalized.is_empty() {
        segment.to_string()
    } else {
        normalized
    }
}

fn split_systemd_suffix(segment: &str) -> Option<(&str, &str)> {
    let (base, suffix) = segment.rsplit_once('.')?;
    if base.is_empty() || suffix.is_empty() {
        None
    } else {
        Some((base, suffix))
    }
}

fn strip_volatile_suffixes(value: &str) -> String {
    if !should_strip_volatile_suffixes(value) {
        return value.to_string();
    }

    let mut parts: Vec<&str> = value.split('-').collect();

    while parts.len() > 1 {
        let Some(last) = parts.last() else {
            break;
        };
        if is_volatile_token(last) {
            parts.pop();
        } else {
            break;
        }
    }

    parts.join("-")
}

fn should_strip_volatile_suffixes(value: &str) -> bool {
    const PREFIXES: [&str; 3] = ["app-", "session-", "vte-spawn-"];
    PREFIXES.iter().any(|prefix| value.starts_with(prefix))
}

fn is_volatile_token(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }

    token.chars().all(|char| char.is_ascii_digit())
        || (token.len() >= 8 && token.chars().all(|char| char.is_ascii_hexdigit()))
        || looks_like_uuid_token(token)
        || looks_like_pod_token(token)
}

fn looks_like_uuid_token(token: &str) -> bool {
    let without_hyphens: String = token.chars().filter(|char| *char != '-').collect();
    without_hyphens.len() >= 16 && without_hyphens.chars().all(|char| char.is_ascii_hexdigit())
}

fn looks_like_pod_token(token: &str) -> bool {
    token
        .strip_prefix("pod")
        .is_some_and(|rest| rest.len() >= 8 && rest.chars().all(|char| char.is_ascii_hexdigit()))
}

fn process_index(processes: &[ProcessInfo]) -> BTreeMap<u32, &ProcessInfo> {
    let mut by_pid = BTreeMap::new();
    for process in processes {
        by_pid.entry(process.pid).or_insert(process);
    }
    by_pid
}

fn lineage_representative(pid: u32, by_pid: &BTreeMap<u32, &ProcessInfo>) -> u32 {
    let mut current = pid;
    let mut visited = BTreeSet::new();

    while visited.insert(current) {
        let Some(process) = by_pid.get(&current) else {
            return current;
        };
        let Some(parent_pid) = process.parent_pid else {
            return current;
        };

        if parent_pid == 1 || parent_pid == 2 || !by_pid.contains_key(&parent_pid) {
            return current;
        }

        current = parent_pid;
    }

    pid
}

fn build_group(
    id: String,
    name: String,
    processes: &[&ProcessInfo],
    representative_pid: Option<u32>,
) -> Option<ProcessGroup> {
    let mut pids: Vec<u32> = processes.iter().map(|process| process.pid).collect();
    pids.sort_unstable();
    pids.dedup();

    let representative_pid = representative_pid.or_else(|| pids.first().copied())?;

    Some(ProcessGroup {
        id,
        name,
        user: common_user(processes),
        pids,
        representative_pid,
    })
}

fn common_user(processes: &[&ProcessInfo]) -> String {
    let users: BTreeSet<&str> = processes
        .iter()
        .map(|process| process.user.as_str())
        .filter(|user| !user.is_empty())
        .collect();

    match users.len() {
        0 => "unknown".to_string(),
        1 => users.iter().next().unwrap().to_string(),
        _ => "mixed".to_string(),
    }
}

fn command_name(command: &str, pid: u32) -> String {
    let first = command.split_whitespace().next().unwrap_or_default();
    if first.is_empty() {
        return format!("pid {pid}");
    }

    Path::new(first)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(first)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(
        pid: u32,
        parent_pid: Option<u32>,
        command: &str,
        user: &str,
        cgroup_path: &str,
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid,
            process_group_id: Some(pid),
            session_id: Some(pid),
            command: command.to_string(),
            user: user.to_string(),
            cgroup_path: cgroup_path.to_string(),
        }
    }

    fn process_with_process_group(
        pid: u32,
        parent_pid: Option<u32>,
        process_group_id: u32,
        command: &str,
        user: &str,
        cgroup_path: &str,
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid,
            process_group_id: Some(process_group_id),
            session_id: Some(process_group_id),
            command: command.to_string(),
            user: user.to_string(),
            cgroup_path: cgroup_path.to_string(),
        }
    }

    #[test]
    fn cgroup_grouping_uses_unit_and_scope_labels() {
        let processes = vec![
            process(100, Some(1), "nginx", "root", "/system.slice/nginx.service"),
            process(
                101,
                Some(100),
                "nginx",
                "root",
                "/system.slice/nginx.service",
            ),
            process(200, Some(1), "sshd", "root", "/system.slice/sshd.service"),
            process(
                300,
                Some(1),
                "gnome-terminal",
                "alice",
                "/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.gnome.Terminal-1122.scope",
            ),
            process(
                301,
                Some(300),
                "zsh",
                "alice",
                "/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.gnome.Terminal-3344.scope",
            ),
        ];

        let groups = CgroupGrouping.group(&processes);
        let ids: Vec<&str> = groups.iter().map(|group| group.id.as_str()).collect();

        assert_eq!(
            ids,
            vec![
                "cgroup:app-org.gnome.Terminal.scope",
                "cgroup:nginx.service",
                "cgroup:sshd.service"
            ]
        );
        assert_eq!(groups[0].pids, vec![300, 301]);
        assert_eq!(groups[1].pids, vec![100, 101]);
        assert_eq!(groups[1].representative_pid, 100);
    }

    #[test]
    fn parent_lineage_grouping_uses_descendant_below_pid_one_or_two() {
        let processes = vec![
            process(1, None, "systemd", "root", "/"),
            process(2, None, "kthreadd", "root", "/"),
            process(100, Some(1), "bash", "alice", ""),
            process(101, Some(100), "python workload.py", "alice", ""),
            process(102, Some(101), "worker", "alice", ""),
            process(200, Some(2), "kworker", "root", ""),
        ];

        let groups = ParentLineageGrouping.group(&processes);
        let workload = groups
            .iter()
            .find(|group| group.id == "lineage:100")
            .expect("user workload should be grouped below PID 1");

        assert_eq!(workload.name, "bash");
        assert_eq!(workload.representative_pid, 100);
        assert_eq!(workload.pids, vec![100, 101, 102]);
        assert!(!groups.iter().any(|group| {
            group.id == "lineage:1" && group.pids.iter().any(|pid| *pid == 100 || *pid == 101)
        }));
        assert!(groups.iter().any(|group| group.id == "lineage:200"));
        assert!(!groups.iter().any(|group| group.id == "lineage:1"));
        assert!(!groups.iter().any(|group| group.id == "lineage:2"));
    }

    #[test]
    fn group_processes_falls_back_from_single_degenerate_cgroup() {
        let processes = vec![
            process(100, Some(1), "bash", "alice", "/"),
            process(101, Some(100), "python", "alice", "/"),
            process(200, Some(1), "node", "alice", "/"),
        ];

        let groups = group_processes(&processes);
        let ids: Vec<&str> = groups.iter().map(|group| group.id.as_str()).collect();

        assert_eq!(ids, vec!["lineage:100", "lineage:200"]);
        assert_eq!(groups[0].pids, vec![100, 101]);
        assert_eq!(groups[1].pids, vec![200]);
    }

    #[test]
    fn group_processes_falls_back_from_single_low_signal_user_slice() {
        let processes = vec![
            process(1, None, "systemd", "root", "/"),
            process(100, Some(1), "python api.py", "alice", "/user.slice"),
            process(101, Some(100), "worker", "alice", "/user.slice"),
            process(200, Some(1), "node server.js", "alice", "/user.slice"),
        ];

        let groups = group_processes(&processes);
        let ids: Vec<&str> = groups.iter().map(|group| group.id.as_str()).collect();

        assert_eq!(ids, vec!["lineage:100", "lineage:200"]);
        assert_eq!(groups[0].name, "python");
        assert_eq!(groups[0].pids, vec![100, 101]);
        assert_eq!(groups[1].name, "node");
        assert_eq!(groups[1].pids, vec![200]);
    }

    #[test]
    fn group_processes_splits_low_signal_tmux_scope_by_process_group() {
        let cgroup = "/user.slice/user-1000.slice/user@1000.service/app.slice/tmux-spawn-c62b4263-07fc-4c42-9cc5-4dda073993ce.scope";
        let processes = vec![
            process_with_process_group(100, Some(1), 100, "zsh", "alice", cgroup),
            process_with_process_group(200, Some(100), 200, "emt --tui", "alice", cgroup),
            process_with_process_group(201, Some(200), 200, "emt helper", "alice", cgroup),
            process_with_process_group(300, Some(100), 300, "python workload.py", "alice", cgroup),
        ];

        let groups = group_processes(&processes);
        let ids: Vec<&str> = groups.iter().map(|group| group.id.as_str()).collect();

        assert_eq!(ids, vec!["pgrp:100", "pgrp:200", "pgrp:300"]);
        assert_eq!(groups[1].name, "emt");
        assert_eq!(groups[1].pids, vec![200, 201]);
        assert_eq!(groups[2].name, "python");
        assert_eq!(groups[2].pids, vec![300]);
    }

    #[test]
    fn scanner_cgroup_parser_prefers_specific_systemd_path() {
        let contents = "\
12:memory:/
11:cpu,cpuacct:/system.slice/ssh.service
0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-firefox-98765.scope
";

        assert_eq!(
            select_cgroup_path(contents),
            "/system.slice/ssh.service".to_string()
        );
        assert_eq!(
            cgroup_label(
                "/user.slice/user-1000.slice/user@1000.service/app.slice/app-firefox-98765.scope"
            )
            .expect("scope label")
            .name,
            "app-firefox.scope"
        );
    }

    #[test]
    fn scanner_stat_parser_reads_parent_pid_after_comm() {
        let stat = "123 (python worker) S 42 1 1 0 0 0";

        assert_eq!(parse_parent_pid_from_stat(stat), Some(42));
    }

    #[test]
    fn scanner_stat_parser_reads_process_group_and_session_ids() {
        let stat = "123 (python worker) S 42 200 300 0 0 0";

        assert_eq!(
            parse_proc_stat_ids(stat),
            Some(ProcStatIds {
                parent_pid: Some(42),
                process_group_id: Some(200),
                session_id: Some(300),
            })
        );
    }

    #[test]
    fn scanner_cmdline_parser_joins_nul_separated_args() {
        let cmdline = b"/usr/bin/python\0script.py\0--flag\0";

        assert_eq!(
            command_from_cmdline(cmdline),
            "/usr/bin/python script.py --flag"
        );
    }

    #[test]
    fn scanner_status_parser_reads_real_uid() {
        let status = "Name:\tpython\nUid:\t1000\t1000\t1000\t1000\n";

        assert_eq!(parse_uid_from_status(status), Some(1000));
    }

    #[test]
    fn cgroup_labels_preserve_container_and_pod_identity() {
        assert_eq!(
            cgroup_label("/system.slice/docker-0123456789abcdef.scope")
                .expect("docker scope")
                .name,
            "docker-0123456789abcdef.scope"
        );
        assert_eq!(
            cgroup_label("/system.slice/cri-containerd-fedcba9876543210.scope")
                .expect("containerd scope")
                .name,
            "cri-containerd-fedcba9876543210.scope"
        );
        assert_eq!(
            cgroup_label("/kubepods.slice/kubepods-besteffort-pod0123456789abcdef.slice")
                .expect("pod slice")
                .name,
            "kubepods-besteffort-pod0123456789abcdef.slice"
        );
    }

    struct SingleGroupStrategy;

    impl GroupingStrategy for SingleGroupStrategy {
        fn group(&self, processes: &[ProcessInfo]) -> Vec<ProcessGroup> {
            let pids: Vec<u32> = processes.iter().map(|process| process.pid).collect();
            vec![ProcessGroup {
                id: "strategy:single".to_string(),
                name: format!("{} processes", processes.len()),
                user: "synthetic".to_string(),
                representative_pid: pids.first().copied().unwrap_or_default(),
                pids,
            }]
        }
    }

    #[test]
    fn grouping_with_strategy_delegates_to_injected_strategy() {
        let processes = vec![
            process(100, Some(1), "bash", "alice", "/system.slice/a.service"),
            process(200, Some(1), "node", "bob", "/system.slice/b.service"),
        ];

        let groups = group_processes_with_strategy(&processes, &SingleGroupStrategy);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "strategy:single");
        assert_eq!(groups[0].name, "2 processes");
        assert_eq!(groups[0].pids, vec![100, 200]);
    }

    #[test]
    fn scanner_delegates_to_injected_strategy() {
        let groups = scan_process_groups(&SingleGroupStrategy);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "strategy:single");
        assert!(!groups[0].pids.is_empty());
    }

    #[test]
    fn helpers_return_deterministic_deduplicated_pids() {
        let groups = vec![
            ProcessGroup {
                id: "b".to_string(),
                name: "b".to_string(),
                user: "alice".to_string(),
                pids: vec![3, 1, 3],
                representative_pid: 1,
            },
            ProcessGroup {
                id: "a".to_string(),
                name: "a".to_string(),
                user: "alice".to_string(),
                pids: vec![2, 1],
                representative_pid: 2,
            },
        ];

        let map = pid_to_group_map(&groups);
        assert_eq!(map.get(&1), Some(&"b".to_string()));
        assert_eq!(map.get(&2), Some(&"a".to_string()));
        assert_eq!(map.get(&3), Some(&"b".to_string()));
        assert_eq!(tracked_pids(&groups), vec![1, 2, 3]);
    }
}
