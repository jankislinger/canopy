use crate::app::{AppState, kind, rows};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};
/// Renders the project tree, details pane, and command hints.
pub fn draw(f: &mut ratatui::Frame, s: &AppState) {
    let a = Layout::vertical([Constraint::Min(8), Constraint::Length(5)]).split(f.area());
    let p =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(a[0]);
    let rs = rows(&s.home, &s.repositories, &s.expanded);
    let items = rs
        .iter()
        .map(|r| {
            let child = rs.iter().any(|x| x.path.parent() == Some(r.path.as_path()));
            let m =
                if child { if s.expanded.contains(&r.path) { "▾" } else { "▸" } } else { " " };
            let k = r.kinds.as_ref().map(|x| x.iter().map(kind).collect::<Vec<_>>().join(","));
            let n = r.path.file_name().unwrap_or(r.path.as_os_str()).to_string_lossy();
            let session_status = r
                .kinds
                .as_ref()
                .map(|_| {
                    let name = crate::backend::sessions::session_name(&r.path);
                    match s.sessions.iter().find(|session| session.name == name) {
                        Some(session) if session.attached_clients > 0 => "● attached",
                        Some(_) => "● running",
                        None => "○ stopped",
                    }
                })
                .unwrap_or("");
            let text = format!(
                "{}{:indent$}{m} {} {} {session_status}",
                if r.path == s.selected { "›" } else { " " },
                "",
                k.map(|x| format!("[{x}]")).unwrap_or_default(),
                n,
                indent = r.depth * 2
            );
            ListItem::new(Line::from(Span::styled(
                text,
                if r.path == s.selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            )))
        })
        .collect::<Vec<_>>();
    f.render_widget(List::new(items).block(Block::bordered().title("Projects")), p[0]);
    f.render_widget(
        Paragraph::new(s.selected.display().to_string()).block(Block::bordered().title("Details")),
        p[1],
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from("Enter: switch  s: start  x: stop"),
            Line::from("t: test  l: lint  r: refresh"),
            Line::from("Esc: close popup  q: quit"),
            Line::from(if s.confirmation.is_some() { "Confirm stop: y/n" } else { &s.status }),
        ])
        .block(Block::bordered().title("Commands")),
        a[1],
    );
    if s.confirmation.is_some() {
        let popup = centered_rect(60, 7, f.area());
        f.render_widget(Clear, popup);
        f.render_widget(
            Paragraph::new(vec![
                Line::from("Stop the selected tmux session?"),
                Line::from(s.selected.display().to_string()),
                Line::from("[y] confirm    [n/Esc] cancel"),
            ])
            .block(Block::bordered().title("Confirm stop")),
            popup,
        );
    }
    if let Some(popup) = &s.command_popup {
        let area = centered_rect(85, 20, f.area());
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(popup.output.as_str())
                .block(Block::bordered().title(popup.title.as_str()))
                .scroll((popup.scroll, 0))
                .wrap(ratatui::widgets::Wrap { trim: false }),
            area,
        );
    }
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
