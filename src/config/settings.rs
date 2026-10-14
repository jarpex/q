use anyhow::{Context, Result};
use std::path::Path;

/// User-configurable defaults loaded from `q.conf`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The default Gemini model, preserving its configured casing.
    pub model: Option<String>,
    /// Whether streaming is disabled by default.
    pub no_stream: Option<bool>,
}

/// Parses the contents of a q configuration file.
///
/// Option names and boolean values are case-insensitive. Model values are
/// preserved exactly as written.
///
/// # Errors
///
/// Returns an error for unknown options, missing values, invalid boolean
/// values, or unterminated quoted values.
pub fn parse_settings(input: &str) -> Result<Settings> {
    let tokens = tokenize(input)?;
    let mut settings = Settings::default();
    let mut index = 0;

    while index < tokens.len() {
        let raw_option = tokens
            .get(index)
            .context("missing configuration option")?
            .clone();
        index += 1;
        let normalized_option = raw_option.to_ascii_lowercase();
        let option = normalized_option.trim_start_matches('-');

        match option {
            "model" | "m" => {
                index = skip_equals(&tokens, index);
                let value = tokens.get(index).context("model requires a value")?;
                if is_option(value) {
                    anyhow::bail!("model requires a value");
                }
                settings.model = Some(value.clone());
                index += 1;
            }
            "no-stream" | "no_stream" | "nostream" => {
                let (value, next_index) = parse_bool_value(&tokens, index)?;
                settings.no_stream = Some(value);
                index = next_index;
            }
            "" => anyhow::bail!("empty configuration option"),
            _ => anyhow::bail!("unknown configuration option: {raw_option}"),
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

    for character in input.chars() {
        if let Some(expected) = quote {
            if character == expected {
                quote = None;
            } else {
                current.push(character);
            }
        } else {
            match character {
                '\'' | '"' => quote = Some(character),
                '=' | ';' => {
                    push_token(&mut tokens, &mut current);
                    tokens.push(character.to_string());
                }
                whitespace if whitespace.is_whitespace() => {
                    push_token(&mut tokens, &mut current);
                }
                _ => current.push(character),
            }
        }
    }

    if quote.is_some() {
        anyhow::bail!("unterminated quoted value");
    }
    push_token(&mut tokens, &mut current);
    Ok(tokens
        .into_iter()
        .filter(|token| token != "=" && token != ";")
        .collect())
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn skip_equals(tokens: &[String], index: usize) -> usize {
    if tokens.get(index).is_some_and(|token| token == "=") {
        index + 1
    } else {
        index
    }
}

#[allow(clippy::type_complexity)]
fn parse_bool_value(tokens: &[String], index: usize) -> Result<(bool, usize)> {
    let Some(value) = tokens.get(index) else {
        return Ok((true, index));
    };
    if value == "=" {
        return parse_bool_value(tokens, index + 1);
    }
    if is_option(value) {
        return Ok((true, index));
    }

    let normalized = value.to_ascii_lowercase();
    let parsed = match normalized.as_str() {
        "true" | "enabled" | "enable" | "yes" | "1" => true,
        "false" | "disabled" | "disable" | "no" | "0" => false,
        _ => anyhow::bail!("invalid no-stream value: {value}"),
    };
    Ok((parsed, index + 1))
}

fn is_option(token: &str) -> bool {
    matches!(
        token.trim_start_matches('-').to_ascii_lowercase().as_str(),
        "model" | "m" | "no-stream" | "no_stream" | "nostream"
    )
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{parse_settings, Settings};

    fn settings(input: &str) -> Settings {
        let result = parse_settings(input);
        assert!(result.is_ok());
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
}
