"""Project configuration.

All values are read from the environment so no secrets are committed.
Copy .env.example to .env and fill in real values, or export the variables
directly in your shell.
"""

import os


def _env_bool(name: str, default: bool = False) -> bool:
    return os.environ.get(name, "true" if default else "false").lower() in {
        "1",
        "true",
        "yes",
        "on",
    }


API_KEY = os.environ.get("TODO_API_KEY", "")
API_BASE = os.environ.get("TODO_API_BASE", "https://api.example.com")
DEBUG = _env_bool("TODO_DEBUG", default=False)
