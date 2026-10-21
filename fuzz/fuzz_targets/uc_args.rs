#![no_main]
use libfuzzer_sys::fuzz_target;
use q::uc::build_args;

fuzz_target!(|data: &str| {
    let args = build_args(data);

    // The program always clears the text, so the clear flag comes first.
    assert_eq!(args.first().map(String::as_str), Some("-c"), "no clear flag");

    // The clear flag appears once, because the flag of the user is dropped.
    assert_eq!(
        args.iter().filter(|arg| *arg == "-c").count(),
        1,
        "the clear flag appears more than once"
    );

    // The show flag would change the text into a list of problems, so it never
    // reaches the program.
    for arg in &args {
        assert_ne!(arg, "-s", "show flag reached the program");
        assert_ne!(arg, "--show", "show flag reached the program");
    }

    // The flags that print something else than the text never reach the
    // program, because they would replace the text of the model.
    for arg in &args {
        for dropped in ["-h", "--help", "-v", "--version"] {
            assert_ne!(arg, dropped, "output flag reached the program");
        }
    }

    // The arguments keep the order of the input, and the separator ends them.
    let mut input = data.split_whitespace();
    for arg in args.iter().skip(1) {
        loop {
            let Some(token) = input.next() else {
                panic!("argument {arg} has no source in the input");
            };
            if token == "--" {
                panic!("argument {arg} comes after the separator");
            }
            if token == arg {
                break;
            }
        }
    }
});
