import json
import os


def gateway_url() -> str:
    return os.environ.get("GATEWAY_URL", "https://pay.example")
