import asyncio
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
    """Strips service XML tags from a stream."""

    BLOCKS = {"ElicitationsGroup"}

    def __init__(self):
        self.state = "TEXT"
        self.buf = ""
        self.block_name = ""

    def feed(self, text):
        out = []
        for ch in text:
            if self.state == "TEXT":
                if ch == "<":
                    self.state = "TAG"
                    self.buf = "<"
                else:
                    out.append(ch)
            elif self.state == "TAG":
                self.buf += ch
                if ch == ">":
                    tag_content = self.buf[1:-1].strip()
                    if tag_content.startswith("/"):
                        self.state = "TEXT"
                    else:
                        parts = tag_content.split()
                        if not parts:
                            self.state = "TEXT"
                        else:
                            name = parts[0].rstrip("/")
                            if name in self.BLOCKS:
                                self.state = "BLOCK"
                                self.block_name = name
                            else:
                                self.state = "TEXT"
                    self.buf = ""
            elif self.state == "BLOCK":
                self.buf += ch
                if self.buf.endswith(f"</{self.block_name}>"):
                    self.state = "TEXT"
                    self.buf = ""
        return "".join(out)


async def main():
    if len(sys.argv) < 5:
        print("Usage: python -c '<script>' <psid> <psidts> <model> <mode>", file=sys.stderr)
        sys.exit(1)

    psid = sys.argv[1]
    psidts = sys.argv[2]
    model_arg = sys.argv[3]
    model = model_arg if model_arg and model_arg != "None" else None
    mode = sys.argv[4]

    query = sys.stdin.read()
    if not query:
        print("Error: empty query received", file=sys.stderr)
        sys.exit(1)

    try:
        client = GeminiClient(psid, psidts)
        await client.init(timeout=60, auto_close=False, close_delay=300, auto_refresh=True)

        kwargs = {"temporary": True}
        if model:
            kwargs["model"] = model

        cleaner = Cleaner()

        if mode == "stream" and hasattr(client, "generate_content_stream"):
            prev_text = ""
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
                    if cleaned:
                        sys.stdout.write(cleaned)
                        sys.stdout.flush()
            sys.stdout.write("\n")
            sys.stdout.flush()
        else:
            response = await client.generate_content(query, **kwargs)
            text = response.text or ""
            sys.stdout.write(cleaner.feed(text))
            sys.stdout.write("\n")
            sys.stdout.flush()
    except Exception:
        traceback.print_exc(file=sys.stderr)
        sys.exit(1)


asyncio.run(main())
