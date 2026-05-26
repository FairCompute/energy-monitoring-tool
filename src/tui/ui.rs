use crate::monitor::MetricsSnapshot;
use crate::tui::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};

pub fn render(frame: &mut Frame, app: &App) {
    let snapshot = app.snapshot();
    let uptime = app.uptime_secs();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], &snapshot, uptime);
    render_body(frame, chunks[1], &snapshot);
    render_footer(frame, chunks[2]);
}

fn render_header(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    snapshot: &MetricsSnapshot,
    uptime: f64,
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
            Span::styled("  Power: ", Style::default().fg(Color::Cyan)),
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
            Span::raw(format!(
                "    Tracked PIDs: {}",
                snapshot.tracked_pids.len()
            )),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" EMT - Energy Monitoring Tool ");
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_body(frame: &mut Frame, area: ratatui::layout::Rect, snapshot: &MetricsSnapshot) {
    if snapshot.workloads.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Workloads ");
        let paragraph =
            Paragraph::new("  No process data yet...").block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec!["PID", "Name", "User", "Energy (J)", "Power (W)"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = snapshot
        .workloads
        .iter()
        .map(|wl| {
            Row::new(vec![
                wl.root_pid.to_string(),
                wl.name.clone(),
                wl.user.clone(),
                format!("{:.4}", wl.energy.total()),
                format!("{:.2}", wl.power_watts),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Min(20),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Workloads "),
    );

    frame.render_widget(table, area);
}

fn render_footer(frame: &mut Frame, area: ratatui::layout::Rect) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Red)),
        Span::raw(" quit"),
    ]));
    frame.render_widget(footer, area);
}
