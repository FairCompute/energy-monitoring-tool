use crate::monitor::MetricsSnapshot;
use crate::tui::App;
use crate::tui::app::PowerHistorySnapshot;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Sparkline, Table};

pub fn render(frame: &mut Frame, app: &App) {
    let snapshot = app.snapshot();
    let uptime = app.uptime_secs();
    let power_history = app.power_history();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], &snapshot, uptime, &power_history);
    render_body(frame, chunks[1], &snapshot);
    render_footer(frame, chunks[2]);
}

fn render_header(
    frame: &mut Frame,
    area: Rect,
    snapshot: &MetricsSnapshot,
    uptime: f64,
    power_history: &PowerHistorySnapshot,
) {
    let total_energy = snapshot.system_total.total();
    let power = if uptime > 0.0 {
        total_energy / uptime
    } else {
        0.0
    };

    let mins = (uptime as u64) / 60;
    let secs = (uptime as u64) % 60;

    let lines = vec![
        Line::from(vec![
            Span::styled("  Avg Power: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{power:.2} W")),
            Span::raw("    "),
            Span::styled("Energy: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{total_energy:.4} J")),
        ]),
        Line::from(vec![
            Span::styled("    CPU: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{:.4} J", snapshot.system_total.cpu_joules)),
            Span::raw("    "),
            Span::styled("DRAM: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{:.4} J", snapshot.system_total.dram_joules)),
            Span::raw("    "),
            Span::styled("GPU: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{:.4} J", snapshot.system_total.gpu_joules)),
        ]),
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
        render_power_history(frame, header_chunks[1], header_chunks[2], power_history);
    }
}

fn render_power_history(
    frame: &mut Frame,
    label_area: Rect,
    sparkline_area: Rect,
    power_history: &PowerHistorySnapshot,
) {
    let label_chunks = split_power_columns(label_area);
    let sparkline_chunks = split_power_columns(sparkline_area);

    render_power_label(
        frame,
        label_chunks[0],
        "CPU",
        power_history.latest_cpu(),
        Color::Yellow,
    );
    render_power_label(
        frame,
        label_chunks[1],
        "DRAM",
        power_history.latest_dram(),
        Color::Magenta,
    );
    render_power_label(
        frame,
        label_chunks[2],
        "GPU",
        power_history.latest_gpu(),
        Color::Green,
    );

    render_component_sparkline(
        frame,
        sparkline_chunks[0],
        &power_history.cpu,
        Color::Yellow,
    );
    render_component_sparkline(
        frame,
        sparkline_chunks[1],
        &power_history.dram,
        Color::Magenta,
    );
    render_component_sparkline(frame, sparkline_chunks[2], &power_history.gpu, Color::Green);
}

fn split_power_columns(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
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

fn render_body(frame: &mut Frame, area: Rect, snapshot: &MetricsSnapshot) {
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

    let rows: Vec<Row> = snapshot
        .workloads
        .iter()
        .map(|wl| {
            Row::new(vec![
                wl.name.clone(),
                wl.user.clone(),
                format!("{:.4}", wl.energy.total()),
                format!("{:.2}", wl.power_watts),
                format!("{:.1}%", wl.percentage_of_system),
            ])
        })
        .collect();

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

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Red)),
        Span::raw(" quit"),
    ]));
    frame.render_widget(footer, area);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
