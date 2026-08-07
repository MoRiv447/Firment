"""Project configuration.

Values are read from environment variables so no secrets are ever committed.
See .env.example for the full list of supported variables.
"""

import os

API_KEY = os.getenv("TODO_API_KEY", "")
API_BASE = os.getenv("TODO_API_BASE", "https://api.example.com")
DEBUG = os.getenv("TODO_DEBUG", "").strip().lower() in {"1", "true", "yes", "on"}
