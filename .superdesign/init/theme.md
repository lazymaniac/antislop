# Existing tokens
Terminal default foreground/background; active border Cyan; selection BOLD|REVERSED. ASCII borders and NO_COLOR are supported. Monospace from terminal. Header/footer 2 rows; chat editor 3 or 7 rows; graph node width 25, 5 rows tall, column spacing 34, row spacing 7. No CSS, font assets, logos, themes, shadows or browser breakpoints exist.
## Raw style implementation
```rust
use super::{
    app::{App, Focus, View},
    model::{AGENT_TABS, RUN_TABS},
    sanitize::sanitize,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn style(app: &App, active: bool) -> Style {
    if app.no_color {
        if active {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default()
        }
    } else if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}
fn block<'a>(app: &App, title: impl Into<Line<'a>>, active: bool) -> Block<'a> {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(style(app, active));
    if app.ascii {
        block.border_set(ratatui::symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        })
    } else {
        block
    }
}

```
