# q

CLI tool for quick, one-shot Gemini queries with no API key required

## Install via Homebrew

```bash
brew install jarpex/formulae/q
```

The formula installs the latest release.
To install the current `main` instead:

```bash
brew install --HEAD jarpex/formulae/q
```

## Download the Release

Every release holds an archive for each platform:

| Platform | Archive |
| --- | --- |
| Linux, x86-64 | `q-linux-amd64.tar.gz` |
| Linux, ARM64 | `q-linux-arm64.tar.gz` |
| macOS, Intel | `q-macos-intel.tar.gz` |
| macOS, Apple Silicon | `q-macos-apple-silicon.tar.gz` |
| macOS, both | `q-macos-universal.tar.gz` |
| Windows, x86-64 | `q-windows-amd64.zip` |
| Windows, x86 | `q-windows-i686.zip` |
| Windows, ARM64 | `q-windows-arm64.zip` |

The archive holds the binary, the man page and the license.
The Windows archives and the man page came with the releases after 0.4.0.

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
      --provider <PROVIDER>  The provider of the models to use
      --api-management   Open the screen that manages the API keys
  -h, --help           Print help
  -V, --version        Print version
```

Optional configuration is loaded from `q.conf` in the platform configuration
directory (`%AppData%\q` on Windows, `~/.config/q` on UNIX-like, `~/Library/Application Support/q` on macOS).
Currently, the supported configuration options are `model`, `no-stream`,
`auth-timeout`, `provider`, `uc` and `uc-path`.

For example:

```text
model="gemini-auto"
no-stream=true
auth-timeout=600
uc="-c -e cp1251"
uc-path="/opt/homebrew/bin/uc"
```

## Providers

The tool talks to Google Gemini through the cookies of the account by default.
Other providers work with a token of your own, the way the API key of a
service does.

Add a token with the screen of the tokens:

```bash
q --api-management
```

The screen shows every token with its middle hidden, so a token never reaches
a screenshot. Press `a` to add one, pick the platform from the list, and type
the token. The token goes into the file of the configuration, which only your
account can read, so it never reaches the history of the shell.

Then name the provider on the command line:

```bash
q --provider openai --model gpt-4o "your question"
q --provider anthropic --model claude-sonnet-4-5 "your question"
q --provider openai -l
```

The providers come from the library that talks to them, so the list always
matches what the tool can do. The list holds OpenAI, Anthropic, Gemini, Groq,
Mistral, DeepSeek, xAI and more, next to the local ones: Ollama for a local
server, and OpenAI for a server that speaks the OpenAI protocol, such as
LM Studio, llama.cpp or vLLM.

The screen of the providers has a search field: type a few letters and the
list narrows down. The letters may be anywhere in the name, so `or` finds
`open_router`. The name of a provider can be written in any way on the
command line: `openrouter`, `open_router` and `OpenRouter` all mean the same
one.

A local server needs its address, so it is set in the screen of the tokens.
The address is the address of the server, without the path: the tool adds the
path of the platform, so the server speaks the same protocol as the platform
it stands for.

```text
http://127.0.0.1:1234
```

The models of the provider are listed with `q --provider openai -l`.

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
Firefox is used instead: the tool downloads a portable copy of a pinned Firefox release itself, so nothing has to be
installed, and the session of the sign-in is deleted from the disk as soon as it has been read.
The browser of the user is never started, never read and never closed.

The manual import stays as a fallback for every platform:

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

## Man Page

The manual page lives in `man/q.1`. Install it into the local man directory:

```bash
just install-man
```

After that the page is at hand:

```bash
man q
```

The release tarballs carry the page as well, under `man/`.

## Third-Party Components & Licenses

This project operates as a Rust wrapper and interacts with the following external Python package:

- **gemini-webapi** (AGPL-3.0 License)  
  Source code: https://github.com/HanaokaYuzu/Gemini-API

More info:

- **[THIRD_PARTY_LICENSES.html](./THIRD_PARTY_LICENSES.html)**: licenses for the Rust dependencies used in this project
- **[PYTHON_LICENSES.txt](./PYTHON_LICENSES.txt)**: licenses for the Python dependencies used in this project
