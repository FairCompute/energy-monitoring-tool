use crate::monitor::{DeviceSource, MetricsSnapshot};
use crate::tui::App;
use crate::tui::app::{PowerHistorySnapshot, SortMode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Sparkline, Table};
use std::collections::{HashMap, HashSet};

pub fn render(frame: &mut Frame, app: &App) {
    let snapshot = app.snapshot();
    let uptime = app.uptime_secs();
    let display_elapsed = app.display_elapsed_secs();
    let power_history = app.power_history();
    let sort_mode = app.sort_mode();
    let selected_group_index = app.selected_group_index();
    let expanded_group_ids = app.expanded_group_ids();
    let child_scroll_offsets = app.child_scroll_offsets();

    render_snapshot(
        frame,
        &snapshot,
        uptime,
        display_elapsed,
        &power_history,
        sort_mode,
        selected_group_index,
        expanded_group_ids,
        child_scroll_offsets,
    );
}

fn render_snapshot(
    frame: &mut Frame,
    snapshot: &MetricsSnapshot,
    uptime: f64,
    display_elapsed: f64,
    power_history: &PowerHistorySnapshot,
    sort_mode: SortMode,
    selected_group_index: usize,
    expanded_group_ids: &HashSet<String>,
    child_scroll_offsets: &HashMap<String, usize>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(
        frame,
        chunks[0],
        snapshot,
        uptime,
        display_elapsed,
        power_history,
    );
    render_body(
        frame,
        chunks[1],
        snapshot,
        selected_group_index,
        expanded_group_ids,
        child_scroll_offsets,
    );
    render_footer(frame, chunks[2], sort_mode);
}

fn render_header(
    frame: &mut Frame,
    area: Rect,
    snapshot: &MetricsSnapshot,
    uptime: f64,
    display_elapsed: f64,
    power_history: &PowerHistorySnapshot,
) {
    let total_energy = snapshot.system_total.total();
    let power = if display_elapsed > 0.0 {
        total_energy / display_elapsed
    } else {
        0.0
    };

    let mins = (uptime as u64) / 60;
    let secs = (uptime as u64) % 60;

    let mut device_line = vec![
        Span::styled("    CPU: ", Style::default().fg(Color::Yellow)),
        Span::raw(format!("{:.4} J", snapshot.system_total.cpu_joules)),
    ];
    append_dram_header(
        &mut device_line,
        snapshot.sources.dram,
        snapshot.system_total.dram_joules,
    );
    if snapshot.gpu_available {
        device_line.extend([
            Span::raw("    "),
            Span::styled("GPU: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{:.4} J", snapshot.system_total.gpu_joules)),
        ]);
    }

    let lines = vec![
        Line::from(vec![
            Span::styled("  Avg Power: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{power:.2} W")),
            Span::raw("    "),
            Span::styled("Energy: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{total_energy:.4} J")),
        ]),
        Line::from(device_line),
        Line::from(vec![
            Span::styled("Uptime: ", Style::default().fg(Color::Green)),
            Span::raw(format!("{mins:02}:{secs:02}")),
            Span::raw(format!("    Tracked PIDs: {}", snapshot.tracked_pids.len())),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" EMT - Energy Monitoring Tool ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let header_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(Paragraph::new(lines), header_chunks[0]);
    if power_history.has_samples() {
        render_power_history(
            frame,
            header_chunks[1],
            header_chunks[2],
            power_history,
            snapshot.sources.dram,
            snapshot.gpu_available,
        );
    }
}

fn append_dram_header(device_line: &mut Vec<Span>, source: DeviceSource, dram_joules: f64) {
    device_line.push(Span::raw("    "));

    match source {
        DeviceSource::Measured => {
            device_line.extend([
                Span::styled("DRAM: ", Style::default().fg(Color::Yellow)),
                Span::raw(format!("{dram_joules:.4} J")),
            ]);
        }
        DeviceSource::IncludedInPackage | DeviceSource::MeasuredPackage => {
            device_line.extend(disabled_dram_spans("-- (included in CPU)"));
        }
        DeviceSource::Unavailable => {
            device_line.extend(disabled_dram_spans("-- (unavailable)"));
        }
    }
}

fn disabled_dram_spans(value: &'static str) -> [Span<'static>; 2] {
    [
        Span::styled("DRAM: ", disabled_style()),
        Span::styled(value, disabled_style()),
    ]
}

fn disabled_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn render_power_history(
    frame: &mut Frame,
    label_area: Rect,
    sparkline_area: Rect,
    power_history: &PowerHistorySnapshot,
    dram_source: DeviceSource,
    gpu_available: bool,
) {
    let label_chunks = split_power_columns(label_area, gpu_available);
    let sparkline_chunks = split_power_columns(sparkline_area, gpu_available);

    let mut column = 0;
    render_power_label(
        frame,
        label_chunks[column],
        "CPU",
        power_history.latest_cpu(),
        Color::Yellow,
    );
    render_component_sparkline(
        frame,
        sparkline_chunks[column],
        &power_history.cpu,
        Color::Yellow,
    );
    column += 1;

    render_dram_power_history(
        frame,
        label_chunks[column],
        sparkline_chunks[column],
        power_history,
        dram_source,
    );
    column += 1;

    if gpu_available {
        render_power_label(
            frame,
            label_chunks[column],
            "GPU",
            power_history.latest_gpu(),
            Color::Green,
        );
        render_component_sparkline(
            frame,
            sparkline_chunks[column],
            &power_history.gpu,
            Color::Green,
        );
    }
}

fn render_dram_power_history(
    frame: &mut Frame,
    label_area: Rect,
    sparkline_area: Rect,
    power_history: &PowerHistorySnapshot,
    source: DeviceSource,
) {
    match source {
        DeviceSource::Measured => {
            render_power_label(
                frame,
                label_area,
                "DRAM",
                power_history.latest_dram(),
                Color::Magenta,
            );
            render_component_sparkline(frame, sparkline_area, &power_history.dram, Color::Magenta);
        }
        DeviceSource::IncludedInPackage | DeviceSource::MeasuredPackage => {
            render_disabled_power_label(
                frame,
                label_area,
                disabled_dram_power_label(label_area.width, "in CPU"),
            );
        }
        DeviceSource::Unavailable => {
            render_disabled_power_label(
                frame,
                label_area,
                disabled_dram_power_label(label_area.width, "unavailable"),
            );
        }
    }
}

fn split_power_columns(area: Rect, gpu_available: bool) -> std::rc::Rc<[Rect]> {
    let column_count = 2 + usize::from(gpu_available);
    let percentage = 100 / column_count as u16;
    let mut constraints = vec![Constraint::Percentage(percentage); column_count];
    if let Some(last) = constraints.last_mut() {
        *last = Constraint::Percentage(100 - percentage * (column_count as u16 - 1));
    }

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area)
}

