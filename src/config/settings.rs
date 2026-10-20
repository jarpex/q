use anyhow::{Context, Result};
use std::path::Path;

/// Default number of seconds to wait for the login window.
pub const DEFAULT_AUTH_TIMEOUT_SECS: u64 = 300;

/// Maximum number of seconds to wait for the login window.
pub const MAX_AUTH_TIMEOUT_SECS: u64 = 86_400;

/// User-configurable defaults loaded from `q.conf`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The default Gemini model, preserving its configured casing.
    pub model: Option<String>,
    /// Whether streaming is disabled by default.
    pub no_stream: Option<bool>,
    /// Authentication timeout in seconds.
    pub auth_timeout: Option<u64>,
}

/// Parses an authentication timeout in seconds.
///
/// # Errors
///
/// Returns an error if the value is not a number, is zero, or exceeds the maximum.
#[allow(clippy::result_large_err)]
pub fn parse_auth_timeout(value: &str) -> Result<u64, String> {
    let seconds = value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid auth timeout: {value}"))?;
    if seconds == 0 {
        return Err("auth timeout must be greater than zero".to_owned());
    }
    if seconds > MAX_AUTH_TIMEOUT_SECS {
        return Err(format!(
            "auth timeout must not exceed {MAX_AUTH_TIMEOUT_SECS} seconds"
        ));
    }
    Ok(seconds)
}

/// Parses the contents of a q configuration file.
///
/// Supports:
/// - `#` comments (from `#` to end of line, except inside quotes)
/// - Multi-line values in quotes (newlines are preserved inside quoted strings)
/// - Line continuation with `\` at end of line (outside and inside quotes)
/// - Escape sequences: `\\`, `\"`, `\'`, `\#`, and `\` before newline
///
/// Option names and boolean values are case-insensitive. Model values are
/// preserved exactly as written.
///
/// # Errors
///
/// Returns an error for unknown options, missing values, invalid boolean
/// values, unterminated quoted values, or trailing backslash.
pub fn parse_settings(input: &str) -> Result<Settings> {
    let tokens = tokenize(input)?;
    let mut settings = Settings::default();
    let mut index = 0;

    while index < tokens.len() {
        let Some(raw_option) = tokens.get(index) else {
            break;
        };
        let option = raw_option.trim_start_matches('-');

        if option.eq_ignore_ascii_case("model") || option.eq_ignore_ascii_case("m") {
            index += 1;
            let Some(value) = tokens.get(index) else {
                anyhow::bail!("model requires a value");
            };
            if is_option(value) {
                anyhow::bail!("model requires a value");
            }
            settings.model = Some(value.clone());
            index += 1;
        } else if option.eq_ignore_ascii_case("no-stream")
            || option.eq_ignore_ascii_case("no_stream")
            || option.eq_ignore_ascii_case("nostream")
        {
            index += 1;
            let (value, next_index) = parse_bool_value(&tokens, index)?;
            settings.no_stream = Some(value);
            index = next_index;
        } else if option.eq_ignore_ascii_case("auth-timeout")
            || option.eq_ignore_ascii_case("auth_timeout")
            || option.eq_ignore_ascii_case("authtimeout")
        {
            index += 1;
            let Some(value) = tokens.get(index) else {
                anyhow::bail!("auth-timeout requires a value");
            };
            if is_option(value) {
                anyhow::bail!("auth-timeout requires a value");
            }
            let seconds = parse_auth_timeout(value).map_err(anyhow::Error::msg)?;
            settings.auth_timeout = Some(seconds);
            index += 1;
        } else if option.is_empty() {
            anyhow::bail!("empty configuration option");
        } else {
            anyhow::bail!("unknown configuration option: {raw_option}");
        }
    }

    Ok(settings)
}

/// Returns the path to the optional q configuration file.
///
/// # Errors
///
/// Returns an error if the platform configuration directory cannot be
/// determined or created.
pub fn config_path() -> Result<std::path::PathBuf> {
    super::config_dir().map(|dir| dir.join("q.conf"))
}

