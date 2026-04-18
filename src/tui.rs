use crate::config::{default_config_path, default_pid_path, GatewayConfig};
use crate::process::{pid_alive, read_pid, send_sighup};
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub async fn run(config_path: PathBuf) -> Result<()> {
    let config_path = if config_path.as_os_str().is_empty() {
        default_config_path()
    } else {
        config_path
    };

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &config_path).await;

    terminal.show_cursor()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config_path: &PathBuf,
) -> Result<()> {
    let pid_path = default_pid_path();
    let mut message = String::from("press r to reload, q to quit");

    loop {
        let config = GatewayConfig::load(config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))?;

        let pid_status = match read_pid(&pid_path) {
            Ok(pid) if pid_alive(pid) => format!("running (pid {})", pid),
            Ok(pid) => format!("stale pid file (pid {})", pid),
            Err(_) => "not running".to_string(),
        };

        terminal.draw(|frame| draw(frame, &config, config_path, &pid_status, &message))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('r') => match read_pid(&pid_path) {
                        Ok(pid) if pid_alive(pid) => match send_sighup(pid) {
                            Ok(_) => message = format!("sent reload signal to pid {}", pid),
                            Err(err) => message = format!("reload failed: {}", err),
                        },
                        Ok(_) => message = String::from("pid file is stale"),
                        Err(err) => message = format!("reload failed: {}", err),
                    },
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn draw(
    frame: &mut Frame,
    config: &GatewayConfig,
    config_path: &Path,
    pid_status: &str,
    message: &str,
) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Min(9),
            Constraint::Length(3),
        ])
        .split(area);

    let banner = Paragraph::new("Itadori\nsoft gateway mode")
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("kawaii proxy")
                .title_alignment(Alignment::Center)
                .style(Style::default().fg(Color::LightCyan)),
        );
    frame.render_widget(banner, chunks[0]);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    let status_lines = vec![
        Line::from(vec![
            Span::styled("config ", Style::default().fg(Color::LightMagenta)),
            Span::raw(config_path.display().to_string()),
        ]),
        Line::from(vec![
            Span::styled("bind   ", Style::default().fg(Color::LightCyan)),
            Span::raw(config.server.bind.to_string()),
        ]),
        Line::from(vec![
            Span::styled("pid    ", Style::default().fg(Color::LightGreen)),
            Span::raw(pid_status.to_string()),
        ]),
    ];

    let status_card = Paragraph::new(status_lines)
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .title("status")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .style(Style::default().fg(Color::LightMagenta)),
        );
    frame.render_widget(status_card, top[0]);

    let controls = Paragraph::new(vec![
        Line::from("r  reload config"),
        Line::from("q  quit"),
        Line::from(""),
        Line::from("cute mode, but still serious"),
    ])
    .style(Style::default().fg(Color::LightYellow))
    .block(
        Block::default()
            .title("controls")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(Style::default().fg(Color::LightYellow)),
    );
    frame.render_widget(controls, top[1]);

    let route_items: Vec<ListItem> = if config.routes.is_empty() {
        vec![ListItem::new("no routes yet. run itadori init").style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
        )]
    } else {
        config
            .routes
            .iter()
            .map(|route| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            route.name.clone(),
                            Style::default()
                                .fg(Color::LightMagenta)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::styled(route.prefix.clone(), Style::default().fg(Color::LightCyan)),
                    ]),
                    Line::from(Span::styled(
                        route.upstream.to_string(),
                        Style::default().fg(Color::Gray),
                    )),
                ])
                .style(Style::default().bg(Color::Rgb(24, 26, 40)))
            })
            .collect()
    };

    let routes = List::new(route_items)
        .block(
            Block::default()
                .title("routes")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .style(Style::default().fg(Color::LightCyan)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(routes, chunks[2]);

    let footer = Paragraph::new(message)
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("status line")
                .style(Style::default().fg(Color::LightGreen)),
        );
    frame.render_widget(footer, chunks[3]);
}
