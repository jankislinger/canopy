use crate::{
    app::{AppState, Confirmation},
    backend::{ProjectKind, sessions::Session},
};
use ansi_to_tui::IntoText;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};
/// Shows a centered wordmark while the initial project list loads.
pub(crate) fn draw_loading(f: &mut ratatui::Frame) {
    let logo = [
        " ██████╗ █████╗ ███╗   ██╗ ██████╗ ██████╗ ██╗   ██╗",
        "██╔════╝██╔══██╗████╗  ██║██╔═══██╗██╔══██╗╚██╗ ██╔╝",
        "██║     ███████║██╔██╗ ██║██║   ██║██████╔╝ ╚████╔╝ ",
        "██║     ██╔══██║██║╚██╗██║██║   ██║██╔═══╝   ╚██╔╝  ",
        "╚██████╗██║  ██║██║ ╚████║╚██████╔╝██║        ██║   ",
        " ╚═════╝╚═╝  ╚═╝╚═╝  ╚═══╝ ╚═════╝ ╚═╝        ╚═╝   ",
    ];
    let area = f.area();
    let mut lines: Vec<Line> = if area.width >= 54 {
        logo.into_iter().map(Line::from).collect()
    } else {
        vec![Line::from("canopy")]
    };
    lines.push(Line::from(""));
    lines.push(Line::from("Loading projects…"));
    lines.push(Line::from("q: quit"));
    let height = lines.len() as u16;
    let rect = ratatui::layout::Rect::new(
        area.x,
        area.y + area.height.saturating_sub(height) / 2,
        area.width,
        height.min(area.height),
    );
    f.render_widget(
        Paragraph::new(lines)
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::Cyan)),
        rect,
    );
}

/// Renders the project tree, details pane, and command hints.
pub fn draw(f: &mut ratatui::Frame, s: &mut AppState) {
    if s.initializing {
        draw_loading(f);
        return;
    }
    let a = Layout::vertical([Constraint::Min(8), Constraint::Length(5)]).split(f.area());
    let p =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(a[0]);
    let rs = s.visible_rows();
    let list_width = p[0].width.saturating_sub(2) as usize;
    let items = rs
        .iter()
        .map(|r| {
            let child = s
                .repositories
                .iter()
                .flat_map(|repo| &repo.projects)
                .any(|project| project.path != r.path && project.path.starts_with(&r.path));
            let m =
                if child { if s.expanded.contains(&r.path) { "▾" } else { "▸" } } else { " " };
            let n = r.path.file_name().unwrap_or(r.path.as_os_str()).to_string_lossy();
            let session_status = r
                .kinds
                .as_ref()
                .map(|_| {
                    let name = Session::name_for_path(&r.path);
                    match s.sessions.iter().find(|session| session.name == name) {
                        Some(session) if session.attached_clients > 0 => "●  attached",
                        Some(_) => "●   running",
                        None => "○   stopped",
                    }
                })
                .unwrap_or("");
            let left = format!(
                "{}{:indent$}{m} {n}",
                if r.path == s.selected { "›" } else { " " },
                "",
                indent = r.depth * 2
            );
            let labels = r
                .kinds
                .as_ref()
                .map(|kinds| kinds.iter().map(ProjectKind::label).collect::<Vec<_>>().join(","));
            let right = match (labels, session_status.is_empty()) {
                (Some(labels), false) => format!("[{labels}]  {session_status}"),
                (Some(labels), true) => format!("[{labels}]"),
                (None, false) => session_status.to_owned(),
                (None, true) => String::new(),
            };
            let gap = if right.is_empty() {
                String::new()
            } else {
                " ".repeat(
                    list_width
                        .saturating_sub(
                            Line::from(left.as_str()).width() + Line::from(right.as_str()).width(),
                        )
                        .max(1),
                )
            };
            let style = if r.hidden {
                let style = Style::default().fg(Color::DarkGray);
                if r.path == s.selected { style.add_modifier(Modifier::BOLD) } else { style }
            } else if r.path == s.selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(format!("{left}{gap}{right}")).style(style))
        })
        .collect::<Vec<_>>();
    s.project_list_state.select(rs.iter().position(|row| row.path == s.selected));
    f.render_stateful_widget(
        List::new(items).block(Block::bordered().title("Projects")).scroll_padding(1),
        p[0],
        &mut s.project_list_state,
    );
    f.render_widget(details(s), p[1]);
    f.render_widget(
        Paragraph::new(vec![
            Line::from("Enter: switch  s: start  x: stop"),
            Line::from("t: test  l: lint  r: refresh  H: hidden"),
            Line::from("Esc: close popup  q: quit"),
            Line::from(if s.confirmation.is_some() { "Confirm stop: y/n" } else { &s.status }),
        ])
        .block(Block::bordered().title("Commands")),
        a[1],
    );
    if let Some(Confirmation::StopSession { path, .. }) = &s.confirmation {
        let popup = centered_rect(60, 7, f.area());
        f.render_widget(Clear, popup);
        f.render_widget(
            Paragraph::new(vec![
                Line::from("Stop the selected tmux session?"),
                Line::from(path.display().to_string()),
                Line::from("[y] confirm    [n/Esc] cancel"),
            ])
            .block(Block::bordered().title("Confirm stop")),
            popup,
        );
    }
    if let Some(popup) = &s.command_popup {
        let area = centered_rect(85, 20, f.area());
        f.render_widget(Clear, area);
        let border_color = match popup.success {
            Some(true) => Color::Green,
            Some(false) => Color::Red,
            None => Color::Yellow,
        };
        f.render_widget(
            Paragraph::new(
                popup
                    .output
                    .as_bytes()
                    .into_text()
                    .unwrap_or_else(|_| popup.output.as_str().into()),
            )
            .block(
                Block::bordered()
                    .title(popup.title.as_str())
                    .border_style(Style::default().fg(border_color)),
            )
            .scroll((popup.scroll, 0))
            .wrap(ratatui::widgets::Wrap { trim: false }),
            area,
        );
    }
    if s.tmux_warning {
        let area = centered_rect(70, 9, f.area());
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new("Canopy was started outside tmux.\n\nSwitching to project sessions requires tmux.\nStart Canopy inside tmux, for example: tmux new-session canopy\n\nEnter/Esc: dismiss    q: quit")
                .wrap(ratatui::widgets::Wrap { trim: false })
                .block(Block::bordered().title("Warning: outside tmux")
                    .border_style(Style::default().fg(Color::Yellow))),
            area,
        );
    }
    if let Some(error) = &s.error_popup {
        let area = centered_rect(70, 9, f.area());
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(error.as_str())
                .wrap(ratatui::widgets::Wrap { trim: false })
                .block(Block::bordered().title("Settings error")),
            area,
        );
    }
}