fn render_power_label(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    latest_watts: Option<f64>,
    color: Color,
) {
    let value = latest_watts
        .map(|watts| format!("{watts:.2} W"))
        .unwrap_or_else(|| "--".to_string());
    let label = Paragraph::new(Line::from(vec![Span::styled(
        format!("{label}: {value}"),
        Style::default().fg(color),
    )]));
    frame.render_widget(label, area);
}

fn render_disabled_power_label(frame: &mut Frame, area: Rect, text: String) {
    let label = Paragraph::new(Line::from(Span::styled(text, disabled_style())));
    frame.render_widget(label, area);
}

fn disabled_dram_power_label(width: u16, detail: &str) -> String {
    let width = usize::from(width);
    let label = format!("DRAM: -- ({detail})");
    if label.len() <= width {
        return label;
    }

    "DRAM: --".to_string()
}

fn render_component_sparkline(frame: &mut Frame, area: Rect, samples: &[f64], color: Color) {
    if samples.is_empty() {
        return;
    }

    let data = sparkline_data(samples);
    let max = data.iter().copied().max().unwrap_or(1).max(1);
    let sparkline = Sparkline::default()
        .data(data)
        .max(max)
        .style(Style::default().fg(color));
    frame.render_widget(sparkline, area);
}

fn sparkline_data(samples: &[f64]) -> Vec<u64> {
    samples
        .iter()
        .map(|watts| power_to_sparkline_value(*watts))
        .collect()
}

fn power_to_sparkline_value(watts: f64) -> u64 {
    if !watts.is_finite() || watts <= 0.0 {
        return 0;
    }

    let milliwatts = (watts * 1_000.0).round().max(1.0);
    if milliwatts >= u64::MAX as f64 {
        u64::MAX
    } else {
        milliwatts as u64
    }
}