/// Loads the optional q configuration file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or contains invalid settings.
pub fn load_settings<P: AsRef<Path>>(path_arg: P) -> Result<Settings> {
    let path = path_arg.as_ref();
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default())
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Cannot read configuration file: {}", path.display()));
        }
    };

    parse_settings(&content)
        .with_context(|| format!("Invalid configuration file: {}", path.display()))
}

fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut in_comment = false;
    let mut escape_next = false;
    let mut chars = input.chars().peekable();

    while let Some(character) = chars.next() {
        if escape_next {
            escape_next = false;
            match character {
                '\n' => continue,
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    continue;
                }
                _ => current.push(character),
            }
            continue;
        }

        if in_comment {
            if character == '\n' {
                in_comment = false;
                push_token(&mut tokens, &mut current);
            }
            continue;
        }

        if let Some(expected) = quote {
            match character {
                '\\' => escape_next = true,
                c if c == expected => quote = None,
                _ => current.push(character),
            }
            continue;
        }

        match character {
            '\\' => escape_next = true,
            '#' => {
                push_token(&mut tokens, &mut current);
                in_comment = true;
            }
            '\'' | '"' => quote = Some(character),
            '=' | ';' => push_token(&mut tokens, &mut current),
            whitespace if whitespace.is_whitespace() => {
                push_token(&mut tokens, &mut current);
            }
            _ => current.push(character),
        }
    }

    if quote.is_some() {
        anyhow::bail!("unterminated quoted value");
    }
    if escape_next {
        anyhow::bail!("trailing backslash at end of file");
    }
    push_token(&mut tokens, &mut current);
    Ok(tokens)
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

/// Checks if a token is a known configuration option without allocating memory.
fn is_option(token: &str) -> bool {
    let stripped = token.trim_start_matches('-');
    stripped.eq_ignore_ascii_case("model")
        || stripped.eq_ignore_ascii_case("m")
        || stripped.eq_ignore_ascii_case("no-stream")
        || stripped.eq_ignore_ascii_case("no_stream")
        || stripped.eq_ignore_ascii_case("nostream")
        || stripped.eq_ignore_ascii_case("auth-timeout")
        || stripped.eq_ignore_ascii_case("auth_timeout")
        || stripped.eq_ignore_ascii_case("authtimeout")
}

