import asyncio
import json
import os
import re
import sys
import traceback

# Python writes text to a pipe in text mode, and on Windows that turns every
# newline into a carriage return plus a newline. The answers are read by the
# Rust side as they are, so the translation is undone here: the answer keeps
# the line breaks it has, on every platform.
for stream in (sys.stdout, sys.stderr):
    reconfigure = getattr(stream, "reconfigure", None)
    if reconfigure is not None:
        reconfigure(newline="\n")

try:
    from gemini_webapi import GeminiClient
except ImportError:
    print(
        f"Error: gemini-webapi is not installed for this Python ({sys.executable}).\n"
        f"Run the application with the venv rebuild flag to fix this.",
        file=sys.stderr,
    )
    sys.exit(2)


# TEMPORARY FIX: remove once gemini-webapi ships the fix from open PR
# HanaokaYuzu/Gemini-API#382. Version 2.1.1 deadlocks in GeminiClient.init
# when an init RPC returns non-200 (upstream issue #381): the @running(retry=2)
# wrapper around _batch_execute catches the APIError, sees _running cleared by
# close_on_error, and re-enters init(), which then waits on the lock the outer
# init() still holds. The client hangs forever instead of raising.
#
# The patch breaks the cycle the same way #382 does: while init() runs, the
# client is marked _initializing and the init RPCs run with close_on_error off,
# so the retry path is never taken mid-init and the real error surfaces.
def _apply_temp_deadlock_fix() -> None:
    original_batch = getattr(GeminiClient, "_batch_execute", None)
    raw_batch = getattr(original_batch, "__wrapped__", None)
    if original_batch is None or raw_batch is None:
        # The library changed and no longer wraps _batch_execute; nothing to do.
        return

    async def patched_batch(
        self, payloads, source_path="/app", close_on_error=True, **kwargs
    ):
        if getattr(self, "_initializing", False):
            return await raw_batch(
                self, payloads, source_path, close_on_error=False, **kwargs
            )
        return await original_batch(
            self, payloads, source_path, close_on_error=close_on_error, **kwargs
        )

    original_init = GeminiClient.init

    async def patched_init(self, *args, **kwargs):
        self._initializing = True
        try:
            return await original_init(self, *args, **kwargs)
        finally:
            self._initializing = False

    GeminiClient._batch_execute = patched_batch
    GeminiClient.init = patched_init


_apply_temp_deadlock_fix()


def _load_extra_cookies(client) -> None:
    """Put the full Google cookie set of the session onto the client.

    Google refuses a session that carries only the two session IDs, so the rest
    of the cookies travel in GEMINI_COOKIES and are set with their own domain.
    A host-only cookie of gemini.google.com must not be sent to lh3.google.com,
    so each cookie keeps the domain the browser stored it under.
    """
    raw = os.environ.get("GEMINI_COOKIES")
    if not raw or raw == "[]":
        return

    try:
        items = json.loads(raw)
    except (ValueError, TypeError):
        return

    from curl_cffi.requests import Cookies

    jar = Cookies()
    for item in items:
        name = item.get("name")
        value = item.get("value")
        domain = item.get("domain") or ".google.com"
        if name and value is not None:
            jar.set(name, value, domain=domain, secure=True)

    client.cookies = jar


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
        _load_extra_cookies(client)
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