fn render_body(
    frame: &mut Frame,
    area: Rect,
    snapshot: &MetricsSnapshot,
    selected_group_index: usize,
    expanded_group_ids: &HashSet<String>,
    child_scroll_offsets: &HashMap<String, usize>,
) {
    if snapshot.workloads.is_empty() {
        let block = Block::default().borders(Borders::ALL).title(" Workloads ");
        let paragraph = Paragraph::new("  No process data yet...").block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec![
        "Group",
        "User",
        "Energy (J)",
        "Avg Power (W)",
        "% Total",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let rows = workload_table_rows(
        snapshot,
        selected_group_index,
        expanded_group_ids,
        child_scroll_offsets,
    );
    let rows = visible_table_rows(rows, selected_group_index, table_body_capacity(area))
        .into_iter()
        .map(WorkloadTableRow::into_row)
        .collect::<Vec<_>>();

    let table = Table::new(
        rows,
        [
            Constraint::Min(24),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(" Workloads "));

    frame.render_widget(table, area);
}

fn render_footer(frame: &mut Frame, area: Rect, sort_mode: SortMode) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Red)),
        Span::raw(" quit"),
        Span::styled("  up/down", Style::default().fg(Color::Cyan)),
        Span::raw(" move"),
        Span::styled("  enter", Style::default().fg(Color::Cyan)),
        Span::raw(" expand"),
        Span::styled("  esc", Style::default().fg(Color::Cyan)),
        Span::raw(" collapse"),
        Span::styled("  s", Style::default().fg(Color::Cyan)),
        Span::raw(" sort "),
        Span::styled(sort_mode.label(), Style::default().fg(Color::Yellow)),
        Span::styled("  r", Style::default().fg(Color::Cyan)),
        Span::raw(" reset"),
    ]));
    frame.render_widget(footer, area);
}

struct WorkloadTableRow {
    cells: Vec<String>,
    style: Style,
    group_index: Option<usize>,
}

impl WorkloadTableRow {
    fn into_row(self) -> Row<'static> {
        Row::new(self.cells).style(self.style)
    }
}

