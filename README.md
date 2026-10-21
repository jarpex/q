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
  -h, --help           Print help
  -V, --version        Print version
```

Optional configuration is loaded from `q.conf` in the platform configuration
directory (`%AppData%\q` on Windows, `~/.config/q` on UNIX-like, `~/Library/Application Support/q` on macOS).
Currently, the only supported configuration options are `model`, `no-stream` and `auth-timeout`.

For example:

```text
model="gemini-auto"
no-stream=true
auth-timeout=600
```

## Authentication Timeout

The login window waits 300 seconds for the Google sign-in to finish. Raise the
limit for slow links or extra verification steps:

```bash
q --reauth --auth-timeout 600
```

Set `auth-timeout` in `q.conf` to make it the default. The command-line value
wins over the configuration file. The maximum is 86400 seconds.

## Environment Variables

| Variable | Effect |
| --- | --- |
| `Q_CONFIG_DIR` | Uses the given directory for the cookies, the metadata, `q.conf`, and the Python virtual environment. |
| `Q_DISABLE_CLIPBOARD` | Turns off the copy to the clipboard, so a run leaves the clipboard of the user alone. |

## Windows Authentication (DBSC)

On Windows, Google's **Device Bound Session Credentials (DBSC)** prevent reliable automatic cookie extraction via WebView2.

### Automatic login through a portable Firefox

Google refuses to sign in to a browser that is driven by automation: it answers "This browser or
app may not be secure" as soon as the identifier is submitted, no matter which engine drives it
and no matter whether `navigator.webdriver` is masked. An automated browser therefore cannot be
used for the sign-in.

A real browser that you drive yourself is what Google allows, and Firefox qualifies twice over:
it does not bind the session cookies to the device the way the Chromium family does, so the
cookies it stores stay usable outside the browser, and on Windows it keeps the cookie values in
plain text in its profile, so they can be read without a debugger.

The `firefox-auth` feature uses that path:

```bash
cargo build --release --features firefox-auth
```

`q --reauth` then does the following, without asking you to install anything:

1. Downloads the official installer of the pinned Firefox release and checks its SHA-256 against
   the pinned digest. The installer is a 7-Zip archive, which is unpacked in process, so neither
   7-Zip nor the Windows installer ever runs on your machine.
2. Keeps the unpacked browser in the configuration directory of the tool, so later logins reuse it.
3. Starts the browser under a name of its own, in a profile of its own. Your own Firefox is never
   started, never read and never closed.
4. Waits for you to sign in, then closes that browser right away, because Google rotates the
   session while a Gemini tab stays open.
5. Reads the final cookies from the closed profile and checks them against Gemini with a real
   request. Only a session that Gemini accepts is saved.
6. Removes the profile and the copy of the browser, whether the sign-in succeeded or not, and
   clears the profiles of the logins that were interrupted. The session of the sign-in does not
   outlive the sign-in.

#### Why not a private window?

A private window keeps its cookies in memory. It writes neither the cookie database nor the
session store to the disk, so the session of the sign-in cannot be read back from the profile.
The remote agent of the browser can read them, but it sets `navigator.webdriver` to `true`.
That is what makes Google refuse the sign-in. The tool uses a window of its own in a profile of
its own, and removes both afterwards.

### The first run takes a moment

The browser is fetched and unpacked once, and kept afterwards. The unpacking is the slow part.
The archive reader matters: the older ones need minutes for the same archive.

To try the sign-in again:

```bash
q --reauth
```

### Manual cookie import

Without the feature, Windows has no automatic login:

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