fn details(s: &AppState) -> Paragraph<'_> {
    let mut lines = vec![Line::from(s.selected.display().to_string())];
    if s.git_status_loading {
        lines.push(Line::from("Loading Git history..."));
    } else if let Some(status) = &s.git_status {
        let (indicator, description, color) = match status.working_tree {
            crate::backend::git::WorkingTree::Clean => ("✓", "clean", Color::Green),
            crate::backend::git::WorkingTree::ChangesOutsideProject => {
                ("●", "changes outside", Color::Yellow)
            }
            crate::backend::git::WorkingTree::ChangesInProject => {
                ("●", "changes in project", Color::Red)
            }
        };
        lines.push(Line::from(vec![
            Span::raw("branch: "),
            Span::styled(&status.branch, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("{indicator} {description}"), Style::default().fg(color)),
        ]));
        lines.push(Line::from(""));
        for commit in &status.commits {
            let foreground =
                if commit.authored_by_user { Color::Cyan } else { Color::Rgb(110, 135, 180) };
            let text_style = if commit.authored_by_user {
                Style::default()
            } else {
                Style::default().fg(Color::Gray)
            };
            lines.push(Line::from(vec![
                Span::styled(&commit.sha, Style::default().fg(foreground)),
                Span::styled(
                    format!("  {:<3}  {:<14} {}", commit.author, commit.age, commit.message),
                    text_style,
                ),
            ]));
        }
    } else {
        lines.push(Line::from("Not in a Git repository"));
    }
    Paragraph::new(lines).block(Block::bordered().title("Details"))
}

