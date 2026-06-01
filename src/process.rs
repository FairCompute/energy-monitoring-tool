use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use sysinfo::{Process, System};
use users::{Users, UsersCache};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CgroupLabel {
    key: String,
    name: String,
    priority: u8,
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

pub fn scan_processes() -> Vec<ProcessInfo> {
    let system = System::new_all();
    let users_cache = UsersCache::new();
    let mut processes: Vec<ProcessInfo> = system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let pid = pid.as_u32();
            ProcessInfo {
                pid,
                parent_pid: process.parent().map(|parent| parent.as_u32()),
                command: process_command(process),
                user: process_user(process, &users_cache),
                cgroup_path: read_cgroup_path(pid).unwrap_or_default(),
            }
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

fn process_command(process: &Process) -> String {
    let command = process
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if command.is_empty() {
        process.name().to_string_lossy().to_string()
    } else {
        command
    }
}

fn process_user(process: &Process, users_cache: &UsersCache) -> String {
    process
        .user_id()
        .map(|uid| resolve_username(**uid, users_cache))
        .unwrap_or_else(|| "unknown".to_string())
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
