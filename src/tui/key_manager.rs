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

/// A provider that the search found, with the score of the match.
struct Match {
    /// The score of the match. The higher the better.
    score: usize,
    /// The name of the provider.
    platform: String,
}

/// Scores a provider against the letters of the search.
///
/// The letters have to come in the same order, but not next to each other,
/// so `or` finds `open_router`. The score is higher when the letters sit
/// close to each other and at the start of the name, so the best match comes
/// first.
fn fuzzy_match(platform: &str, query: &[char]) -> Option<usize> {
    let name: Vec<char> = platform.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0_usize;
    let mut at = 0_usize;

    for letter in query {
        let rest = name.get(at..)?;
        let found = rest.iter().position(|character| character == letter)?;
        let position = at + found;
        // The letters that follow each other are worth more.
        score += 10 - position.min(9);
        if position == at && at > 0 {
            score += 5;
        }
        at = position + 1;
    }

    // The name that starts with the search is the best one.
    if name.starts_with(query) {
        score += 100;
    }
    // The short names are the better ones.
    score += 50_usize.saturating_sub(name.len().min(50));

    Some(score)
}

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
    /// The text that narrows the list of the platforms.
    search: String,
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
            search: String::new(),
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
    ///
    /// The letters go into the search, so the list narrows down while the
    /// user types. The arrows and the tab move the selection.
    fn handle_platforms_screen(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                self.leave_the_platform_list();
                None
            }
            KeyCode::Enter => {
                let Some(platform) = self
                    .visible_platforms()
                    .get(self.platform_selected)
                    .cloned()
                else {
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
            KeyCode::Backspace => {
                self.search.pop();
                self.platform_selected = 0;
                None
            }
            KeyCode::Up => {
                self.move_platform_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Tab => {
                self.move_platform_selection(1);
                None
            }
            KeyCode::Char('q') if self.search.is_empty() => {
                self.screen = Screen::Keys;
                self.platform = None;
                None
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(character);
                self.platform_selected = 0;
                None
            }
            _ => None,
        }
    }

    /// Leaves the list of the platforms.
    ///
    /// The search is cleared first, so the escape goes back one step at a
    /// time.
    fn leave_the_platform_list(&mut self) {
        if self.search.is_empty() {
            self.screen = Screen::Keys;
            self.platform = None;
        } else {
            self.search.clear();
            self.platform_selected = 0;
        }
    }

    /// The platforms that the search leaves, in the order they come.
    ///
    /// The search looks for the letters in order, so `or` finds `open_router`.
    /// A platform that starts with the text comes first, so the exact name is
    /// at the top.
    fn visible_platforms(&self) -> Vec<String> {
        let query: Vec<char> = self.search.chars().flat_map(char::to_lowercase).collect();
        if query.is_empty() {
            return self.platforms.clone();
        }

        let mut matches: Vec<Match> = self
            .platforms
            .iter()
            .filter_map(|platform| {
                fuzzy_match(platform, &query).map(|score| Match {
                    score,
                    platform: platform.clone(),
                })
            })
            .collect();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.platform.cmp(&right.platform))
        });
        matches.into_iter().map(|found| found.platform).collect()
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
        let count = self.visible_platforms().len();
        if count == 0 {
            self.platform_selected = 0;
            return;
        }
        let last = count - 1;
        self.platform_selected = self.platform_selected.saturating_add_signed(step).min(last);
    }

    /// Removes the token that is selected.
    fn remove_selected(&mut self) {
        let Some(key) = self.store.keys.get(self.selected) else {
            return;
        };
        let platform = key.provider.clone();
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
            provider: platform,
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
            .position(|key| key.provider == self.selected_platform())
            .unwrap_or(0);
        self.message = Some(String::from("Token saved"));
        self.message_is_error = false;
    }

    /// The platform of the selected token.
    fn selected_platform(&self) -> String {
        self.store
            .keys
            .get(self.selected)
            .map_or_else(String::new, |key| key.provider.clone())
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
            Screen::Platforms => " Add a token: pick the provider ",
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
                        format!("{:<18}", key.provider),
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
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(area);
        let (Some(search_area), Some(list_area)) = (chunks.first(), chunks.get(1)) else {
            return;
        };

        let search = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(self.search.clone()),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Search (Esc to clear) "),
        );
        frame.render_widget(search, *search_area);

        let platforms = self.visible_platforms();
        let items: Vec<ListItem<'_>> = platforms
            .iter()
            .map(|platform| ListItem::new(Line::from(Span::raw(platform.clone()))))
            .collect();

        let title = if platforms.is_empty() {
            " Providers: nothing found ".to_owned()
        } else {
            format!(" Providers ({}) ", platforms.len())
        };
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_symbol("> ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut state = ListState::default();
        state.select(Some(self.platform_selected));
        frame.render_stateful_widget(list, *list_area, &mut state);
    }

    /// Draws the field of the token.
    fn draw_token(&self, frame: &mut Frame<'_>, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .split(area);
        let (Some(token_area), Some(endpoint_area), Some(buttons_area)) =
            (chunks.first(), chunks.get(1), chunks.get(2))
        else {
            return;
        };

        let platform = self.platform.as_deref().unwrap_or("");
        let token_style = if self.field == Field::Token {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let field = Paragraph::new(self.input.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Token for {platform} ")),
            )
            .style(token_style);
        frame.render_widget(field, *token_area);

        let endpoint_style = if self.field == Field::Endpoint {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let endpoint = Paragraph::new(self.endpoint.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Address of a local server, without the path (Tab, optional) "),
            )
            .style(endpoint_style);
        frame.render_widget(endpoint, *endpoint_area);

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
            Screen::Platforms => " type: search  up/down: move  Enter: pick  Esc: back ",
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
                provider: "openai".to_owned(),
                token: Zeroizing::new("sk-secret-value".to_owned()),
                endpoint: None,
            }],
        };
        KeyManager::new(
            store,
            vec![
                "openai".to_owned(),
                "anthropic".to_owned(),
                "open_router".to_owned(),
            ],
        )
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
    fn the_search_narrows_the_list_of_the_platforms() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "rtr".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }

        let visible = screen.visible_platforms();
        assert!(visible.iter().any(|p| p == "open_router"), "{visible:?}");
    }

    #[test]
    fn the_search_finds_the_platforms_by_any_letters() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "or".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }

        let visible = screen.visible_platforms();
        assert!(visible.iter().any(|p| p == "open_router"), "{visible:?}");
    }

    #[test]
    fn the_search_that_finds_nothing_leaves_an_empty_list() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "zzz".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }

        assert!(
            screen.visible_platforms().is_empty(),
            "the list is not empty"
        );
    }

    #[test]
    fn the_backspace_takes_a_letter_back_from_the_search() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "or".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        screen.handle_key(key(KeyCode::Backspace));

        let visible = screen.visible_platforms();
        assert!(visible.iter().any(|p| p == "open_router"), "{visible:?}");
    }

    #[test]
    fn the_escape_clears_the_search_first() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "or".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        screen.handle_key(key(KeyCode::Esc));

        assert_eq!(screen.screen, super::Screen::Platforms);
        assert_eq!(screen.search, "", "the search is not empty");
    }

    #[test]
    fn the_search_finds_the_platform_that_starts_with_it_first() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "opena".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }

        let visible = screen.visible_platforms();
        assert_eq!(
            visible.first().map(String::as_str),
            Some("openai"),
            "{visible:?}"
        );
    }

    #[test]
    fn the_screen_shows_the_search_and_the_found_platforms() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "or".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        let text = screen_text(&screen);

        assert!(text.contains("Search"), "{text}");
        assert!(text.contains("open_router"), "{text}");
        assert!(!text.contains("cohere"), "{text}");
    }

    #[test]
    fn the_screen_shows_how_many_platforms_are_left() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        for character in "or".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        let text = screen_text(&screen);

        assert!(text.contains("Providers (1)"), "{text}");
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

    /// The screen that the manager draws, as plain text.
    fn screen_text(manager: &KeyManager) -> String {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let backend = TestBackend::new(80, 20);
        let Some(mut terminal) = Terminal::new(backend).ok() else {
            return String::new();
        };
        if terminal.draw(|frame| manager.draw(frame)).is_err() {
            return String::new();
        }
        let buffer = terminal.backend().buffer().clone();
        let symbols: Vec<&str> = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        symbols
            .chunks(80)
            .map(|row| row.iter().copied().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_screen_shows_the_tokens_with_the_middle_hidden() {
        let text = screen_text(&manager());

        assert!(text.contains("openai"), "{text}");
        assert!(text.contains("sk-s"), "{text}");
        assert!(!text.contains("sk-secret-value"), "{text}");
    }

    #[test]
    fn the_screen_says_when_there_is_no_token() {
        let empty = KeyManager::new(KeyStore::default(), vec!["openai".to_owned()]);
        let text = screen_text(&empty);

        assert!(text.contains("No tokens yet"), "{text}");
    }

    #[test]
    fn the_screen_shows_the_platforms() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        let text = screen_text(&screen);

        assert!(text.contains("openai"), "{text}");
        assert!(text.contains("anthropic"), "{text}");
    }

    #[test]
    fn the_screen_shows_the_field_of_the_token() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        let text = screen_text(&screen);

        assert!(text.contains("Token for openai"), "{text}");
        assert!(text.contains("Save"), "{text}");
        assert!(text.contains("Cancel"), "{text}");
    }

    #[test]
    fn the_screen_shows_the_field_of_the_address() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        let text = screen_text(&screen);

        assert!(text.contains("Address of a local server"), "{text}");
    }

    #[test]
    fn the_screen_shows_what_was_typed_in_the_field_of_the_address() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        screen.handle_key(key(KeyCode::Tab));
        for character in "127.0.0.1:1234".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        let text = screen_text(&screen);

        assert!(text.contains("127.0.0.1:1234"), "{text}");
    }

    #[test]
    fn the_token_typed_in_the_field_is_the_one_that_is_saved() {
        let mut screen = manager();
        screen.handle_key(key(KeyCode::Char('a')));
        screen.handle_key(key(KeyCode::Enter));
        for character in "sk-secret-value".chars() {
            screen.handle_key(key(KeyCode::Char(character)));
        }
        screen.handle_key(key(KeyCode::Enter));

        let key = screen
            .store()
            .keys
            .iter()
            .find(|item| item.provider == "openai");
        assert_eq!(
            key.map(|item| item.token.as_str()),
            Some("sk-secret-value"),
            "the token was not saved"
        );
    }

    #[test]
    fn the_selection_of_the_token_moves() {
        let mut screen = manager();
        screen.store.upsert(ApiKey {
            provider: "anthropic".to_owned(),
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
