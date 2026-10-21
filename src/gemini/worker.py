import asyncio
import json
import os
import re
import sys
import traceback

try:
    from gemini_webapi import GeminiClient
except ImportError:
    print(
        f"Error: gemini-webapi is not installed for this Python ({sys.executable}).\n"
        f"Run the application with the venv rebuild flag to fix this.",
        file=sys.stderr,
    )
    sys.exit(2)


class Cleaner:
    """Strips known service XML tags from a stream.

    Only removes explicitly listed tags (e.g., <ElicitationsGroup>...</ElicitationsGroup>).
    All other content stays unchanged, including legitimate '<' and '>' characters.
    """

    BLOCK_TAGS = {"ElicitationsGroup"}

    # Matches <Tag ...> ... </Tag> with any attributes, across newlines.
    _BLOCK_RE = re.compile(
        r"</?(?:" + "|".join(re.escape(t) for t in BLOCK_TAGS) + r")(?:\s+[^>]*)?>",
        re.DOTALL,
    )

    def feed(self, text: str) -> str:
        for block_tag in self.BLOCK_TAGS:
            pattern = re.compile(
                rf"<{re.escape(block_tag)}(?:\s+[^>]*)?(?:>.*?</{re.escape(block_tag)}>|/>)",
                re.DOTALL,
            )
            text = pattern.sub("", text)

        text = self._BLOCK_RE.sub("", text)

        return text


def has_visible_text(text):
    without_ansi = re.sub(r"\x1b(?:\[[0-?]*[ -/]*[@-~\x80-\xFF]|\][^\a]*(?:\a|\x1b\\))", "", text)
    return any(ch.isprintable() and not ch.isspace() for ch in without_ansi)


async def main():
    if len(sys.argv) < 2:
        print("Usage: python -c '<script>' <mode> [model]", file=sys.stderr)
        sys.exit(1)

    # Cookies are passed through the environment, never through command-line
    # arguments: on multi-user systems argv (and /proc/<pid>/cmdline) is
    # world-readable, while environ is only visible to the process owner.
    psid = os.environ.get("GEMINI_PSID")
    psidts = os.environ.get("GEMINI_PSIDTS")
    if not psid or not psidts:
        print(
            "Error: GEMINI_PSID and GEMINI_PSIDTS environment variables are required",
            file=sys.stderr,
        )
        sys.exit(1)

    mode = sys.argv[1]
    model_arg = sys.argv[2] if len(sys.argv) > 2 else ""
    model = model_arg if model_arg and model_arg not in {"None", "gemini-auto"} else None

    try:
        client = GeminiClient(psid, psidts)
        verbose = os.environ.get("Q_GEMINI_VERBOSE") == "1"
        await client.init(
            timeout=60,
            auto_close=False,
            close_delay=300,
            auto_refresh=True,
            verbose=verbose,
        )

        if mode == "verify":
            # The account status is what the server answered to the session.
            # The model list is a static registry, so it tells nothing about
            # the session, and the client reports a successful init even when
            # the server rejected the cookies.
            if client.account_status.name != "AVAILABLE":
                print(
                    f"Gemini reports the account as {client.account_status.name}: "
                    f"{client.account_status.description}",
                    file=sys.stderr,
                )
                sys.exit(1)
            print("ok")
            return

        if mode == "list-models":
            models = client.list_models() or []
            names = [
                f"{getattr(model, 'display_name', '')}: {getattr(model, 'model_name', '')}"
                for model in models
            ]
            names = [name for name in names if not name.endswith(": ")]
            if not names:
                print("Gemini returned no available models.", file=sys.stderr)
                sys.exit(1)
            print(json.dumps(names))
            return

        if mode == "usage-limits":
            print(
                json.dumps(
                    {
                        "usage_info": client.usage_info,
                        "quotas": client.quotas,
                    }
                )
            )
            return

        query = sys.stdin.read()
        if not query:
            print("Error: empty query received", file=sys.stderr)
            sys.exit(1)

        kwargs = {"temporary": True}
        if model:
            kwargs["model"] = model

        cleaner = Cleaner()

        if mode == "stream" and hasattr(client, "generate_content_stream"):
            prev_text = ""
            has_output = False
            async for chunk in client.generate_content_stream(query, **kwargs):
                delta = getattr(chunk, "text_delta", None)
                if delta is None:
                    current_text = getattr(chunk, "text", "") or ""
                    if current_text.startswith(prev_text):
                        delta = current_text[len(prev_text):]
                    else:
                        delta = current_text
                    prev_text = current_text
                
                if delta:
                    cleaned = cleaner.feed(delta)
                    if has_visible_text(cleaned):
                        has_output = True
                    if cleaned:
                        sys.stdout.write(cleaned)
                        sys.stdout.flush()
            if not has_output:
                print("Gemini returned an empty response.", file=sys.stderr)
                sys.exit(1)
            sys.stdout.write("\n")
            sys.stdout.flush()
        else:
            response = await client.generate_content(query, **kwargs)
            text = response.text or ""
            cleaned = cleaner.feed(text)
            if not has_visible_text(cleaned):
                print("Gemini returned an empty response.", file=sys.stderr)
                sys.exit(1)
            sys.stdout.write(cleaned)
            sys.stdout.write("\n")
            sys.stdout.flush()
    except Exception:
        traceback.print_exc(file=sys.stderr)
        sys.exit(1)


asyncio.run(main())
