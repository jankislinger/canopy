use crate::app::{AppState, kind, rows};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
};
/// Renders the project tree, details pane, and experiments panel.
pub fn draw(f: &mut ratatui::Frame, s: &AppState) {
    let a = Layout::vertical([Constraint::Min(8), Constraint::Length(5)]).split(f.area());
    let p =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(a[0]);
    let rs = rows(&s.home, &s.repositories, &s.expanded);
    let items = rs
        .iter()
        .map(|r| {
            let child = rs.iter().any(|x| x.path.parent() == Some(r.path.as_path()));
            let m = if child {
                if s.expanded.contains(&r.path) {
                    "▾"
                } else {
                    "▸"
                }
            } else {
                " "
            };
            let k = r
                .kinds
                .as_ref()
                .map(|x| x.iter().map(kind).collect::<Vec<_>>().join(","));
            let n = r
                .path
                .file_name()
                .unwrap_or(r.path.as_os_str())
                .to_string_lossy();
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
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            )))
        })
        .collect::<Vec<_>>();
    f.render_widget(
        List::new(items).block(Block::bordered().title("Projects")),
        p[0],
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(s.selected.display().to_string()),
            Line::from("Enter: switch  s: start  x: stop"),
            Line::from("r: refresh  q: quit"),
            Line::from(if s.stop_confirmation {
                "Confirm stop: y/n"
            } else {
                &s.status
            }),
        ])
        .block(Block::bordered().title("Details")),
        p[1],
    );
    f.render_widget(
        Paragraph::new("Hello, world!").block(Block::bordered().title("Experiments")),
        a[1],
    );
}
