"""Project configuration, read from environment variables.

No secrets are hard-coded in this repository.
"""

import os

API_KEY = os.environ.get("TODO_API_KEY", "")
API_BASE = os.environ.get("TODO_API_BASE", "https://api.example.com")
DEBUG = os.environ.get("TODO_DEBUG", "0") == "1"
