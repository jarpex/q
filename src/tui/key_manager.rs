//! The screen that manages the API keys of the user.
//!
//! The screen shows every token with its middle hidden, so a token never
//! reaches the screen of a recording or a screenshot. A new token is typed in
//! the terminal, so it never reaches the history of the shell.

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use zeroize::Zeroizing;

use crate::config::{mask_token, ApiKey, KeyStore};

/// The screen that is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// The list of the tokens.
    Keys,
    /// The list of the platforms to add a token for.
    Platforms,
    /// The fields to type a token and an address in.
    Token,
}

/// The field that takes the characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The token itself.
    Token,
    /// The address of a local server.
    Endpoint,
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The user left the screen without changing anything.
    Cancel,
    /// The user saved the tokens.
    Save(Box<KeyStore>),
}

/// The state of the screen.
pub struct KeyManager {
    /// The screen that is shown.
    screen: Screen,
    /// The tokens of the user.
    store: KeyStore,
    /// The platform that the user picked for the new token.
    platform: Option<String>,
    /// The platforms that a new token can be added for.
    platforms: Vec<String>,
    /// The token that the user types.
    input: Zeroizing<String>,
    /// The address of a local server, when the platform needs one.
    endpoint: String,
    /// The line of the list that is selected.
    selected: usize,
    /// The line of the platform list that is selected.
    platform_selected: usize,
    /// The message that tells what happened.
    message: Option<String>,
    /// The message is an error, not a note.
    message_is_error: bool,
    /// The field that takes the characters.
    field: Field,
}

impl KeyManager {
    /// Creates the screen with the tokens of the user.
    ///
    /// The platforms come from the provider library, so the list always
    /// matches the platforms the tool can talk to.
    #[must_use]
    pub fn new(store: KeyStore, platforms: Vec<String>) -> Self {
        Self {
            screen: Screen::Keys,
            store,
            platform: None,
            platforms,
            input: Zeroizing::new(String::new()),
            endpoint: String::new(),
            selected: 0,
            platform_selected: 0,
            message: None,
            message_is_error: false,
            field: Field::Token,
        }
    }

    /// The tokens of the user.
    #[must_use]
    pub const fn store(&self) -> &KeyStore {
        &self.store
    }

    /// Runs the screen until the user leaves it.
    ///
    /// # Errors
    ///
    /// Returns an error when the terminal cannot be set up.
    pub fn run<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<Action> {
        loop {
            terminal
                .draw(|frame| self.draw(frame))
                .map_err(|error| anyhow::anyhow!("Cannot draw the screen: {error}"))?;

            let Event::Key(key) = crossterm::event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if let Some(action) = self.handle_key(key) {
                return Ok(action);
            }
        }
    }

