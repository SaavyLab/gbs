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
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Terminal,
};

struct Branch {
    name: String,
    short_sha: String,
    is_current: bool,
}

struct App {
    branches: Vec<Branch>,
    selected: usize,
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

// returns Ok(Some(idx)) if user hit enter, Ok(None) if they quit
fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<Option<usize>> {
    loop {
        terminal.draw(|f| {
            let size = f.area();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
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

            let help = Line::from(vec![Span::raw(
                "j/k or ↑/↓ to move, enter to switch, q to quit",
            )]);
            let help_block = Block::default().title("help").borders(Borders::TOP);
            let paragraph =
                ratatui::widgets::Paragraph::new(help).block(help_block).alignment(Alignment::Left);
            f.render_widget(paragraph, chunks[1]);
        })?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                // ignore key repeats on some platforms
                if key.kind == KeyEventKind::Release {
                    continue;
                }

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
                    _ => {}
                }
            }
        }
    }
}
