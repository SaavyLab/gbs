use std::io;
use std::process::Command;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Terminal,
};

struct Branch {
    name: String,
    short_sha: String,
    is_current: bool,
}

#[derive(PartialEq)]
enum AppMode {
    Normal,
    ConfirmDelete,
}

struct App {
    branches: Vec<Branch>,
    selected: usize,
    mode: AppMode,
}

fn load_branches() -> Result<Vec<Branch>> {
    let output = Command::new("git")
        .args(&[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)|%(objectname:short)|%(HEAD)",
            "refs/heads",
        ])
        .output()?;

    if !output.status.success() {
        anyhow::bail!("git for-each-ref failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8(output.stdout)?;
    let branches = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('|');
            let name = parts.next().unwrap_or("").to_string();
            let sha = parts.next().unwrap_or("").to_string();
            let head_flag = parts.next().unwrap_or("");
            Branch {
                name,
                short_sha: sha,
                is_current: head_flag == "*",
            }
        })
        .collect();
    Ok(branches)
}

fn main() -> Result<()> {
    let branches = load_branches()?;
    if branches.is_empty() {
        eprintln!("no local branches found");
        return Ok(());
    }

    let mut app = App {
        branches,
        selected: 0,
        mode: AppMode::Normal,
    };

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut app);

    // teardown
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // if app ended with a selection, do the switch
    if let Ok(Some(idx)) = res {
        let branch = &app.branches[idx];
        let status = Command::new("git")
            .args(&["switch", &branch.name])
            .status()?;

        // propagate failure if git switch failed
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    } else if let Err(err) = res {
        eprintln!("error: {err}");
        std::process::exit(1);
    }

    Ok(())
}

fn delete_branch(name: &str) -> Result<()> {
    let output = Command::new("git")
        .args(&["branch", "-d", name])
        .output()?;

    if !output.status.success() {
        // Try force delete if normal delete fails (unmerged branch)
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not fully merged") {
            let force_output = Command::new("git")
                .args(&["branch", "-D", name])
                .output()?;
            if !force_output.status.success() {
                anyhow::bail!(
                    "git branch -D failed: {}",
                    String::from_utf8_lossy(&force_output.stderr)
                );
            }
        } else {
            anyhow::bail!("git branch -d failed: {}", stderr);
        }
    }
    Ok(())
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(height)) / 2;
    Rect::new(x, y, popup_width, height)
}

// returns Ok(Some(idx)) if user hit enter, Ok(None) if they quit
fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<Option<usize>> {
    loop {
        let selected_branch_name = app.branches.get(app.selected).map(|b| b.name.clone());
        let selected_is_current = app
            .branches
            .get(app.selected)
            .map(|b| b.is_current)
            .unwrap_or(false);

        terminal.draw(|f| {
            let size = f.area();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Min(1), Constraint::Length(5)].as_ref())
                .split(size);

            let items: Vec<ListItem> = app
                .branches
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let marker = if b.is_current { "*" } else { " " };
                    let prefix = if i == app.selected { ">" } else { " " };
                    let content = Line::from(vec![
                        Span::raw(format!("{prefix}{marker} ")),
                        Span::styled(&b.name, Style::default().add_modifier(
                            if i == app.selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            },
                        )),
                        Span::raw(format!("  {}", b.short_sha)),
                    ]);
                    ListItem::new(content)
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("branches"))
                .highlight_symbol("> ");

            f.render_widget(list, chunks[0]);

            let help_lines = if app.mode == AppMode::ConfirmDelete {
                vec![
                    Line::from(vec![
                        Span::styled("y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::raw(" confirm  "),
                        Span::styled("n/Esc", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::raw(" cancel"),
                    ]),
                ]
            } else {
                vec![
                    Line::from(vec![
                        Span::styled("j/↓", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::raw(" down  "),
                        Span::styled("k/↑", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::raw(" up  "),
                        Span::styled("Enter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::raw(" switch"),
                    ]),
                    Line::from(vec![
                        Span::styled("D", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::raw(" delete  "),
                        Span::styled("q/Esc", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        Span::raw(" quit"),
                    ]),
                ]
            };
            let help_block = Block::default().title("keybinds").borders(Borders::ALL);
            let paragraph =
                Paragraph::new(help_lines).block(help_block).alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);

            // Render confirmation dialog if in ConfirmDelete mode
            if app.mode == AppMode::ConfirmDelete {
                if let Some(ref name) = selected_branch_name {
                    let popup_area = centered_rect(60, 5, size);
                    f.render_widget(Clear, popup_area);

                    let text = vec![
                        Line::from(""),
                        Line::from(vec![
                            Span::raw("Delete branch "),
                            Span::styled(name, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                            Span::raw("?"),
                        ]),
                        Line::from(Span::styled("(y/n)", Style::default().fg(Color::Gray))),
                    ];

                    let popup = Paragraph::new(text)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Confirm Delete")
                                .border_style(Style::default().fg(Color::Red)),
                        )
                        .alignment(Alignment::Center);

                    f.render_widget(popup, popup_area);
                }
            }
        })?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                // ignore key repeats on some platforms
                if key.kind == KeyEventKind::Release {
                    continue;
                }

                if app.mode == AppMode::ConfirmDelete {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let Some(ref name) = selected_branch_name {
                                if let Err(e) = delete_branch(name) {
                                    // For now, just cancel and the error will show after exit
                                    app.mode = AppMode::Normal;
                                    anyhow::bail!("Failed to delete branch: {}", e);
                                }
                                // Reload branches after deletion
                                app.branches = load_branches()?;
                                // Adjust selection if needed
                                if app.selected >= app.branches.len() && app.selected > 0 {
                                    app.selected = app.branches.len() - 1;
                                }
                                // Return to normal mode if no branches left
                                if app.branches.is_empty() {
                                    return Ok(None);
                                }
                            }
                            app.mode = AppMode::Normal;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            app.mode = AppMode::Normal;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.selected + 1 < app.branches.len() {
                                app.selected += 1;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if app.selected > 0 {
                                app.selected -= 1;
                            }
                        }
                        KeyCode::Enter => return Ok(Some(app.selected)),
                        KeyCode::Char('D') => {
                            // Don't allow deleting the current branch
                            if !selected_is_current {
                                app.mode = AppMode::ConfirmDelete;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
