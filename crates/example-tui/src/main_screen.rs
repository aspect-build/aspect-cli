use ratatui::Frame;
use ratatui::layout::*;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListDirection, ListState, Paragraph};
use serde::{Deserialize, Serialize};

const LOGO: &'static str = include_str!("./bazel.txt");

#[derive(Default, Deserialize)]
pub struct ScreenState {
    number_of_actions: u32,
    build_state: BuildState,
    version: String,
    #[serde(skip)]
    pub memory: String,
    actions: Vec<Action>,
}

#[derive(Default, Deserialize)]
pub struct Action {
    mnemonic: String,
    label: String,
    state: ActionState,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionState {
    Started,
    Finished,
    Failed,
    Cancelled,
    #[default]
    Idle,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildState {
    Running,
    Finished,
    Failed,
    Cancelled,
    #[default]
    Idle,
}

impl ToString for BuildState {
    fn to_string(&self) -> String {
        match self {
            BuildState::Running => String::from("Running"),
            BuildState::Finished => String::from("Finished"),
            BuildState::Failed => String::from("Failed"),
            BuildState::Cancelled => String::from("Cancelled"),
            BuildState::Idle => String::from("Idle"),
        }
    }
}

#[derive(Default)]
pub struct MainScreen {
    pub state: ScreenState,
}

impl MainScreen {
    pub fn set_state(&mut self, state: ScreenState) {
        self.state = state;
    }

    pub fn draw(&self, frame: &mut Frame<'_>) {
        self.draw_info(frame, frame.area());
    }

    fn draw_info(&self, frame: &mut Frame<'_>, area: Rect) {
        let layout = Layout::horizontal(vec![
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ])
        .split(area.inner(Margin::new(3, 2)));

        let p = Paragraph::new(Text::from(
            LOGO.lines()
                .map(|l| {
                    Line::from(
                        l.chars()
                            .map(|c| {
                                if c == '%' {
                                    Span::styled(
                                        String::from(c),
                                        Style::new().bold().fg(Color::Rgb(67, 160, 71)),
                                    )
                                } else if c == '*' {
                                    Span::styled(String::from(c), Style::new().bold().green())
                                } else if c == '#' {
                                    Span::styled(String::from(c), Style::new().green())
                                } else {
                                    Span::styled(
                                        String::from(c),
                                        Style::new().fg(Color::Rgb(117, 210, 117)),
                                    )
                                }
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        ))
        .style(Style::new().slow_blink());
        frame.render_widget(p, layout[0]);

        let mlayout =
            Layout::vertical(vec![Constraint::Percentage(30), Constraint::Percentage(70)])
                .split(layout[1]);

        let build_state = self.state.build_state.to_string();
        let num_actions = self.state.number_of_actions.to_string();
        let info = vec![
            ("Build State", build_state.as_str()),
            ("Number of Actions", num_actions.as_str()),
            ("Version", self.state.version.as_str()),
            ("Peak Allocation", self.state.memory.as_str()),
        ];
        use tui_big_text::{BigText, PixelSize};
        let p = Paragraph::new(Text::from(
            info.into_iter()
                .map(|(k, v)| {
                    Line::from(vec![
                        Span::styled(k, Style::default().green()),
                        Span::styled(" : ", Style::default().green().bold()),
                        Span::styled(v, Style::default().bold()),
                    ])
                })
                .collect::<Vec<_>>(),
        ));

        frame.render_widget(p, mlayout[0]);

        let p = Paragraph::new(
            Text::from(
                r#"
 ____    __    ____  ____  __
(  _ \  /__\  (_   )( ___)(  )
 ) _ < /(__)\  / /_  )__)  )(__
(____/(__)(__)(____)(____)(____)
         ___    ___
        ( _ )  / _ \
        / _ \ ( (_) )
        \___/()\___/
"#,
            )
            .style(Style::default().blue().bold()),
        );
        frame.render_widget(p, mlayout[1]);

        // let big_text = BigText::builder()
        //     .pixel_size(PixelSize::HalfWidth)
        //     .style(Style::new().blue())
        //     .lines(vec!["10s".green().into(), "~~~".blue().into()])
        //     .build();

        // let cent = center(
        //     layout[2],
        //     Constraint::Percentage(50),
        //     Constraint::Percentage(50),
        // );
        // frame.render_widget(big_text, cent);
        //
        let items = self
            .state
            .actions
            .iter()
            .rev()
            .take(10)
            .rev()
            .map(|action| {
                Text::from(Line::from(vec![
                    Span::styled(
                        &action.mnemonic,
                        Style::default()
                            .fg(match action.state {
                                ActionState::Started => Color::Blue,
                                ActionState::Failed => Color::Red,
                                ActionState::Finished => Color::Green,
                                ActionState::Cancelled => Color::Yellow,
                                ActionState::Idle => Color::Blue,
                            })
                            .bold(),
                    ),
                    Span::from(" "),
                    Span::styled(&action.label, Style::default()),
                    Span::from(" "),
                    Span::styled(
                        match action.state {
                            ActionState::Started => "⏱️ ",
                            ActionState::Failed => "🧯 ",
                            ActionState::Finished => "👌",
                            ActionState::Cancelled => "🪇 ",
                            ActionState::Idle => "⏳",
                        },
                        Style::default(),
                    ),
                    match action.state {
                        ActionState::Failed => {
                            Span::styled("failed", Style::default().fg(Color::Red).italic())
                        }
                        ActionState::Cancelled => {
                            Span::styled("cancelled", Style::default().fg(Color::Yellow).italic())
                        }
                        _ => Span::styled("", Style::default()),
                    },
                ]))
            })
            .collect::<Vec<_>>();

        let mut state = ListState::default().with_selected(Some(0));
        let list = List::new(items)
            .style(Style::new().white())
            // .highlight_style(Style::new().italic().fg(Color::Green))
            .highlight_symbol(" ▐ ")
            .direction(ListDirection::TopToBottom);

        frame.render_stateful_widget(list, layout[2], &mut state);
    }
}

fn center(area: Rect, horizontal: Constraint, vertical: Constraint) -> Rect {
    let [area] = Layout::horizontal([horizontal])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([vertical]).flex(Flex::Center).areas(area);
    area
}
