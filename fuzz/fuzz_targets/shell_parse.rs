#![no_main]
use libfuzzer_sys::fuzz_target;
use q::commands::shell::parse_command;

fuzz_target!(|data: &str| {
    let command = parse_command(data);

    assert!(
        !command.contains('\n'),
        "command must be one line: {command:?}"
    );

    // A fence marker must never start the command.
    assert!(!command.starts_with("```"), "fence leaked: {command:?}");

    // The command must not keep a prompt prefix.
    assert!(!command.starts_with("$ "), "prompt leaked: {command:?}");
    assert!(!command.starts_with("# "), "comment leaked: {command:?}");

    // The command is taken from the response, so it must be a part of it.
    // Joining a continued line drops the backslash and the line break, so
    // they go away from both sides before the parts are compared.
    let source = strip_continuations(data);
    let command = strip_continuations(&command);
    assert!(
        source.contains(&command),
        "command is not a part of the response: {command:?}"
    );

    // A language identifier on the fence line is not part of the command,
    // unless a flag follows it.
    if let Some(rest) = data.strip_prefix("```bash ") {
        let code = rest.trim_start_matches(['`', ' ']);
        if !code.starts_with('-') {
            assert!(
                !command.starts_with("bash "),
                "language identifier kept: {command:?}"
            );
        }
    }
});

/// Removes the backslashes and the whitespace, the parts that a join drops.
fn strip_continuations(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '\\')
        .collect()
}
