import json
import os


def gateway_url():
    return os.environ.get("GATEWAY_URL", "https://pay.example")
