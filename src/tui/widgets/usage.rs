//! Account usage limits and model quota report formatting.

use serde_json::Value;

use super::get_max_content_width;

/// Prints account usage windows and model quotas in a compact terminal report.
#[allow(clippy::print_stdout)]
pub fn print_usage_limits(usage_info: &Value, quotas: &Value) {
    let width = get_max_content_width();
    let entries = quota_entries(quotas);
    let report = Report::new(width, &entries);

    println!();
    for line in usage_limit_lines(usage_info, &report) {
        println!("{line}");
    }

    if !entries.is_empty() {
        println!();
        for line in quota_limit_lines(&entries, &report) {
            println!("{line}");
        }
    }
    println!();
}

/// The width of the column of the labels.
///
/// The labels of the windows are shorter than this, so the labels of the
/// models have room.
const LABEL_WIDTH: usize = 15;

/// The geometry of the report.
///
/// Every block of the report uses it, so the bars of one block stand over the
/// bars of the other.
struct Report {
    /// The width of the column of the labels.
    label_width: usize,
    /// The width of the bar of the percentage.
    bar_width: usize,
}

impl Report {
    /// Builds the geometry that fits the given width and the quotas.
    fn new(width: usize, entries: &[QuotaEntry<'_>]) -> Self {
        let names_width = entries
            .iter()
            .map(|(name, _)| name.chars().count())
            .max()
            .unwrap_or(0)
            .min(MAX_QUOTA_LABEL_WIDTH);
        let label_width = LABEL_WIDTH.max(names_width);

        let credits_width = entries
            .iter()
            .map(|(_, bucket)| quota_credits(bucket).chars().count())
            .max()
            .unwrap_or(0);
        // The quotas of the models add a column of their own at the end.
        let extra = if credits_width == 0 {
            0
        } else {
            2 + credits_width
        };
        let bar_width = width
            .saturating_sub(2 + label_width + 1 + 4 + extra)
            .clamp(3, 24);

        Self {
            label_width,
            bar_width,
        }
    }
}

fn usage_limit_lines(usage_info: &Value, report: &Report) -> Vec<String> {
    let windows = [
        ("5-hour limit", usage_info.get("current_5h")),
        ("Weekly limit", usage_info.get("weekly")),
    ];
    std::iter::once("Usage limits:".to_owned())
        .chain(
            windows
                .iter()
                .map(|(label, value)| format_usage_line(label, *value, report)),
        )
        .collect()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn format_usage_line(label: &str, window: Option<&Value>, report: &Report) -> String {
    let Some(window_value) = window else {
        return format!("  {label:<width$}Unavailable", width = report.label_width);
    };
    let percentage = window_percentage(window_value);
    let bar = usage_bar(percentage, report.bar_width);
    format!(
        "  {label:<width$}{bar} {percentage:>3.0}%",
        width = report.label_width
    )
}

/// Reads the used percentage of a window.
fn window_percentage(window: &Value) -> f64 {
    window
        .get("usage_percentage")
        .and_then(Value::as_f64)
        .or_else(|| {
            window
                .get("usage_level")
                .and_then(Value::as_f64)
                .map(|level| level * 100.0)
        })
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
}

/// Builds the bar of a percentage.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn usage_bar(percentage: f64, bar_width: usize) -> String {
    let filled = ((bar_width as f64) * percentage / 100.0).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled.min(bar_width)),
        "░".repeat(bar_width.saturating_sub(filled))
    )
}

/// The largest width of a model name in the quota lines.
const MAX_QUOTA_LABEL_WIDTH: usize = 24;

/// The keys of the quota map that hold something else than a model bucket.
const QUOTA_RESERVED_KEYS: [&str; 2] = ["usage_info", "extra"];

/// A model bucket of the quota map, with the name of the model.
type QuotaEntry<'a> = (String, &'a Value);