#[allow(clippy::type_complexity)]
fn parse_bool_value(tokens: &[String], index: usize) -> Result<(bool, usize)> {
    let Some(value) = tokens.get(index) else {
        return Ok((true, index));
    };

    if is_option(value) {
        return Ok((true, index));
    }

    let parsed = if value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("enabled")
        || value.eq_ignore_ascii_case("enable")
        || value.eq_ignore_ascii_case("yes")
        || value == "1"
    {
        true
    } else if value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("disabled")
        || value.eq_ignore_ascii_case("disable")
        || value.eq_ignore_ascii_case("no")
        || value == "0"
    {
        false
    } else {
        anyhow::bail!("invalid no-stream value: {value}");
    };

    Ok((parsed, index + 1))
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{parse_settings, Settings};

    fn settings(input: &str) -> Settings {
        let result = parse_settings(input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
        result.unwrap_or_default()
    }

    #[test]
    fn accepts_case_insensitive_aliases_and_separators() {
        let settings = settings("MODEL='Gemini-2.5-Pro'; --NO_STREAM = EnAbLeD");
        assert_eq!(
            settings,
            Settings {
                model: Some("Gemini-2.5-Pro".to_owned()),
                no_stream: Some(true),
                auth_timeout: None,
            }
        );
    }

    #[test]
    fn preserves_model_case_and_supports_short_alias() {
        let settings = settings("-m \"Gemini-2.5-Flash\" no-stream=0");
        assert_eq!(settings.model.as_deref(), Some("Gemini-2.5-Flash"));
        assert_eq!(settings.no_stream, Some(false));
    }

    #[test]
    fn bare_no_stream_is_true() {
        assert_eq!(settings("no-stream").no_stream, Some(true));
    }

    #[test]
    fn accepts_all_boolean_spellings() {
        for value in ["true", "TRUE", "enabled", "enable", "yes", "1"] {
            assert_eq!(
                settings(&format!("no-stream {value}")).no_stream,
                Some(true)
            );
        }
        for value in ["false", "FALSE", "disabled", "disable", "no", "0"] {
            assert_eq!(
                settings(&format!("no-stream {value}")).no_stream,
                Some(false)
            );
        }
    }

    #[test]
    fn rejects_unknown_and_malformed_options() {
        assert!(parse_settings("unknown=value").is_err());
        assert!(parse_settings("model").is_err());
        assert!(parse_settings("no-stream=maybe").is_err());
        assert!(parse_settings("model='unterminated").is_err());
    }

    #[test]
    fn parses_auth_timeout() {
        assert_eq!(settings("auth-timeout=600").auth_timeout, Some(600));
        assert_eq!(settings("auth_timeout 120").auth_timeout, Some(120));
        assert_eq!(settings("AUTHTIMEOUT=45").auth_timeout, Some(45));
    }

    #[test]
    fn rejects_invalid_auth_timeout() {
        assert!(parse_settings("auth-timeout=abc").is_err());
        assert!(parse_settings("auth-timeout=0").is_err());
        assert!(parse_settings("auth-timeout=999999").is_err());
        assert!(parse_settings("auth-timeout").is_err());
        assert!(parse_settings("auth-timeout model=x").is_err());
    }

    #[test]
    fn auth_timeout_defaults_to_none() {
        assert_eq!(settings("model=x").auth_timeout, None);
    }

    #[test]
    fn ignores_full_line_comments() {
        let s = settings("# comment\nmodel = value");
        assert_eq!(s.model.as_deref(), Some("value"));
    }

    #[test]
    fn ignores_inline_comments() {
        let s = settings("model = value # inline comment");
        assert_eq!(s.model.as_deref(), Some("value"));
    }

    #[test]
    fn hash_inside_quotes_is_not_a_comment() {
        let s = settings("model = \"gemini#2.5\"");
        assert_eq!(s.model.as_deref(), Some("gemini#2.5"));
    }

    #[test]
    fn escaped_hash_outside_quotes_is_literal() {
        let s = settings("model = gemini\\#2.5");
        assert_eq!(s.model.as_deref(), Some("gemini#2.5"));
    }

    #[test]
    fn supports_multiline_values_in_quotes() {
        let s = settings("model = \"line1\nline2\nline3\"");
        assert_eq!(s.model.as_deref(), Some("line1\nline2\nline3"));
    }

    #[test]
    fn supports_multiline_values_with_crlf() {
        let s = settings("model = \"line1\r\nline2\"");
        assert_eq!(s.model.as_deref(), Some("line1\r\nline2"));
    }

    #[test]
    fn supports_line_continuation_outside_quotes() {
        let s = settings("model = gemini-\\\n2.5-pro");
        assert_eq!(s.model.as_deref(), Some("gemini-2.5-pro"));
    }

    #[test]
    fn supports_line_continuation_with_crlf() {
        let s = settings("model = gemini-\\\r\n2.5-pro");
        assert_eq!(s.model.as_deref(), Some("gemini-2.5-pro"));
    }

    #[test]
    fn supports_line_continuation_inside_quotes() {
        let s = settings("model = \"gemini-\\\n2.5\"");
        assert_eq!(s.model.as_deref(), Some("gemini-2.5"));
    }

    #[test]
    fn supports_escaped_backslash() {
        let s = settings("model = \"gemini\\\\2.5\"");
        assert_eq!(s.model.as_deref(), Some("gemini\\2.5"));
    }

    #[test]
    fn supports_escaped_quote_inside_quotes() {
        let s = settings("model = \"gemini\\\"2.5\"");
        assert_eq!(s.model.as_deref(), Some("gemini\"2.5"));
    }

    #[test]
    fn rejects_trailing_backslash() {
        assert!(parse_settings("model = value\\").is_err());
    }

    #[test]
    fn complex_config_with_comments_and_multiline() {
        let input = "\
# Configuration for q\n\
model = \"gemini-2.5-pro\"  # use pro model\n\
\n\
# Streaming settings\n\
no-stream = true\n\
\n\
# Multi-line model name (unusual but valid)\n\
model = \"test-\n\
model\"\n";
        let s = settings(input);
        assert_eq!(s.model.as_deref(), Some("test-\nmodel"));
        assert_eq!(s.no_stream, Some(true));
    }
}
