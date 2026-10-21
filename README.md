# q

CLI tool for quick, one-shot Gemini queries with no API key required

## Demo

query mode:

<img src="https://raw.githubusercontent.com/jarpex/q/refs/heads/main/assets/q-query.gif" alt="Query mode demo" width="700" />

command mode:

<img src="https://raw.githubusercontent.com/jarpex/q/refs/heads/main/assets/q-command.gif" alt="Command mode demo" width="700" />

```
Usage: q [OPTIONS] [QUERY]...

Arguments:
  [QUERY]...  Your query

Options:
  -c, --command-mode   Command mode: print only the requested command and copy to clipboard
      --reauth         Force re-authentication with Gemini
  -l, --list-models    List the models available for the authenticated account
  -u, --usage-limits  Show account usage limits
  -i, --import-cookies Manually import cookies from browser (recommended for Windows)
  -m, --model <MODEL>  Model to use [default: gemini-auto]
      --no-stream      Disable streaming
  -d, --debug          Enable debug output
      --rebuild-venv   Force recreate Python virtual environment
      --auth-timeout <SECONDS>  Seconds to wait for the login window [default: 300]
      --uc <ARGS>        Run the output through the uc program with the given arguments
      --uc-path <PATH>   Path of the uc program to run
  -h, --help           Print help
  -V, --version        Print version
```

Optional configuration is loaded from `q.conf` in the platform configuration
directory (`%AppData%\q` on Windows, `~/.config/q` on UNIX-like, `~/Library/Application Support/q` on macOS).
Currently, the supported configuration options are `model`, `no-stream`,
`auth-timeout`, `uc` and `uc-path`.

For example:

```text
model="gemini-auto"
no-stream=true
auth-timeout=600
uc="-c -e cp1251"
uc-path="/opt/homebrew/bin/uc"
```

## Authentication Timeout

The login window waits 300 seconds for the Google sign-in to finish. Raise the
limit for slow links or extra verification steps:

```bash
q --reauth --auth-timeout 600
```

Set `auth-timeout` in `q.conf` to make it the default. The command-line value
wins over the configuration file. The maximum is 86400 seconds.

## Unicode Cleanup

The [uc program](https://github.com/jarpex/uc) removes the typographic garbage
that language models leave in the text. Run the output through it:

```bash
q --uc "-c -e cp1251" "your question"
```

The program always runs with the `-c` flag, which clears the text, and never
with `-s`, which only shows the problems. A `-c` or `-s` flag in the arguments
is ignored, so the mode of the program never depends on them. The other
arguments go to the program as they are.

The text is cleaned before it is shown and before it goes to the clipboard, so
both hold the same text. The `-e` flag picks the characters that survive, so
`-e cp1251` keeps Cyrillic text and `-e ascii` does not.

The program is looked up in `PATH` under the name `uc`. Give a path of your own
to run a different build:

```bash
q --uc "-c -e cp1251" --uc-path "/opt/homebrew/bin/uc" "your question"
```

The program at that path runs with the rights of the user, so point it only at
a build you trust.

Set `uc` and `uc-path` in `q.conf` to make them the default. The command-line
value wins over the configuration file.

A missing program does not stop the run. The text is shown as it is, and a
warning above the frame says that the program was not found.

## Environment Variables

| Variable | Effect |
| --- | --- |
| `Q_CONFIG_DIR` | Uses the given directory for the cookies, the metadata, `q.conf`, and the Python virtual environment. |
| `Q_DISABLE_CLIPBOARD` | Turns off the copy to the clipboard, so a run leaves the clipboard of the user alone. |

## Windows Authentication (DBSC)

On Windows, Google's **Device Bound Session Credentials (DBSC)** prevent reliable automatic cookie extraction via WebView2.

**You must use manual cookie import on Windows:**

```bash
q --import-cookies
# or
q -i
```

This will guide you through:
1. Opening Firefox in incognito mode
2. Logging into https://gemini.google.com/app
3. Opening DevTools (F12) -> Cookies -> https://gemini.google.com
4. Copying `__Secure-1PSID` and `__Secure-1PSIDTS` values
5. **CRITICAL: Closing the Gemini tab immediately** (cookies expire ~15 min if left open)

Firefox is recommended (avoids DBSC). A separate Google account is recommended for keep-alive services.

## Third-Party Components & Licenses

This project operates as a Rust wrapper and interacts with the following external Python package:

- **gemini-webapi** (AGPL-3.0 License)  
  Source code: https://github.com/HanaokaYuzu/Gemini-API

More info:

- **[THIRD_PARTY_LICENSES.html](./THIRD_PARTY_LICENSES.html)**: licenses for the Rust dependencies used in this project
- **[PYTHON_LICENSES.txt](./PYTHON_LICENSES.txt)**: licenses for the Python dependencies used in this project
