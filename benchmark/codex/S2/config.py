"""Runtime configuration.

Values are read from environment variables so no secrets are hard-coded.
Everything is optional; the project works out of the box with defaults.
"""

import os

API_KEY = os.environ.get("TODO_API_KEY", "")
API_BASE = os.environ.get("TODO_API_BASE", "")
DEBUG = os.environ.get("TODO_DEBUG", "0").lower() in {"1", "true", "yes", "on"}
