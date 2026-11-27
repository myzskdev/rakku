use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Terminal;

use crate::cli::MonitorArgs;
use crate::config::APP_NAME;
use crate::state::StateStore;
use crate::system::metrics::{self, JobState, RunnerLiveStatus};
use crate::system::systemd;

struct MonitorState {
    runners: Vec<RunnerLiveStatus>,
    table_state: TableState,
    logs: Vec<String>,
    show_logs: bool,
    status_message: Option<String>,
    last_refresh: String,
    interval: Duration,
}

impl MonitorState {
    fn new(interval_ms: u64) -> Self {
        let mut st = Self {
            runners: Vec::new(),
            table_state: TableState::default(),
            logs: Vec::new(),
            show_logs: true,
            status_message: None,
            last_refresh: "-".into(),
            interval: Duration::from_millis(interval_ms.max(500)),
        };
        st.refresh();
        st
    }

    fn refresh(&mut self) {
        match StateStore::load_default() {
            Ok(store) => {
                self.runners = store
                    .runners
                    .iter()
                    .map(metrics::collect_runner_status)
                    .collect();

                if self.runners.is_empty() {
                    self.table_state.select(None);
                    self.logs.clear();
                } else {
                    let selected = self
                        .table_state
                        .selected()
                        .unwrap_or(0)
                        .min(self.runners.len() - 1);
                    self.table_state.select(Some(selected));
                    self.refresh_selected_logs();
                }
                self.last_refresh = Local::now().format("%H:%M:%S").to_string();
            }
            Err(err) => {
                self.status_message = Some(format!("Failed to load state: {err}"));
            }
        }
    }

    fn refresh_selected_logs(&mut self) {
        if !self.show_logs {
            return;
        }
        if let Some(runner) = self.selected_runner() {
            self.logs = systemd::read_recent_logs(&runner.service_name, 14);
        } else {
            self.logs.clear();
        }
    }

    fn selected_runner(&self) -> Option<&RunnerLiveStatus> {
        let idx = self.table_state.selected()?;
        self.runners.get(idx)
    }

    fn select_next(&mut self) {
        if self.runners.is_empty() {
            return;
        }
        let next = match self.table_state.selected() {
            Some(i) => (i + 1) % self.runners.len(),
            None => 0,
        };
        self.table_state.select(Some(next));
        self.refresh_selected_logs();
    }

    fn select_prev(&mut self) {
        if self.runners.is_empty() {
            return;
        }
        let prev = match self.table_state.selected() {
            Some(0) | None => self.runners.len() - 1,
            Some(i) => i - 1,
        };
        self.table_state.select(Some(prev));
        self.refresh_selected_logs();
    }

    fn control_selected(&mut self, action: &str) {
        let Some(runner) = self.selected_runner().cloned() else {
            return;
        };
        match systemd::control_service(&runner.service_name, action) {
            Ok(()) => {
                self.status_message = Some(format!("{action} succeeded for `{}`", runner.name));
                self.refresh();
            }
            Err(err) => {
                self.status_message = Some(format!(
                    "{action} failed for `{}` (need sudo?): {}",
                    runner.name,
                    first_line(&err.to_string())
                ));
            }
        }
    }
}