fn workload_table_rows(
    snapshot: &MetricsSnapshot,
    selected_group_index: usize,
    expanded_group_ids: &HashSet<String>,
    child_scroll_offsets: &HashMap<String, usize>,
) -> Vec<WorkloadTableRow> {
    let mut rows = Vec::new();

    for (group_index, workload) in snapshot.workloads.iter().enumerate() {
        let is_expanded = expanded_group_ids.contains(&workload.group_id);
        let disclosure = if !workload.is_live {
            "x "
        } else if workload.processes.is_empty() {
            "  "
        } else if is_expanded {
            "v "
        } else {
            "> "
        };
        let style = if group_index == selected_group_index {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if !workload.is_live {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };

        rows.push(WorkloadTableRow {
            cells: vec![
                format!("{disclosure}{}", workload.name),
                workload.user.clone(),
                format!("{:.4}", workload.energy.total()),
                format!("{:.2}", workload.power_watts),
                format!("{:.1}%", workload.percentage_of_system),
            ],
            style,
            group_index: Some(group_index),
        });

        if workload.is_live && is_expanded {
            let offset = child_scroll_offsets
                .get(&workload.group_id)
                .copied()
                .unwrap_or_default()
                .min(workload.processes.len().saturating_sub(1));
            for process in workload.processes.iter().skip(offset) {
                rows.push(WorkloadTableRow {
                    cells: vec![
                        format!("  pid {} {}", process.pid, process.name),
                        String::new(),
                        format!("{:.4}", process.energy.total()),
                        format!("{:.2}", process.power_watts),
                        String::new(),
                    ],
                    style: Style::default().fg(Color::DarkGray),
                    group_index: None,
                });
            }
        }
    }

    rows
}

fn visible_table_rows(
    rows: Vec<WorkloadTableRow>,
    selected_group_index: usize,
    capacity: usize,
) -> Vec<WorkloadTableRow> {
    if rows.len() <= capacity {
        return rows;
    }

    let selected_row_index = rows
        .iter()
        .position(|row| row.group_index == Some(selected_group_index))
        .unwrap_or_default();
    let selected_group_has_children = rows
        .get(selected_row_index + 1)
        .is_some_and(|row| row.group_index.is_none());
    let start = if selected_group_has_children {
        selected_row_index.min(rows.len().saturating_sub(capacity))
    } else {
        selected_row_index
            .saturating_add(1)
            .saturating_sub(capacity)
    };

    rows.into_iter().skip(start).take(capacity).collect()
}

fn table_body_capacity(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(3)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{DeviceEnergy, DeviceSource, DeviceSources, WorkloadSnapshot};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn sources(cpu: DeviceSource, dram: DeviceSource, gpu: DeviceSource) -> DeviceSources {
        DeviceSources { cpu, dram, gpu }
    }

    #[test]
    fn sparkline_data_uses_milliwatts_to_keep_sub_watt_values_visible() {
        assert_eq!(sparkline_data(&[0.0, 0.001, 1.25]), vec![0, 1, 1_250]);
    }

    #[test]
    fn sparkline_data_drops_invalid_or_negative_values_to_zero() {
        assert_eq!(
            sparkline_data(&[f64::NAN, f64::INFINITY, -1.0]),
            vec![0, 0, 0]
        );
    }

    #[test]
    fn render_snapshot_shows_average_and_interval_power_labels() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: false,
            sources: sources(
                DeviceSource::MeasuredPackage,
                DeviceSource::Measured,
                DeviceSource::Unavailable,
            ),
            system_total: DeviceEnergy {
                cpu_joules: 180.0,
                dram_joules: 60.0,
                gpu_joules: 0.0,
            },
            workloads: vec![WorkloadSnapshot {
                root_pid: 123,
                group_id: "pid:123".to_string(),
                name: "python workload.py".to_string(),
                user: "alice".to_string(),
                processes: Vec::new(),
                is_live: true,
                energy: DeviceEnergy {
                    cpu_joules: 120.0,
                    dram_joules: 30.0,
                    gpu_joules: 0.0,
                },
                power_watts: 2.5,
                percentage_of_system: 62.5,
            }],
            unattributed: DeviceEnergy {
                cpu_joules: 60.0,
                dram_joules: 30.0,
                gpu_joules: 0.0,
            },
            tracked_pids: vec![123, 124],
            ..MetricsSnapshot::default()
        };
        let power_history = PowerHistorySnapshot {
            cpu: vec![5.0],
            dram: vec![1.5],
            gpu: vec![],
        };
        let expanded = HashSet::new();
        let child_scroll_offsets = HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();

        terminal
            .draw(|frame| {
                render_snapshot(
                    frame,
                    &snapshot,
                    60.0,
                    60.0,
                    &power_history,
                    SortMode::Energy,
                    0,
                    &expanded,
                    &child_scroll_offsets,
                )
            })
            .unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("Avg Power: 4.00 W"));
        assert!(screen.contains("CPU: 5.00 W"));
        assert!(screen.contains("DRAM: 1.50 W"));
        assert!(!screen.contains("GPU:"));
        assert!(!screen.contains("GPU: 3.00 W"));
        assert!(screen.contains("Avg Power (W)"));
        assert!(screen.contains("2.50"));
        assert!(screen.contains("python workload.py"));
        assert!(screen.contains("Tracked PIDs: 2"));
        assert!(screen.contains("sort energy"));
        assert!(screen.contains("reset"));
    }

    #[test]
    fn render_snapshot_grays_dram_included_in_package_without_dram_metric() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: false,
            sources: sources(
                DeviceSource::MeasuredPackage,
                DeviceSource::IncludedInPackage,
                DeviceSource::Unavailable,
            ),
            system_total: DeviceEnergy {
                cpu_joules: 180.0,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
            tracked_pids: vec![123],
            ..MetricsSnapshot::default()
        };
        let power_history = PowerHistorySnapshot {
            cpu: vec![5.0],
            dram: vec![1.5],
            gpu: vec![],
        };
        let expanded = HashSet::new();
        let child_scroll_offsets = HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();

        terminal
            .draw(|frame| {
                render_snapshot(
                    frame,
                    &snapshot,
                    60.0,
                    60.0,
                    &power_history,
                    SortMode::Energy,
                    0,
                    &expanded,
                    &child_scroll_offsets,
                )
            })
            .unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("DRAM: -- (included in CPU)"));
        assert!(screen.contains("DRAM: -- (in CPU)"));
        assert!(!screen.contains("DRAM: 0.0000 J"));
        assert!(!screen.contains("DRAM: 1.50 W"));
        assert!(!screen.contains("DRAM unavailable"));
    }

    #[test]
    fn render_snapshot_marks_unavailable_dram_without_dram_metric() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: false,
            sources: sources(
                DeviceSource::MeasuredPackage,
                DeviceSource::Unavailable,
                DeviceSource::Unavailable,
            ),
            system_total: DeviceEnergy {
                cpu_joules: 180.0,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
            tracked_pids: vec![123],
            ..MetricsSnapshot::default()
        };
        let power_history = PowerHistorySnapshot {
            cpu: vec![5.0],
            dram: vec![1.5],
            gpu: vec![],
        };
        let expanded = HashSet::new();
        let child_scroll_offsets = HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();

        terminal
            .draw(|frame| {
                render_snapshot(
                    frame,
                    &snapshot,
                    60.0,
                    60.0,
                    &power_history,
                    SortMode::Energy,
                    0,
                    &expanded,
                    &child_scroll_offsets,
                )
            })
            .unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("DRAM: -- (unavailable)"));
        assert!(!screen.contains("DRAM: 0.0000 J"));
        assert!(!screen.contains("DRAM: 1.50 W"));
        assert!(!screen.contains("DRAM included in package energy"));
    }

    #[test]
    fn render_snapshot_shows_gpu_energy_when_gpu_is_available() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: true,
            system_total: DeviceEnergy {
                cpu_joules: 180.0,
                dram_joules: 60.0,
                gpu_joules: 30.0,
            },
            workloads: Vec::new(),
            unattributed: DeviceEnergy {
                cpu_joules: 180.0,
                dram_joules: 60.0,
                gpu_joules: 30.0,
            },
            tracked_pids: Vec::new(),
            ..MetricsSnapshot::default()
        };
        let power_history = PowerHistorySnapshot {
            cpu: vec![5.0],
            dram: vec![1.5],
            gpu: vec![3.0],
        };
        let expanded = HashSet::new();
        let child_scroll_offsets = HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();

        terminal
            .draw(|frame| {
                render_snapshot(
                    frame,
                    &snapshot,
                    60.0,
                    60.0,
                    &power_history,
                    SortMode::Energy,
                    0,
                    &expanded,
                    &child_scroll_offsets,
                )
            })
            .unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("GPU: 30.0000 J"));
        assert!(screen.contains("GPU: 3.00 W"));
    }

    #[test]
    fn render_snapshot_shows_expanded_pid_rows_and_selected_group() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: false,
            system_total: DeviceEnergy {
                cpu_joules: 20.0,
                dram_joules: 4.0,
                gpu_joules: 0.0,
            },
            workloads: vec![WorkloadSnapshot {
                root_pid: 123,
                group_id: "pid:123".to_string(),
                name: "cargo test".to_string(),
                user: "alice".to_string(),
                processes: vec![crate::monitor::ProcessEnergySnapshot {
                    pid: 123,
                    name: "cargo".to_string(),
                    energy: DeviceEnergy {
                        cpu_joules: 12.0,
                        dram_joules: 3.0,
                        gpu_joules: 0.0,
                    },
                    power_watts: 7.5,
                }],
                is_live: true,
                energy: DeviceEnergy {
                    cpu_joules: 20.0,
                    dram_joules: 4.0,
                    gpu_joules: 0.0,
                },
                power_watts: 12.0,
                percentage_of_system: 100.0,
            }],
            unattributed: DeviceEnergy::default(),
            tracked_pids: vec![123],
            ..MetricsSnapshot::default()
        };
        let power_history = PowerHistorySnapshot::default();
        let expanded = HashSet::from(["pid:123".to_string()]);
        let child_scroll_offsets = HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();

        terminal
            .draw(|frame| {
                render_snapshot(
                    frame,
                    &snapshot,
                    2.0,
                    2.0,
                    &power_history,
                    SortMode::Energy,
                    0,
                    &expanded,
                    &child_scroll_offsets,
                )
            })
            .unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("v cargo test"));
        assert!(screen.contains("pid 123 cargo"));
        assert!(screen.contains("15.0000"));
        assert!(screen.contains("7.50"));
    }

    #[test]
    fn workload_table_rows_mark_selected_group_and_hide_collapsed_children() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: false,
            system_total: DeviceEnergy {
                cpu_joules: 8.0,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
            workloads: vec![WorkloadSnapshot {
                root_pid: 123,
                group_id: "pid:123".to_string(),
                name: "python".to_string(),
                user: "alice".to_string(),
                processes: vec![crate::monitor::ProcessEnergySnapshot {
                    pid: 123,
                    name: "python".to_string(),
                    energy: DeviceEnergy {
                        cpu_joules: 8.0,
                        dram_joules: 0.0,
                        gpu_joules: 0.0,
                    },
                    power_watts: 4.0,
                }],
                is_live: true,
                energy: DeviceEnergy {
                    cpu_joules: 8.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
                power_watts: 4.0,
                percentage_of_system: 100.0,
            }],
            unattributed: DeviceEnergy::default(),
            tracked_pids: vec![123],
            ..MetricsSnapshot::default()
        };

        let rows = workload_table_rows(&snapshot, 0, &HashSet::new(), &HashMap::new());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cells[0], "> python");
        assert!(rows[0].style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn workload_table_rows_mark_dead_groups() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: false,
            system_total: DeviceEnergy {
                cpu_joules: 8.0,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
            workloads: vec![
                WorkloadSnapshot {
                    root_pid: 100,
                    group_id: "pid:100".to_string(),
                    name: "live".to_string(),
                    user: "alice".to_string(),
                    processes: Vec::new(),
                    is_live: true,
                    energy: DeviceEnergy {
                        cpu_joules: 3.0,
                        dram_joules: 0.0,
                        gpu_joules: 0.0,
                    },
                    power_watts: 3.0,
                    percentage_of_system: 37.5,
                },
                WorkloadSnapshot {
                    root_pid: 200,
                    group_id: "pid:200".to_string(),
                    name: "finished".to_string(),
                    user: "alice".to_string(),
                    processes: vec![crate::monitor::ProcessEnergySnapshot {
                        pid: 201,
                        name: "stale-child".to_string(),
                        energy: DeviceEnergy {
                            cpu_joules: 2.0,
                            dram_joules: 0.0,
                            gpu_joules: 0.0,
                        },
                        power_watts: 0.0,
                    }],
                    is_live: false,
                    energy: DeviceEnergy {
                        cpu_joules: 5.0,
                        dram_joules: 0.0,
                        gpu_joules: 0.0,
                    },
                    power_watts: 0.0,
                    percentage_of_system: 62.5,
                },
            ],
            unattributed: DeviceEnergy::default(),
            tracked_pids: vec![100],
            ..MetricsSnapshot::default()
        };
        let expanded = HashSet::from(["pid:200".to_string()]);

        let rows = workload_table_rows(&snapshot, 0, &expanded, &HashMap::new());

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].cells[0], "x finished");
        assert_eq!(rows[1].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn workload_table_rows_apply_child_scroll_offset_to_expanded_group() {
        let snapshot = MetricsSnapshot {
            timestamp: 1_000,
            gpu_available: false,
            system_total: DeviceEnergy {
                cpu_joules: 3.0,
                dram_joules: 0.0,
                gpu_joules: 0.0,
            },
            workloads: vec![WorkloadSnapshot {
                root_pid: 1,
                group_id: "pid:1".to_string(),
                name: "python".to_string(),
                user: "alice".to_string(),
                processes: (1..=3)
                    .map(|pid| crate::monitor::ProcessEnergySnapshot {
                        pid,
                        name: format!("child-{pid}"),
                        energy: DeviceEnergy {
                            cpu_joules: 1.0,
                            dram_joules: 0.0,
                            gpu_joules: 0.0,
                        },
                        power_watts: 1.0,
                    })
                    .collect(),
                is_live: true,
                energy: DeviceEnergy {
                    cpu_joules: 3.0,
                    dram_joules: 0.0,
                    gpu_joules: 0.0,
                },
                power_watts: 3.0,
                percentage_of_system: 100.0,
            }],
            unattributed: DeviceEnergy::default(),
            tracked_pids: vec![1, 2, 3],
            ..MetricsSnapshot::default()
        };
        let expanded = HashSet::from(["pid:1".to_string()]);
        let child_scroll_offsets = HashMap::from([("pid:1".to_string(), 1)]);

        let rows = workload_table_rows(&snapshot, 0, &expanded, &child_scroll_offsets);

        assert_eq!(rows[0].cells[0], "v python");
        assert_eq!(rows[1].cells[0], "  pid 2 child-2");
        assert_eq!(rows[2].cells[0], "  pid 3 child-3");
    }

    #[test]
    fn visible_table_rows_keep_selected_group_visible() {
        let rows = (0..8)
            .map(|index| WorkloadTableRow {
                cells: vec![format!("row {index}")],
                style: Style::default(),
                group_index: Some(index),
            })
            .collect::<Vec<_>>();

        let visible = visible_table_rows(rows, 7, 3);

        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].cells[0], "row 5");
        assert_eq!(visible[2].cells[0], "row 7");
    }
}
