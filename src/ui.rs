use crate::{
    app::{AppState, Confirmation},
    backend::{ProjectKind, sessions::Session, update::BuildInfo},
};
use ansi_to_tui::IntoText;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};
/// Shows a centered wordmark while the initial project list loads.
pub(crate) fn draw_loading(f: &mut ratatui::Frame, build: &BuildInfo) {
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
    lines.push(Line::from(build.label()).style(Style::default().fg(Color::DarkGray)));
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

fn commands_block(s: &AppState) -> Block<'static> {
    let block = Block::bordered().title("Commands");
    if s.upgraded {
        let hint = " Upgraded: restart Canopy to apply ";
        return block
            .title(Line::from(hint).right_aligned().style(Style::default().fg(Color::Green)));
    }
    let Some(update) = &s.update else { return block };
    let commits = if update.ahead_by == 1 { "commit" } else { "commits" };
    let hint = if s.build.dev {
        format!(" {} is {} {commits} ahead ", crate::backend::update::BRANCH, update.ahead_by)
    } else {
        let short = update.latest.get(..7).unwrap_or(&update.latest);
        format!(" New version available ({} {commits}, {short})  u: upgrade ", update.ahead_by)
    };
    block.title(Line::from(hint).right_aligned().style(Style::default().fg(Color::Yellow)))
}

