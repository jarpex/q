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
  -l, --login          Force re-authentication with Gemini
  -m, --model <MODEL>  Model to use (gemini-flash, gemini-pro, gemini-flash-lite) [default: gemini-flash]
      --no-stream      Disable streaming
  -d, --debug          Enable debug output
      --rebuild-venv   Force recreate Python virtual environment
  -h, --help           Print help
  -V, --version        Print version
```

## Third-Party Components & Licenses

This project operates as a Rust wrapper and interacts with the following external Python package:

- **gemini-webapi** (AGPL-3.0 License)  
  Source code: https://github.com/HanaokaYuzu/Gemini-API

More info:

- **[THIRD_PARTY_LICENSES.html](./THIRD_PARTY_LICENSES.html)** — Licenses for the rust dependencies used in this project
- **[PYTHON_LICENSES.txt](./PYTHON_LICENSES.txt)** — Licenses for the Python dependencies used in this project
