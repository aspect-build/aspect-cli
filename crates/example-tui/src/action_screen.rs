use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, List, ListDirection, ListState, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::{Frame, Terminal, layout::*};

#[derive(Debug, Clone, Default)]
pub struct ActionScreen {}

impl ActionScreen {
    pub fn draw(&self, frame: &mut Frame<'_>) {
        let main_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(frame.area());

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .title_top(
                Line::from(" Actions ")
                    .style(Style::default().fg(Color::Green))
                    .centered(),
            )
            .title_top(
                Line::from("  10/1445 ")
                    .style(Style::default().bold())
                    .right_aligned(),
            )
            .title_bottom(
                Line::from(vec![Span::styled(
                    " Select <enter> | ↑ | ↓ ",
                    Style::new().bold(),
                )])
                .centered(),
            );

        let inner_chunks = Layout::default()
            .direction(Direction::Vertical)
            .vertical_margin(0)
            .horizontal_margin(2)
            .constraints(vec![Constraint::Min(5)])
            .split(main_layout[1]);

        let item = {
            let fail = false;
            Text::from(Line::from(vec![
                Span::styled(if fail { "🧯" } else { "⏳" }, Style::default()),
                Span::styled(
                    " TsProject",
                    Style::default()
                        .fg(if fail { Color::Red } else { Color::Blue })
                        .bold(),
                ),
                Span::styled(format!(" @@//app/{}:pkg", 0), Style::default()),
                Span::styled(" ....  ", Style::default()),
                if fail {
                    Span::styled("failed", Style::default().fg(Color::Red).italic())
                } else {
                    Span::styled("", Style::default())
                },
            ]))
        };
        let items = vec![item.clone(), item.clone(), item.clone()];

        let mut state = ListState::default().with_selected(Some(0));
        let list = List::new(items)
            .block(block)
            .style(Style::new().white())
            .highlight_style(Style::new().italic().fg(Color::Green))
            .highlight_symbol(" ▐ ")
            .direction(ListDirection::TopToBottom);

        frame.render_stateful_widget(list, main_layout[1], &mut state);

        let mut scrollbar_state = ScrollbarState::default().content_length(4).position(0);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .thumb_symbol("|")
            .thumb_style(Style::new().bold())
            .track_symbol(None)
            .end_symbol(Some("↓"));

        frame.render_stateful_widget(
            scrollbar,
            main_layout[1].inner(Margin::new(1, 1)),
            &mut scrollbar_state,
        );
    }
}