/// Renders the project tree, details pane, and command hints.
pub fn draw(f: &mut ratatui::Frame, s: &mut AppState) {
    if s.initializing {
        draw_loading(f, &s.build);
        return;
    }
    let a = Layout::vertical([Constraint::Min(8), Constraint::Length(5)]).split(f.area());
    let p =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(a[0]);
    let rs = s.visible_rows();
    let list_width = p[0].width.saturating_sub(2) as usize;
    // Budget for the widest visible project, including its depth below Home.
    // Use the longest status so attaching a client doesn't change the layout.
    let compact_status = rs
        .iter()
        .filter_map(|row| {
            let kinds = row.kinds.as_ref()?;
            let name = row.path.file_name().unwrap_or(row.path.as_os_str()).to_string_lossy();
            let labels = kinds.iter().map(ProjectKind::label).collect::<Vec<_>>().join(",");
            Some(
                3 + row.depth * 2
                    + Line::from(name.as_ref()).width()
                    + 1
                    + Line::from(format!("[{labels}]  ●  attached")).width(),
            )
        })
        .max()
        .unwrap_or(0)
        > list_width;
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
                        Some(_) if compact_status => "●",
                        None if compact_status => "○",
                        Some(session) if session.attached_clients > 0 => "●  attached",
                        Some(_) => "●   running",
                        None => "○   stopped",
                    }
                })
                .unwrap_or("");
            let mut left = format!(
                "{}{:indent$}{m} {n}",
                if r.path == s.selected { "›" } else { " " },
                "",
                indent = r.depth * 2
            );
            let labels = r
                .kinds
                .as_ref()
                .map(|kinds| kinds.iter().map(ProjectKind::label).collect::<Vec<_>>().join(","));
            let mut right = match (labels, session_status.is_empty()) {
                (Some(labels), false) => format!("[{labels}]  {session_status}"),
                (Some(labels), true) => format!("[{labels}]"),
                (None, false) => session_status.to_owned(),
                (None, true) => String::new(),
            };
            // Keep the state indicator visible even when names or indentation
            // exceed the available width. Drop labels only as a last resort.
            if compact_status && Line::from(right.as_str()).width() + 1 >= list_width {
                right = session_status.to_owned();
            }
            if !right.is_empty() {
                let left_width = list_width.saturating_sub(Line::from(right.as_str()).width() + 1);
                left = truncate_project_name(&left, left_width);
            }
            let gap = if right.is_empty() {
                String::new()
            } else {
                " ".repeat(
                    list_width
                        .saturating_sub(
                            Line::from(left.as_str()).width() + Line::from(right.as_str()).width(),
                        )
                        .max(usize::from(!left.is_empty())),
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
            Line::from("Enter: switch  o0–9: window  s: start  x: stop"),
            Line::from("t: test  l: lint  :: Just  r: refresh  H: hidden"),
            Line::from("/: find  a: sessions  Esc: close popup  q: quit"),
            Line::from(if s.confirmation.is_some() { "Confirm stop: y/n" } else { &s.status }),
        ])
        .block(commands_block(s)),
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
    if let Some(prompt) = &mut s.recipe_prompt {
        let matches = prompt.filtered_recipes();
        let selected = matches.iter().position(|(index, _)| Some(*index) == prompt.selected);
        let recipe_name = selected.map(|position| matches[position].1.name.as_str()).unwrap_or("");
        let height =
            prompt.listing.as_deref().unwrap_or("").lines().count().clamp(1, 8) as u16 + 10;
        let area = centered_rect(75, height, f.area());
        f.render_widget(Clear, area);
        let block =
            Block::bordered().title("Just recipes").border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let sections = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(inner);
        prompt.list_state.select(selected);
        let items = matches
            .iter()
            .map(|(_, recipe)| ListItem::new(recipe.display.clone()))
            .collect::<Vec<_>>();
        if items.is_empty() {
            f.render_widget(
                Paragraph::new(if prompt.listing.is_none() {
                    "Loading recipes..."
                } else {
                    "No matching recipes"
                }),
                sections[0],
            );
        } else {
            f.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("› ")
                    .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                    .scroll_padding(1),
                sections[0],
                &mut prompt.list_state,
            );
        }
        f.render_widget(
            Paragraph::new("↑/↓: select  Tab: filter/args  Enter: run  Esc: cancel")
                .style(Style::default().fg(Color::DarkGray)),
            sections[1],
        );
        f.render_widget(
            Paragraph::new(prompt.error.as_deref().unwrap_or(""))
                .style(Style::default().fg(Color::Red)),
            sections[2],
        );
        let input_block = Block::bordered().title("Filter recipes");
        let input_area = input_block.inner(sections[3]);
        let input = format!("> {}", prompt.input);
        let cursor_column = Line::from(format!("> {}", &prompt.input[..prompt.cursor])).width();
        let horizontal_scroll = cursor_column
            .saturating_sub(input_area.width.saturating_sub(1) as usize)
            .min(u16::MAX as usize) as u16;
        f.render_widget(
            Paragraph::new(input).block(input_block).scroll((0, horizontal_scroll)),
            sections[3],
        );
        if !prompt.editing_arguments && input_area.width > 0 && input_area.height > 0 {
            f.set_cursor_position((
                input_area.x
                    + (cursor_column - horizontal_scroll as usize)
                        .min(input_area.width as usize - 1) as u16,
                input_area.y,
            ));
        }
        let arguments_block = Block::bordered().title("Arguments (Tab)");
        let arguments_area = arguments_block.inner(sections[4]);
        let arguments = format!("just {recipe_name} {}", prompt.arguments);
        let cursor_column = Line::from(format!(
            "just {recipe_name} {}",
            &prompt.arguments[..prompt.argument_cursor]
        ))
        .width();
        let horizontal_scroll = cursor_column
            .saturating_sub(arguments_area.width.saturating_sub(1) as usize)
            .min(u16::MAX as usize) as u16;
        f.render_widget(
            Paragraph::new(arguments).block(arguments_block).scroll((0, horizontal_scroll)),
            sections[4],
        );
        if prompt.editing_arguments && arguments_area.width > 0 && arguments_area.height > 0 {
            f.set_cursor_position((
                arguments_area.x
                    + (cursor_column - horizontal_scroll as usize)
                        .min(arguments_area.width as usize - 1) as u16,
                arguments_area.y,
            ));
        }
    }
    if s.fuzzy.is_some() {
        draw_fuzzy(f, s);
    }
    if s.active_sessions.is_some() {
        draw_active_sessions(f, s);
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

/// Draws projects that currently have a tmux session.
fn draw_active_sessions(f: &mut ratatui::Frame, s: &AppState) {
    let projects = s.active_session_projects();
    let selected = s.active_sessions.as_ref().map(|popup| popup.selected).unwrap_or(0);
    let list_rows = projects.len().clamp(1, 12) as u16;
    let height = (list_rows + 3).min(f.area().height.saturating_sub(2)).max(5);
    let area = centered_rect(70, height, f.area());
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(format!("Active sessions ({})", projects.len()))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let sections = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    if projects.is_empty() {
        f.render_widget(Paragraph::new("No projects have an active session"), sections[0]);
    } else {
        let items = projects
            .iter()
            .map(|project| {
                let windows = match project.window_count {
                    1 => "1 window".to_owned(),
                    count => format!("{count} windows"),
                };
                let status = if project.attached_clients > 0 {
                    format!("attached ({})", project.attached_clients)
                } else {
                    "running".to_owned()
                };
                ListItem::new(Line::from(format!("{}  {status} · {windows}", project.display)))
            })
            .collect::<Vec<_>>();
        let mut state = ratatui::widgets::ListState::default();
        state.select(Some(selected.min(items.len() - 1)));
        f.render_stateful_widget(
            List::new(items)
                .highlight_symbol("› ")
                .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
                .scroll_padding(1),
            sections[0],
            &mut state,
        );
    }
    f.render_widget(
        Paragraph::new("Enter: switch    Esc: close").style(Style::default().fg(Color::DarkGray)),
        sections[1],
    );
}

/// Draws the fzf-style project finder: results above, prompt below.
fn draw_fuzzy(f: &mut ratatui::Frame, s: &AppState) {
    let results = s.fuzzy_results();
    let query = s.fuzzy.as_ref().map(|search| search.query.as_str()).unwrap_or("");
    let selected = s.fuzzy.as_ref().map_or(0, |search| search.selected);
    let area = centered_rect(80, f.area().height.saturating_sub(4).clamp(5, 20), f.area());
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(format!(
            "Find project {}/{}",
            results.len(),
            s.repositories.iter().map(|r| r.projects.len()).sum::<usize>()
        ))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let sections = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let items = results
        .iter()
        .map(|result| {
            let spans = result
                .display
                .chars()
                .enumerate()
                .map(|(index, character)| {
                    if result.positions.contains(&index) {
                        Span::styled(
                            character.to_string(),
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::raw(character.to_string())
                    }
                })
                .collect::<Vec<_>>();
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    let mut state = ratatui::widgets::ListState::default();
    state.select((!items.is_empty()).then_some(selected.min(items.len() - 1)));
    f.render_stateful_widget(
        List::new(items)
            .highlight_symbol("› ")
            .highlight_style(Style::default().bg(Color::DarkGray))
            .scroll_padding(1),
        sections[0],
        &mut state,
    );
    f.render_widget(
        Paragraph::new(format!("/ {query}")).style(Style::default().fg(Color::Cyan)),
        sections[1],
    );
    f.set_cursor_position((
        sections[1].x
            + (Line::from(format!("/ {query}")).width() as u16)
                .min(sections[1].width.saturating_sub(1)),
        sections[1].y,
    ));
}

fn truncate_project_name(text: &str, width: usize) -> String {
    if Line::from(text).width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut truncated = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = Line::from(character.to_string()).width();
        if used + character_width > width - 1 {
            break;
        }
        truncated.push(character);
        used += character_width;
    }
    truncated.push('…');
    truncated
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
    if let Some(session) = s.selected_session() {
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "tmux: {} · {}",
            session.name,
            if session.attached_clients > 0 {
                format!("attached ({} clients)", session.attached_clients)
            } else {
                "running".into()
            }
        )));
        for window in &session.windows {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "  {}: {}{}",
                        window.index,
                        window.name,
                        if window.active { " *" } else { "" }
                    ),
                    if window.active { Style::default().fg(Color::Cyan) } else { Style::default() },
                ),
                Span::raw(format!(
                    " · {} {} · {}",
                    window.pane_count,
                    if window.pane_count == 1 { "pane" } else { "panes" },
                    window.command
                )),
            ]));
        }
    }
    if let Some(listing) = &s.just_listing {
        lines.push(Line::from(""));
        lines.push(Line::from("Just recipes:"));
        lines.extend(listing.lines().map(Line::from));
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
            open_window_pending: false,
            session_backend: SessionBackend::default(),
            confirmation: None,
            project_scan_generation: 0,
            initializing: false,
            project_scan_pending: false,
            command_popup: None,
            recipe_prompt: None,
            fuzzy: None,
            active_sessions: None,
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
            just_listing: None,
            git_status_cache: Default::default(),
            git_status_loading: false,
            git_status_generation: 0,
            sessions_refresh_generation: 0,
            sessions_refresh_pending: false,
            build: Default::default(),
            update: None,
            upgraded: false,
        }
    }

    #[test]
    fn commands_panel_suggests_upgrade_only_when_master_is_ahead() {
        use crate::backend::update::UpdateInfo;
        let mut state = test_state();
        let render = |state: &mut AppState| {
            let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
            terminal.draw(|frame| draw(frame, state)).unwrap();
            terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>()
        };
        let text = render(&mut state);
        assert!(!text.contains("u: upgrade") && !text.contains("ahead"));
        state.update = Some(UpdateInfo { latest: "abcdef123456".into(), ahead_by: 3 });
        assert!(
            render(&mut state).contains("New version available (3 commits, abcdef1)  u: upgrade")
        );
        state.build.dev = true;
        let text = render(&mut state);
        assert!(text.contains(&format!("{} is 3 commits ahead", crate::backend::update::BRANCH)));
        assert!(!text.contains("u: upgrade"));
        state.build.dev = false;
        state.upgraded = true;
        let text = render(&mut state);
        assert!(text.contains("Upgraded: restart Canopy to apply"));
        assert!(!text.contains("u: upgrade"));
    }

    #[test]
    fn active_sessions_popup_lists_projects_with_sessions() {
        use crate::{
            app::ActiveSessions,
            backend::sessions::{Session, Window},
        };
        let mut state = test_state();
        let running = PathBuf::from("/home/test/repo");
        let attached = PathBuf::from("/home/test/repo/service");
        let idle = PathBuf::from("/home/test/other");
        state.repositories.push(Repository {
            path: running.clone(),
            projects: vec![
                Project::new(running.clone(), vec![ProjectKind::Git], None),
                Project::new(attached.clone(), vec![ProjectKind::Rust], None),
                Project::new(idle.clone(), vec![ProjectKind::Python], None),
            ],
        });
        state.sessions = vec![
            Session {
                name: Session::name_for_path(&running),
                attached_clients: 0,
                working_directory: running,
                windows: vec![Window {
                    index: 0,
                    id: "@1".into(),
                    name: "editor".into(),
                    active: true,
                    pane_count: 1,
                    command: "nvim".into(),
                    working_directory: PathBuf::from("/home/test/repo"),
                }],
            },
            Session {
                name: Session::name_for_path(&attached),
                attached_clients: 2,
                working_directory: attached,
                windows: vec![],
            },
        ];
        state.active_sessions = Some(ActiveSessions { selected: 1 });
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text: String =
            terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Active sessions (2)"));
        assert!(text.contains("repo  running · 1 window"));
        assert!(text.contains("repo/service  attached (2) · 0 windows"));
        assert!(text.contains("Enter: switch"));
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
    fn details_shows_windows_for_running_and_attached_sessions() {
        use crate::backend::sessions::{Session, Window};
        let mut state = test_state();
        state.repositories.push(Repository {
            path: state.selected.clone(),
            projects: vec![Project::new(state.selected.clone(), vec![ProjectKind::Git], None)],
        });
        state.sessions.push(Session {
            name: Session::name_for_path(&state.selected),
            attached_clients: 0,
            working_directory: state.selected.clone(),
            windows: vec![Window {
                index: 3,
                id: "@42".into(),
                name: "agent".into(),
                active: true,
                pane_count: 2,
                command: "codex".into(),
                working_directory: state.selected.clone(),
            }],
        });
        for clients in [0, 2] {
            state.sessions[0].attached_clients = clients;
            let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
            terminal.draw(|frame| frame.render_widget(details(&state), frame.area())).unwrap();
            let text: String =
                terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains(if clients == 0 { "running" } else { "attached (2 clients)" }));
            assert!(text.contains("3: agent * · 2 panes · codex"));
            assert!(text.contains("123456"));
        }
    }

    #[test]
    fn details_omits_missing_justfile_and_separates_recipe_listing_from_commits() {
        let mut state = test_state();
        let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
        terminal.draw(|frame| frame.render_widget(details(&state), frame.area())).unwrap();
        let text: String =
            terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(!text.contains("Just recipes:"));

        state.just_listing =
            Some("  deploy target  # Deploy to target\n  test           # Run tests".into());
        terminal.draw(|frame| frame.render_widget(details(&state), frame.area())).unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y| (1..99).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(row(5).contains("abcdef"));
        assert!(row(6).trim().is_empty());
        assert!(row(7).contains("Just recipes:"));
        assert!(row(8).contains("deploy target  # Deploy to target"));
        assert!(row(9).contains("test           # Run tests"));
    }

    #[test]
    fn recipe_prompt_shows_recipes_and_command_input_at_different_sizes() {
        let mut state = test_state();
        state.recipe_prompt = Some(crate::app::RecipePrompt::new(
            state.selected.clone(),
            state.selected.join("justfile"),
            0,
            Some("  deploy target  # Deploy\n  test           # Test".into()),
        ));
        let prompt = state.recipe_prompt.as_mut().unwrap();
        prompt.input = "dply".into();
        prompt.cursor = prompt.input.len();
        prompt.arguments = "'a long target name'".into();
        prompt.argument_cursor = prompt.arguments.len();
        prompt.editing_arguments = true;
        for (width, height) in [(100, 24), (32, 12), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            if width == 100 {
                let text: String =
                    terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
                assert!(text.contains("Just recipes"));
                assert!(text.contains("deploy target  # Deploy"));
                assert!(text.contains("just deploy 'a long target name'"));
            }
            if width > 1 {
                let cursor = terminal.get_cursor_position().unwrap();
                assert!(cursor.x < width && cursor.y < height);
            }
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
                assert!(height < 10 || text.contains("Build unknown"));
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
    fn project_status_compacts_based_on_names_depth_and_available_width() {
        use crate::backend::sessions::Session;
        // A top-level "repo" with [Git] needs 26 columns including full status.
        for (path, inner_width, compact) in [
            ("repo", 26, false),
            ("repo", 25, true),
            ("work/repos/repo", 26, true),
            ("some-long-project", 26, true),
            ("工具工具", 26, true),
            ("some-long-project", 12, true),
            ("repo", 8, true),
            ("repo", 1, true),
        ] {
            let mut state = test_state();
            state.selected = state.home.join(path);
            state.repositories = vec![Repository {
                path: state.selected.clone(),
                projects: vec![Project::new(state.selected.clone(), vec![ProjectKind::Git], None)],
            }];
            for ancestor in state.selected.ancestors().skip(1) {
                state.expanded.insert(ancestor.to_path_buf());
            }
            let row =
                state.visible_rows().iter().position(|row| row.path == state.selected).unwrap()
                    as u16
                    + 1;
            for clients in [None, Some(0), Some(1)] {
                state.sessions = clients
                    .map(|attached_clients| Session {
                        name: Session::name_for_path(&state.selected),
                        attached_clients,
                        working_directory: state.selected.clone(),
                        windows: Vec::new(),
                    })
                    .into_iter()
                    .collect();
                let mut terminal =
                    Terminal::new(TestBackend::new((inner_width + 2) * 2, 24)).unwrap();
                terminal.draw(|frame| draw(frame, &mut state)).unwrap();
                let buffer = terminal.backend().buffer();
                let text: String = (1..=inner_width).map(|x| buffer[(x, row)].symbol()).collect();
                let indicator = if clients.is_some() { "●" } else { "○" };
                assert!(text.contains(indicator), "{path} at {inner_width}: {text}");
                let status = match clients {
                    None => "stopped",
                    Some(0) => "running",
                    Some(_) => "attached",
                };
                assert_eq!(text.contains(status), !compact, "{path} at {inner_width}: {text}");
                if !compact {
                    assert!(text.contains("repo"));
                }
                if path == "some-long-project" && inner_width == 12 {
                    assert!(text.contains('…'));
                }
            }
        }
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