    /// Handles a key and returns the action when the screen ends.
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self.screen {
            Screen::Keys => self.handle_keys_screen(key),
            Screen::Platforms => self.handle_platforms_screen(key),
            Screen::Token => self.handle_token_screen(key),
        }
    }

    /// The keys of the list of the tokens.
    fn handle_keys_screen(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Char('a') => {
                self.screen = Screen::Platforms;
                self.platform_selected = 0;
                self.message = None;
                None
            }
            KeyCode::Char('d') => {
                self.remove_selected();
                None
            }
            KeyCode::Char('s') => Some(Action::Save(Box::new(self.store.clone()))),
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            _ => None,
        }
    }

    /// The keys of the list of the platforms.
    fn handle_platforms_screen(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.screen = Screen::Keys;
                self.platform = None;
                None
            }
            KeyCode::Enter => {
                let Some(platform) = self.platforms.get(self.platform_selected).cloned() else {
                    self.screen = Screen::Keys;
                    return None;
                };
                self.platform = Some(platform);
                self.screen = Screen::Token;
                self.input.clear();
                self.endpoint.clear();
                self.field = Field::Token;
                self.message = None;
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_platform_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_platform_selection(1);
                None
            }
            _ => None,
        }
    }

    /// The keys of the field of the token.
    fn handle_token_screen(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::Keys;
                self.input.clear();
                self.endpoint.clear();
                None
            }
            KeyCode::Enter => {
                self.save_token();
                None
            }
            KeyCode::Tab => {
                self.field = match self.field {
                    Field::Token => Field::Endpoint,
                    Field::Endpoint => Field::Token,
                };
                None
            }
            KeyCode::Backspace => {
                match self.field {
                    Field::Token => {
                        self.input.pop();
                    }
                    Field::Endpoint => {
                        self.endpoint.pop();
                    }
                }
                None
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.endpoint.clear();
                None
            }
            KeyCode::Char(character) => {
                match self.field {
                    Field::Token => self.input.push(character),
                    Field::Endpoint => self.endpoint.push(character),
                }
                None
            }
            _ => None,
        }
    }

    /// Moves the selection of the list of the tokens.
    fn move_selection(&mut self, step: isize) {
        if self.store.keys.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.store.keys.len() - 1;
        self.selected = self.selected.saturating_add_signed(step).min(last);
    }

    /// Moves the selection of the list of the platforms.
    fn move_platform_selection(&mut self, step: isize) {
        if self.platforms.is_empty() {
            self.platform_selected = 0;
            return;
        }
        let last = self.platforms.len() - 1;
        self.platform_selected = self.platform_selected.saturating_add_signed(step).min(last);
    }

    /// Removes the token that is selected.
    fn remove_selected(&mut self) {
        let Some(key) = self.store.keys.get(self.selected) else {
            return;
        };
        let platform = key.platform.clone();
        self.store.remove(&platform);
        self.selected = self.selected.min(self.store.keys.len().saturating_sub(1));
        self.message = Some(format!("Removed the token of {platform}"));
        self.message_is_error = false;
    }

    /// Saves the token that the user typed.
    fn save_token(&mut self) {
        let Some(platform) = self.platform.clone() else {
            self.screen = Screen::Keys;
            return;
        };
        let token = self.input.to_string();
        if token.trim().is_empty() {
            self.message = Some(String::from("The token cannot be empty"));
            self.message_is_error = true;
            return;
        }

        let endpoint = self.endpoint.trim();
        self.store.upsert(ApiKey {
            platform,
            token: Zeroizing::new(token),
            endpoint: if endpoint.is_empty() {
                None
            } else {
                Some(endpoint.to_owned())
            },
        });
        self.input.clear();
        self.endpoint.clear();
        self.platform = None;
        self.screen = Screen::Keys;
        self.selected = self
            .store
            .keys
            .iter()
            .position(|key| key.platform == self.selected_platform())
            .unwrap_or(0);
        self.message = Some(String::from("Token saved"));
        self.message_is_error = false;
    }

    /// The platform of the selected token.
    fn selected_platform(&self) -> String {
        self.store
            .keys
            .get(self.selected)
            .map_or_else(String::new, |key| key.platform.clone())
    }

    /// Draws the screen.
    fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(area);
        let (Some(header_area), Some(body_area), Some(footer_area)) =
            (chunks.first(), chunks.get(1), chunks.get(2))
        else {
            return;
        };

        let title = match self.screen {
            Screen::Keys => " API keys ",
            Screen::Platforms => " Add a token: pick the platform ",
            Screen::Token => " Add a token: type it ",
        };
        let header = Paragraph::new(Line::from(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(header, *header_area);

        match self.screen {
            Screen::Keys => self.draw_keys(frame, *body_area),
            Screen::Platforms => self.draw_platforms(frame, *body_area),
            Screen::Token => self.draw_token(frame, *body_area),
        }

        let footer = self.draw_footer();
        frame.render_widget(footer, *footer_area);
    }

    /// Draws the list of the tokens.
    fn draw_keys(&self, frame: &mut Frame<'_>, area: Rect) {
        if self.store.keys.is_empty() {
            let empty = Paragraph::new(Line::from(Span::styled(
                "No tokens yet. Press a to add one.",
                Style::default().fg(Color::DarkGray),
            )))
            .block(Block::default().borders(Borders::ALL).title(" Tokens "));
            frame.render_widget(empty, area);
            return;
        }

        let items: Vec<ListItem<'_>> = self
            .store
            .keys
            .iter()
            .map(|key| {
                let masked = mask_token(key.token.as_str());
                let line = Line::from(vec![
                    Span::styled(
                        format!("{:<18}", key.platform),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(masked),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Tokens "))
            .highlight_symbol("> ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut state = ListState::default();
        state.select(Some(self.selected));
        frame.render_stateful_widget(list, area, &mut state);
    }

    /// Draws the list of the platforms.
    fn draw_platforms(&self, frame: &mut Frame<'_>, area: Rect) {
        let items: Vec<ListItem<'_>> = self
            .platforms
            .iter()
            .map(|platform| ListItem::new(Line::from(Span::raw(platform.clone()))))
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Platforms (Esc to go back) "),
            )
            .highlight_symbol("> ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut state = ListState::default();
        state.select(Some(self.platform_selected));
        frame.render_stateful_widget(list, area, &mut state);
    }

    /// Draws the field of the token.
    fn draw_token(&self, frame: &mut Frame<'_>, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(area);
        let (Some(field_area), Some(buttons_area)) = (chunks.first(), chunks.get(1)) else {
            return;
        };

        let field = Paragraph::new(self.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(format!(
                " Token for {} ",
                self.platform.as_deref().unwrap_or("")
            )))
            .style(Style::default().fg(Color::White));
        frame.render_widget(field, *field_area);

        let buttons = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(*buttons_area);
        let (Some(save_area), Some(cancel_area)) = (buttons.first(), buttons.get(1)) else {
            return;
        };

        let save = Paragraph::new(Line::from(Span::styled(
            " Save (Enter) ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(save, *save_area);

        let cancel = Paragraph::new(Line::from(Span::styled(
            " Cancel (Esc) ",
            Style::default().fg(Color::DarkGray),
        )))
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(cancel, *cancel_area);
    }

    /// Draws the line of the keys and the message.
    fn draw_footer(&self) -> Paragraph<'static> {
        let keys = match self.screen {
            Screen::Keys => " up/down: move  a: add  d: delete  s: save  q: quit ",
            Screen::Platforms => " up/down: move  Enter: pick  Esc: back ",
            Screen::Token => " Tab: next field  Enter: save  Esc: back ",
        };
        let mut spans = vec![Span::styled(keys, Style::default().fg(Color::DarkGray))];

        if let Some(message) = &self.message {
            let color = if self.message_is_error {
                Color::Red
            } else {
                Color::Green
            };
            spans.push(Span::raw("  "));
            spans.push(Span::styled(message.clone(), Style::default().fg(color)));
        }

        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true })
    }
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{Action, KeyManager, Screen};
    use crate::config::{ApiKey, KeyStore};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use zeroize::Zeroizing;

    fn manager() -> KeyManager {
        let store = KeyStore {
            keys: vec![ApiKey {
                platform: "openai".to_owned(),
                token: Zeroizing::new("sk-secret-value".to_owned()),
                endpoint: None,
            }],
        };
        KeyManager::new(store, vec!["openai".to_owned(), "anthropic".to_owned()])
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_screen_starts_with_the_list_of_the_tokens() {
        assert_eq!(manager().screen, Screen::Keys);
    }

    #[test]
    fn the_add_key_opens_the_list_of_the_platforms() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));

        assert_eq!(screen.screen, Screen::Platforms);
    }

    #[test]
    fn the_platform_list_goes_back_to_the_tokens() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Esc));

        assert_eq!(screen.screen, Screen::Keys);
    }

    #[test]
    fn the_platform_list_goes_back_with_the_quit_key() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Char('q')));

        assert_eq!(screen.screen, Screen::Keys);
    }

    #[test]
    fn the_selection_of_the_platform_moves() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Down));

        assert_eq!(screen.platform_selected, 1);
    }

    #[test]
    fn the_selection_of_the_platform_stays_in_the_list() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Up));

        assert_eq!(screen.platform_selected, 0);
    }

    #[test]
    fn a_chosen_platform_opens_the_field_of_the_token() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Down));
        screen.handle_key(key(KeyCode::Enter));

        assert_eq!(screen.screen, Screen::Token);
        assert_eq!(screen.platform.as_deref(), Some("anthropic"));
    }

    #[test]
    fn the_field_of_the_token_takes_the_characters() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        for character in "sk-abc".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }

        assert_eq!(screen.input.as_str(), "sk-abc");
    }

    #[test]
    fn the_field_of_the_token_takes_the_backspace() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Backspace));

        assert_eq!(screen.input.as_str(), "");
    }

    #[test]
    fn a_saved_token_goes_into_the_store() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        for character in "sk-new".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        screen.handle_key(key(KeyCode::Enter));

        assert_eq!(screen.screen, Screen::Keys);
        assert_eq!(screen.store().keys.len(), 1);
        let key = screen.store().keys.first();
        assert!(
            key.is_some_and(|item| item.token.as_str() == "sk-new"),
            "{key:?}"
        );
    }

    #[test]
    fn an_empty_token_is_not_saved() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        screen.handle_key(key(KeyCode::Enter));

        assert_eq!(screen.screen, Screen::Token);
        assert!(screen.message.is_some());
    }

    #[test]
    fn the_field_of_the_token_goes_back_without_saving() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        screen.handle_key(key(KeyCode::Char('x')));
        screen.handle_key(key(KeyCode::Esc));

        assert_eq!(screen.screen, Screen::Keys);
        let key = screen.store().keys.first();
        assert!(
            key.is_some_and(|item| item.token.as_str() == "sk-secret-value"),
            "{key:?}"
        );
    }

    #[test]
    fn the_delete_key_removes_the_selected_token() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('d')));

        assert_eq!(screen.store().keys, Vec::new());
    }

    #[test]
    fn the_quit_key_leaves_the_screen() {
        let mut screen = manager();

        assert_eq!(
            screen.handle_key(key(KeyCode::Char('q'))),
            Some(Action::Cancel)
        );
    }

    #[test]
    fn the_save_key_returns_the_store() {
        let mut screen = manager();

        let action = screen.handle_key(key(KeyCode::Char('s')));
        assert!(matches!(action, Some(Action::Save(_))));
    }

    #[test]
    fn the_tab_moves_between_the_fields() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));

        assert_eq!(screen.field, super::Field::Token);
        screen.handle_key(key(KeyCode::Tab));
        assert_eq!(screen.field, super::Field::Endpoint);
        screen.handle_key(key(KeyCode::Tab));
        assert_eq!(screen.field, super::Field::Token);
    }

    #[test]
    fn the_address_of_a_local_server_is_typed_in_the_second_field() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        for character in "sk-token".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        screen.handle_key(key(KeyCode::Tab));
        for character in "http://127.0.0.1:1234/v1".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        screen.handle_key(key(KeyCode::Enter));

        let key = screen.store().keys.first();
        assert_eq!(
            key.and_then(|item| item.endpoint.as_deref()),
            Some("http://127.0.0.1:1234/v1"),
            "{key:?}"
        );
    }

    #[test]
    fn a_token_without_an_address_has_no_address() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        for character in "sk-token".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        screen.handle_key(key(KeyCode::Enter));

        let key = screen.store().keys.first();
        assert_eq!(
            key.and_then(|item| item.endpoint.as_deref()),
            None,
            "{key:?}"
        );
    }

    #[test]
    fn the_selection_of_the_token_moves() {
        let mut screen = manager();
        screen.store.upsert(ApiKey {
            platform: "anthropic".to_owned(),
            token: Zeroizing::new("sk-other".to_owned()),
            endpoint: None,
        });

        screen.handle_key(key(KeyCode::Down));
        assert_eq!(screen.selected, 1);

        screen.handle_key(key(KeyCode::Up));
        assert_eq!(screen.selected, 0);
    }

    #[test]
    fn the_selection_of_the_token_stays_in_the_list() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Up));

        assert_eq!(screen.selected, 0);
    }
}