pub fn run(args: MonitorArgs) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_event_loop(&mut terminal, args.interval_ms);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    res
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    interval_ms: u64,
) -> Result<()> {
    let mut app = MonitorState::new(interval_ms);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|frame| draw_ui(frame, &mut app))?;

        let timeout = app
            .interval
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_millis(50));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _)
                    | (KeyCode::Esc, _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => app.select_next(),
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => app.select_prev(),
                    (KeyCode::Char('l'), _) => {
                        app.show_logs = !app.show_logs;
                        app.refresh_selected_logs();
                    }
                    (KeyCode::Char(' '), _) => {
                        app.refresh();
                        last_tick = Instant::now();
                    }
                    (KeyCode::Char('r'), _) => app.control_selected("restart"),
                    (KeyCode::Char('s'), _) => {
                        if let Some(sel) = app.selected_runner() {
                            let action = if sel.active_state == "active" {
                                "stop"
                            } else {
                                "start"
                            };
                            app.control_selected(action);
                        }
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= app.interval {
            app.refresh();
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn draw_ui(frame: &mut ratatui::Frame<'_>, app: &mut MonitorState) {
    let total = app.runners.len();
    let active = app
        .runners
        .iter()
        .filter(|r| r.active_state == "active")
        .count();
    let busy = app
        .runners
        .iter()
        .filter(|r| matches!(r.job_state, JobState::Running(_)))
        .count();
    let offline = total.saturating_sub(active);

    let constraints = if app.show_logs {
        vec![
            Constraint::Length(3),
            Constraint::Min(7),
            Constraint::Length(12),
            Constraint::Length(2),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(frame.area());

    // Header summary bar
    let header_line = Line::from(vec![
        Span::styled(
            format!(" {APP_NAME} monitor "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::raw(format!("Total: {total}  ")),
        Span::styled(
            format!("Active: {active}  "),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!("Busy: {busy}  "),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(
            format!("Offline: {offline}  "),
            Style::default().fg(if offline > 0 {
                Color::Red
            } else {
                Color::DarkGray
            }),
        ),
        Span::raw(format!("| Updated: {}", app.last_refresh)),
    ]);
    let header = Paragraph::new(header_line).block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, chunks[0]);

    // Runners table
    if app.runners.is_empty() {
        let empty = Paragraph::new(format!(
            "No runners installed by {APP_NAME} on this machine.\nRun `sudo {APP_NAME} install` to register your first runner."
        ))
        .block(
            Block::default()
                .title(" Local Runners ")
                .borders(Borders::ALL),
        );
        frame.render_widget(empty, chunks[1]);
    } else {
        let rows: Vec<Row<'_>> = app
            .runners
            .iter()
            .map(|r| {
                let svc_color = match r.active_state.as_str() {
                    "active" => Color::Green,
                    "failed" => Color::Red,
                    "activating" | "reloading" => Color::Yellow,
                    _ => Color::DarkGray,
                };
                let job_color = match &r.job_state {
                    JobState::Running(_) => Color::Magenta,
                    JobState::Idle => Color::Green,
                    JobState::Starting => Color::Yellow,
                    JobState::Error(_) | JobState::Offline => Color::Red,
                    JobState::Unknown => Color::DarkGray,
                };
                let cpu = r
                    .cpu_percent
                    .map(|c| format!("{c:.1}%"))
                    .unwrap_or_else(|| "-".into());
                let mem = metrics::format_bytes(r.memory_bytes);
                let disk = metrics::format_bytes(r.disk_bytes);
                let up = metrics::format_uptime(r.uptime_secs);

                Row::new(vec![
                    Cell::from(r.name.clone()),
                    Cell::from(r.provider.to_string()).style(Style::default().fg(Color::Cyan)),
                    Cell::from(r.active_state.clone()).style(Style::default().fg(svc_color)),
                    Cell::from(r.job_state.to_string()).style(Style::default().fg(job_color)),
                    Cell::from(r.linux_user.clone()),
                    Cell::from(cpu),
                    Cell::from(mem),
                    Cell::from(disk),
                    Cell::from(up),
                    Cell::from(r.restarts.to_string()),
                    Cell::from(r.url.clone()),
                ])
            })
            .collect();

        let header_row = Row::new([
            "NAME", "PROVIDER", "SERVICE", "JOB", "USER", "CPU", "MEM", "DISK", "UPTIME", "RST",
            "TARGET",
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

        let widths = [
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(20),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(4),
            Constraint::Min(18),
        ];

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .title(" Local Runners ")
                    .borders(Borders::ALL),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, chunks[1], &mut app.table_state);
    }

    // Optional logs pane
    let footer_idx = if app.show_logs {
        let log_title = match app.selected_runner() {
            Some(r) => format!(" Recent Logs: {} ({}) ", r.name, r.service_name),
            None => " Recent Logs ".to_string(),
        };
        let log_text = if app.logs.is_empty() {
            "No journalctl logs available for this runner.".to_string()
        } else {
            app.logs.join("\n")
        };
        let log_widget = Paragraph::new(log_text)
            .wrap(Wrap { trim: false })
            .block(Block::default().title(log_title).borders(Borders::ALL));
        frame.render_widget(log_widget, chunks[2]);
        3
    } else {
        2
    };

    // Footer keybindings + status toast
    let keys = " [↑/↓ or j/k] Select  [r] Restart  [s] Start/Stop  [l] Toggle Logs  [Space] Refresh  [q] Quit";
    let footer_text = match &app.status_message {
        Some(msg) => format!("{keys}\n Status: {msg}"),
        None => keys.to_string(),
    };
    let footer = Paragraph::new(footer_text).style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, chunks[footer_idx]);
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}
