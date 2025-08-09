use codex_core::config_types::ReasoningEffort;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;

use super::bottom_pane_view::BottomPaneView;
use super::BottomPane;

/// Interactive UI for selecting reasoning effort level
pub(crate) struct ReasoningSelectionView {
    current_effort: ReasoningEffort,
    selected_effort: ReasoningEffort,
    app_event_tx: AppEventSender,
    is_complete: bool,
}

impl ReasoningSelectionView {
    pub fn new(current_effort: ReasoningEffort, app_event_tx: AppEventSender) -> Self {
        Self {
            current_effort,
            selected_effort: current_effort,
            app_event_tx,
            is_complete: false,
        }
    }

    fn get_effort_options() -> Vec<(ReasoningEffort, &'static str, &'static str)> {
        vec![
            (ReasoningEffort::None, "None", "No reasoning (fastest)"),
            (ReasoningEffort::Low, "Low", "Basic reasoning"),
            (ReasoningEffort::Medium, "Medium", "Balanced reasoning (default)"),
            (ReasoningEffort::High, "High", "Deep reasoning (slower)"),
        ]
    }

    fn move_selection_up(&mut self) {
        let options = Self::get_effort_options();
        let current_idx = options
            .iter()
            .position(|(e, _, _)| *e == self.selected_effort)
            .unwrap_or(0);
        
        let new_idx = if current_idx == 0 {
            options.len() - 1
        } else {
            current_idx - 1
        };
        
        self.selected_effort = options[new_idx].0;
    }

    fn move_selection_down(&mut self) {
        let options = Self::get_effort_options();
        let current_idx = options
            .iter()
            .position(|(e, _, _)| *e == self.selected_effort)
            .unwrap_or(0);
        
        let new_idx = (current_idx + 1) % options.len();
        self.selected_effort = options[new_idx].0;
    }

    fn confirm_selection(&self) {
        // Send event to update reasoning effort
        self.app_event_tx.send(AppEvent::UpdateReasoningEffort(self.selected_effort));
    }
}

impl<'a> BottomPaneView<'a> for ReasoningSelectionView {
    fn handle_key_event(&mut self, _pane: &mut BottomPane<'a>, key_event: KeyEvent) {
        match key_event {
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_selection_up();
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_selection_down();
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.confirm_selection();
                self.is_complete = true;
            }
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.is_complete = true;
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.is_complete
    }

    fn desired_height(&self, _width: u16) -> u16 {
        10 // Height for the selection box
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        // Clear the area first
        Clear.render(area, buf);

        // Create a centered box
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue))
            .title(" Select Reasoning Effort ")
            .title_alignment(Alignment::Center);

        let inner_area = block.inner(area);
        block.render(area, buf);

        // Build the content
        let mut lines = vec![
            Line::from(vec![
                Span::raw("Current: "),
                Span::styled(
                    format!("{}", self.current_effort),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        // Add options
        for (effort, name, description) in Self::get_effort_options() {
            let is_selected = effort == self.selected_effort;
            let is_current = effort == self.current_effort;
            
            let mut style = Style::default();
            if is_selected {
                style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
            }
            if is_current {
                style = style.fg(Color::Yellow);
            }

            let prefix = if is_selected { "▶ " } else { "  " };
            let line = Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("{:<8}", name), style),
                Span::raw(" - "),
                Span::styled(description, Style::default().fg(Color::Gray)),
            ]);
            lines.push(line);
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(Color::Cyan)),
            Span::raw(" Navigate  "),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" Select  "),
            Span::styled("Esc", Style::default().fg(Color::Red)),
            Span::raw(" Cancel"),
        ]));

        let paragraph = Paragraph::new(lines)
            .alignment(Alignment::Left);
        paragraph.render(inner_area, buf);
    }
}