fn centered_rect(
    width_percent: u16,
    height: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let vertical =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(height), Constraint::Fill(1)])
            .split(area);
    Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(width_percent),
        Constraint::Fill(1),
    ])
    .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::{details, draw};
    use crate::{
        app::AppState,
        backend::{
            Project, ProjectKind, Repository,
            git::{GitCommit, GitStatus, WorkingTree},
            sessions::SessionBackend,
        },
        config::Settings,
    };
    use ratatui::{Terminal, backend::TestBackend};
    use std::{collections::HashSet, path::PathBuf};

    fn test_state() -> AppState {
        AppState {
            repositories: Vec::new(),
            home: PathBuf::from("/home/test"),
            selected: PathBuf::from("/home/test/repo"),
            project_list_state: Default::default(),
            expanded: HashSet::new(),
            show_hidden: false,
            settings: Settings::default(),
            error_popup: None,
            tmux_warning: false,
            loading: false,
            completed: 0,
            spinner: 0,
            status: String::new(),
            sessions: Vec::new(),
            session_backend: SessionBackend::default(),
            confirmation: None,
            project_scan_generation: 0,
            initializing: false,
            project_scan_pending: false,
            command_popup: None,
            command_generation: 0,
            command_cancellation: None,
            git_status: Some(GitStatus {
                repository: PathBuf::from("/home/test/repo"),
                branch: "main".into(),
                working_tree: WorkingTree::Clean,
                commits: vec![
                    GitCommit {
                        sha: "123456".into(),
                        author: "AL".into(),
                        authored_by_user: true,
                        message: "latest".into(),
                        age: "1 hour ago".into(),
                    },
                    GitCommit {
                        sha: "abcdef".into(),
                        author: "GH".into(),
                        authored_by_user: false,
                        message: "older".into(),
                        age: "2 hours ago".into(),
                    },
                ],
            }),
            git_status_cache: Default::default(),
            git_status_loading: false,
            git_status_generation: 0,
            sessions_refresh_generation: 0,
            sessions_refresh_pending: false,
        }
    }

    #[test]
    fn details_renders_six_characters_for_every_commit_sha() {
        let state = test_state();
        let backend = TestBackend::new(100, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| frame.render_widget(details(&state), frame.area())).unwrap();

        let buffer = terminal.backend().buffer();
        for (y, expected) in [(4, "123456"), (5, "abcdef")] {
            let rendered: String = (1..7).map(|x| buffer[(x, y)].symbol()).collect();
            assert_eq!(rendered, expected);
        }
    }

    #[test]
    fn startup_screen_handles_large_and_small_terminals() {
        let mut state = test_state();
        state.initializing = true;
        state.tmux_warning = true;
        for (width, height) in [(100, 24), (30, 10), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(!text.contains("Projects"));
            assert!(!text.contains("outside tmux"));
            if width > 1 {
                assert!(text.contains("Loading projects…"));
                if width < 54 {
                    assert!(text.contains("canopy"));
                } else {
                    assert!(text.contains("██████"));
                }
            }
        }
    }

    #[test]
    fn tmux_warning_is_visible_after_loading_and_can_be_dismissed() {
        let mut state = test_state();
        state.tmux_warning = true;
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text: String =
            terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Warning: outside tmux"));
        assert!(text.contains("tmux new-session canopy"));
        state.tmux_warning = false;
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text: String =
            terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(!text.contains("Warning: outside tmux"));
    }

    #[test]
    fn directory_arrows_show_collapsed_and_expanded_states() {
        let mut state = test_state();
        let repository = state.selected.clone();
        state.repositories = vec![Repository {
            path: repository.clone(),
            projects: vec![
                Project::new(repository.clone(), vec![ProjectKind::Git], None),
                Project::new(repository.join("library"), vec![ProjectKind::Rust], None),
            ],
        }];
        state.expanded.insert(state.home.clone());
        let mut terminal = Terminal::new(TestBackend::new(100, 15)).unwrap();

        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(terminal.backend().buffer()[(2, 1)].symbol(), "▸");

        state.expanded.insert(repository);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 1)].symbol(), "▾");
        assert_eq!(buffer[(4, 2)].symbol(), " ");
    }

    #[test]
    fn project_list_scrolls_before_selection_reaches_the_bottom() {
        let mut state = test_state();
        let projects: Vec<_> = (0..20)
            .map(|index| {
                Project::new(
                    state.home.join(format!("project-{index:02}")),
                    vec![ProjectKind::Git],
                    None,
                )
            })
            .collect();
        state.repositories = vec![Repository { path: state.home.clone(), projects }];
        state.expanded.insert(state.home.clone());
        let mut terminal = Terminal::new(TestBackend::new(100, 15)).unwrap();

        for (index, expected_offset) in [(0, 0), (6, 0), (7, 1), (15, 9), (0, 0)] {
            state.selected = state.home.join(format!("project-{index:02}"));
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            assert_eq!(state.project_list_state.offset(), expected_offset);
            let selected_y = 1 + (index - expected_offset) as u16;
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(1, selected_y)].symbol(), "›");
            let below: String = (1..49).map(|x| buffer[(x, selected_y + 1)].symbol()).collect();
            assert!(below.contains(&format!("project-{:02}", index + 1)));
        }

        // A smaller viewport must still show the next project below the selected one.
        state.selected = state.home.join("project-15");
        terminal.backend_mut().resize(100, 13);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(state.project_list_state.offset(), 11);
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(1, 5)].symbol(), "›");
        let below: String = (1..49).map(|x| buffer[(x, 6)].symbol()).collect();
        assert!(below.contains("project-16"));
    }
}
