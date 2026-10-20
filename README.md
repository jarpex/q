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
  -h, --help           Print help
  -V, --version        Print version
```

Optional configuration is loaded from `q.conf` in the platform configuration
directory (`%AppData%\q` on Windows, `~/.config/q` on UNIX-like, `~/Library/Application Support/q` on macOS).
Currently, the only supported configuration options are `model` and `no-stream`.

For example:

```text
model="gemini-auto"
no-stream=true
```

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
