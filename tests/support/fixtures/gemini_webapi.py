"""Stub of the gemini_webapi package for the q integration tests.

The real package talks to Google. This one records what the Rust side asked
for and answers with canned data, so the test can assert on the request.

The Rust side passes the configuration directory to the worker through
`GEMINI_COOKIE_PATH`. The stub reads the canned response and writes its log
next to it, so the test needs no extra environment variables.
"""

import json
import os
import sys


def _config_dir():
    cache = os.environ.get("GEMINI_COOKIE_PATH")
    if not cache:
        return None
    return os.path.dirname(os.path.abspath(cache))


def _log(payload):
    config_dir = _config_dir()
    if not config_dir:
        return
    with open(os.path.join(config_dir, "stub_log.jsonl"), "a", encoding="utf-8") as handle:
        handle.write(json.dumps(payload) + "\n")


def _response():
    config_dir = _config_dir()
    if not config_dir:
        return "stub response"
    path = os.path.join(config_dir, "stub_response.txt")
    if not os.path.exists(path):
        return "stub response"
    with open(path, encoding="utf-8") as handle:
        return handle.read().strip()


class _Model:
    def __init__(self, display_name, model_name):
        self.display_name = display_name
        self.model_name = model_name


class _Chunk:
    def __init__(self, text):
        self.text = text
        self.text_delta = text


class _Response:
    def __init__(self, text):
        self.text = text


class GeminiClient:
    def __init__(self, psid, psidts):
        self.psid = psid
        self.psidts = psidts
        self.usage_info = {"current_5h": {"usage_percentage": 12.5}}
        self.quotas = {"flash": {"remaining": 7, "total": 10}}

    async def init(self, **kwargs):
        _log(
            {
                "kind": "init",
                "psid": self.psid,
                "psidts": self.psidts,
                "argv": sys.argv,
            }
        )

    def list_models(self):
        _log({"kind": "list_models"})
        return [_Model("Gemini 2.5 Pro", "gemini-2.5-pro")]

    async def generate_content(self, query, **kwargs):
        _log({"kind": "generate_content", "query": query, "kwargs": kwargs})
        return _Response(_response())

    async def generate_content_stream(self, query, **kwargs):
        _log({"kind": "generate_content_stream", "query": query, "kwargs": kwargs})
        for part in _response().split("|"):
            yield _Chunk(part)
