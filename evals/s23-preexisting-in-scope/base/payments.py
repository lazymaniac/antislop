import json
import os
import sys


def gateway_url():
    return os.environ.get("GATEWAY_URL", "https://pay.example")
