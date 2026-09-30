#!/usr/bin/env python3
"""
Collect licenses for Python dependencies used by the project.
Creates a temporary venv, installs dependencies, extracts licenses,
and cleans up.
"""

import json
import subprocess
import sys
import venv
from pathlib import Path

PROJECT_DEPS = ["gemini-webapi", "httpx"]


def run_cmd(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    """Run command and raise on failure."""
    if "stdout" not in kwargs and "stderr" not in kwargs:
        kwargs["capture_output"] = True
    result = subprocess.run(cmd, text=True, **kwargs)
    if result.returncode != 0:
        print(f"Command failed: {' '.join(cmd)}", file=sys.stderr)
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        sys.exit(1)
    return result


def find_python() -> Path:
    """Find a suitable Python 3.11+ executable."""
    candidates = ["python3", "python"]

    if sys.platform == "win32":
        candidates.append("py")

    for minor in range(11, 16):
        candidates.append(f"python3.{minor}")

    for name in candidates:
        try:
            result = subprocess.run(
                [name, "--version"],
                capture_output=True,
                text=True,
                check=False,
                timeout=5,
            )
            version_str = result.stdout.strip() or result.stderr.strip()
            if not version_str:
                continue

            parts = version_str.split()
            if len(parts) >= 2:
                ver = parts[1].split(".")
                if len(ver) >= 2:
                    major = int(ver[0])
                    minor = int(ver[1])
                    if (major, minor) >= (3, 11):
                        print(f"  ✓ Found Python {major}.{minor} as '{name}'")
                        return Path(name)
        except (FileNotFoundError, ValueError, IndexError, subprocess.TimeoutExpired):
            continue

    print("Error: Python 3.11+ not found", file=sys.stderr)
    print("Tried candidates:", ", ".join(candidates), file=sys.stderr)
    sys.exit(1)


def extract_license_info(venv_path: Path) -> list[tuple[str, str, str, str]]:
    """
    Extract license info from installed packages.
    Returns list of (package_name, version, license_name, license_text).
    """
    pip_licenses = (
        venv_path / "bin" / "pip-licenses"
        if (venv_path / "bin").exists()
        else venv_path / "Scripts" / "pip-licenses.exe"
    )

    result = run_cmd(
        [str(pip_licenses), "--format=json", "--with-license-file"],
    )

    packages = json.loads(result.stdout)
    output = []

    for pkg in packages:
        name = pkg.get("Name", "unknown")
        version = pkg.get("Version", "?")
        license_name = pkg.get("License", "Unknown")
        license_text = pkg.get("LicenseText", "")

        if license_text:
            license_text = "\n".join(
                line.rstrip() for line in license_text.splitlines()
            ).strip()

        output.append((name, version, license_name, license_text))

    return output


def format_licenses(packages: list[tuple[str, str, str, str]]) -> str:
    """Format license information into a readable text file."""
    lines = []
    for name, version, license_name, license_text in packages:
        lines.append(name)
        lines.append(version)
        lines.append(license_name)
        lines.append("")
        if license_text:
            lines.append(license_text)
        else:
            lines.append(f"(License text not available. Declared as: {license_name})")
        lines.append("")
        lines.append("")
    return "\n".join(lines)

def main() -> None:
    output_path = Path("PYTHON_LICENSES.txt").resolve()
    python_bin = find_python()
    print(f"Using Python: {python_bin}")

    venv_root = Path(".venv-licenses")

    if venv_root.exists():
        print(f"Removing stale {venv_root}...")
        import shutil
        shutil.rmtree(venv_root, ignore_errors=True)

    print(f"Creating temporary venv at {venv_root}...")
    builder = venv.EnvBuilder(with_pip=True, clear=True)
    builder.create(venv_root)

    pip = (
        venv_root / "bin" / "pip"
        if (venv_root / "bin").exists()
        else venv_root / "Scripts" / "pip.exe"
    )

    try:
        print("Upgrading pip...")
        run_cmd([str(pip), "install", "--upgrade", "pip", "wheel"])

        print(f"Installing dependencies: {', '.join(PROJECT_DEPS)}...")
        run_cmd([str(pip), "install", *PROJECT_DEPS])

        print("Installing pip-licenses...")
        run_cmd([str(pip), "install", "pip-licenses"])

        print("Extracting licenses...")
        packages = extract_license_info(venv_root)

        print(f"Found {len(packages)} packages")
        formatted = format_licenses(packages)

        output_path.write_text(formatted, encoding="utf-8")
        print(f"Licenses written to {output_path}")

    finally:
        print(f"Cleaning up {venv_root}...")
        import shutil
        shutil.rmtree(venv_root, ignore_errors=True)
        print("Done.")


if __name__ == "__main__":
    main()