/// Builds the lines of the model quotas.
fn quota_limit_lines(entries: &[QuotaEntry<'_>], report: &Report) -> Vec<String> {
    let mut lines = vec!["Model quotas:".to_owned()];
    for (name, bucket) in entries {
        let percentage = window_percentage(bucket);
        let bar = usage_bar(percentage, report.bar_width);
        let credits = quota_credits(bucket);
        lines.push(format!(
            "  {:<width$}{bar} {percentage:>3.0}%  {credits}",
            shorten(name, report.label_width),
            width = report.label_width
        ));
    }
    lines
}

/// Collects the buckets of the models out of the quota map.
///
/// The quota map holds one bucket per model. It also holds a copy of the usage
/// info and the caps of the extra features, which have another shape and are
/// left out.
fn quota_entries(quotas: &Value) -> Vec<QuotaEntry<'_>> {
    let Some(buckets) = quotas.as_object() else {
        return Vec::new();
    };

    let mut entries: Vec<QuotaEntry<'_>> = buckets
        .iter()
        .filter(|(key, _)| !QUOTA_RESERVED_KEYS.contains(&key.as_str()))
        .filter(|(_, bucket)| bucket.is_object())
        .map(|(key, bucket)| (model_name(bucket, key), bucket))
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

/// The name of a model, without the id of its bucket.
fn model_name(bucket: &Value, key: &str) -> String {
    let label = bucket.get("label").and_then(Value::as_str).unwrap_or(key);
    label.split(" [").next().unwrap_or(label).to_owned()
}

/// The credits left in a quota bucket.
fn quota_credits(bucket: &Value) -> String {
    let total = bucket.get("total").and_then(Value::as_i64);
    let left = bucket.get("remaining").and_then(Value::as_i64);
    match (left, total) {
        (Some(0), Some(0)) => "Unlimited".to_owned(),
        (Some(remaining), Some(credits)) => format!("{remaining}/{credits}"),
        _ => "Unavailable".to_owned(),
    }
}

/// Shortens a name that does not fit the column.
fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(width.saturating_sub(1)).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::{
        format_usage_line, model_name, quota_credits, quota_entries, quota_limit_lines, shorten,
        usage_limit_lines, Report,
    };
    use serde_json::json;

    /// The geometry of a report without any quota of the models.
    fn report(width: usize) -> Report {
        Report::new(width, &[])
    }

    #[test]
    fn reports_a_missing_window_as_unavailable() {
        let line = format_usage_line("5-hour limit", None, &report(80));
        assert!(line.contains("Unavailable"), "{line}");
    }

    #[test]
    fn reads_the_percentage_when_it_is_given() {
        let value = json!({"usage_percentage": 42.0});
        let line = format_usage_line("5-hour limit", Some(&value), &report(80));
        assert!(line.contains("42%"), "{line}");
    }

    #[test]
    fn derives_the_percentage_from_the_level() {
        let value = json!({"usage_level": 0.25});
        let line = format_usage_line("5-hour limit", Some(&value), &report(80));
        assert!(line.contains("25%"), "{line}");
    }

    #[test]
    fn keeps_the_percentage_within_bounds() {
        let value = json!({"usage_percentage": 150.0});
        let line = format_usage_line("5-hour limit", Some(&value), &report(80));
        assert!(line.contains("100%"), "{line}");
    }

    #[test]
    fn lists_both_windows() {
        let usage = json!({
        "current_5h": {"usage_percentage": 10.0},
        "weekly": {"usage_percentage": 20.0}
        });
        let lines = usage_limit_lines(&usage, &report(80));
        assert_eq!(lines.len(), 3);
        assert!(
            lines.first().is_some_and(|l| l.contains("Usage limits")),
            "{lines:?}"
        );
        assert!(
            lines.get(1).is_some_and(|l| l.contains("5-hour limit")),
            "{lines:?}"
        );
        assert!(
            lines.get(2).is_some_and(|l| l.contains("Weekly limit")),
            "{lines:?}"
        );
    }

    #[test]
    fn marks_a_missing_window_as_unavailable() {
        let usage = json!({"current_5h": {"usage_percentage": 10.0}});
        let lines = usage_limit_lines(&usage, &report(80));
        assert_eq!(lines.len(), 3);
        assert!(lines.get(1).is_some_and(|l| l.contains("10%")), "{lines:?}");
        assert!(
            lines.get(2).is_some_and(|l| l.contains("Unavailable")),
            "{lines:?}"
        );
    }

    #[test]
    fn the_bars_of_the_two_blocks_stand_over_each_other() {
        let usage = json!({
            "current_5h": {"usage_percentage": 4.0},
            "weekly": {"usage_percentage": 4.0}
        });
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Flash [None-11]"
            },
            "None-4": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Pro [None-4]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let limits = usage_limit_lines(&usage, &report);
        let models = quota_limit_lines(&entries, &report);

        // The bar of every line starts at the same column.
        let at = |line: &str| line.find('█').or_else(|| line.find('░'));
        let first = at(limits.get(1).map(String::as_str).unwrap_or_default());
        for line in limits.iter().skip(1).chain(models.iter().skip(1)) {
            assert_eq!(at(line), first, "the bars do not line up: {line:?}");
        }
    }

    #[test]
    fn the_bars_of_the_two_blocks_have_the_same_width() {
        let usage = json!({"current_5h": {"usage_percentage": 4.0}});
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Flash [None-11]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let limits = usage_limit_lines(&usage, &report);
        let models = quota_limit_lines(&entries, &report);

        let width_of = |line: &str| -> usize {
            line.chars()
                .filter(|character| *character == '█' || *character == '░')
                .count()
        };
        let limit = limits.get(1).map(String::as_str).unwrap_or_default();
        let model = models.get(1).map(String::as_str).unwrap_or_default();
        assert_eq!(width_of(limit), width_of(model), "{limit:?} and {model:?}");
    }

    #[test]
    fn a_long_model_name_is_shortened_to_the_column() {
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "A very very very long model name [None-11]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let models = quota_limit_lines(&entries, &report);

        // The name is shortened to the width of the column.
        let line = models.get(1).map(String::as_str).unwrap_or_default();
        assert!(line.contains('…'), "{line}");
    }

    #[test]
    fn a_model_name_that_fits_stays_whole() {
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Flash [None-11]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let models = quota_limit_lines(&entries, &report);

        let line = models.get(1).map(String::as_str).unwrap_or_default();
        assert!(line.contains("Gemini Flash"), "{line}");
        assert!(!line.contains('…'), "{line}");
    }

    #[test]
    fn lists_the_model_quotas() {
        let quotas = json!({
            "None-11": {
                "usage_percentage": 15,
                "total": 600,
                "remaining": 509,
                "label": "Gemini Flash [None-11]"
            },
            "None-4": {
                "usage_percentage": 15,
                "total": 600,
                "remaining": 509,
                "label": "Gemini Pro [None-4]"
            }
        });

        let lines = quota_limit_lines(&quota_entries(&quotas), &report(80));
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines.first().is_some_and(|l| l.contains("Model quotas")),
            "{lines:?}"
        );
        assert!(
            lines.get(1).is_some_and(|l| l.contains("Gemini Flash")),
            "{lines:?}"
        );
        assert!(
            lines.get(1).is_some_and(|l| l.contains("509/600")),
            "{lines:?}"
        );
        assert!(lines.get(1).is_some_and(|l| l.contains("15%")), "{lines:?}");
        assert!(
            lines.get(2).is_some_and(|l| l.contains("Gemini Pro")),
            "{lines:?}"
        );
    }

    #[test]
    fn leaves_out_the_reserved_keys_of_the_quotas() {
        let quotas = json!({
            "usage_info": {"current_5h": {"usage_percentage": 15}},
            "extra": {"default": {"is_blocked": false}},
            "None-11": {"usage_percentage": 15, "total": 600, "remaining": 509}
        });

        let lines = quota_limit_lines(&quota_entries(&quotas), &report(80));
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines.get(1).is_some_and(|l| l.contains("None-11")),
            "{lines:?}"
        );
    }

    #[test]
    fn shows_no_quota_lines_without_buckets() {
        assert_eq!(quota_entries(&json!({})).len(), 0);
        assert_eq!(quota_entries(&json!({"usage_info": {}})).len(), 0);
        assert_eq!(quota_entries(&json!({"extra": {}})).len(), 0);
    }

    #[test]
    fn reads_the_credits_of_a_bucket() {
        let bucket = json!({"total": 600, "remaining": 509});
        assert_eq!(quota_credits(&bucket), "509/600");

        let empty = json!({"total": 0, "remaining": 0});
        assert_eq!(quota_credits(&empty), "Unlimited");

        let missing = json!({});
        assert_eq!(quota_credits(&missing), "Unavailable");
    }

    #[test]
    fn drops_the_id_of_a_bucket_from_the_name() {
        let bucket = json!({"label": "Gemini Flash [None-11]"});
        assert_eq!(model_name(&bucket, "None-11"), "Gemini Flash");

        let bare = json!({});
        assert_eq!(model_name(&bare, "None-4"), "None-4");
    }

    #[test]
    fn shortens_a_name_that_does_not_fit() {
        assert_eq!(shorten("Gemini Flash", 20), "Gemini Flash");
        assert_eq!(shorten("Gemini Flash Thinking", 10), "Gemini Fl…");
        assert_eq!(shorten("Gemini Flash Thinking", 10).chars().count(), 10);
    }
}
