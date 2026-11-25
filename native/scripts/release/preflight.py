"""Fail before expensive builds when protected publication is not configured."""
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import subprocess


def configuration(env):
    required = ("TAURI_SIGNING_PRIVATE_KEY_B64", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
                "R2_ACCOUNT_ID", "R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY")
    missing = [name for name in required if not env.get(name)]
    if missing:
        raise ValueError("Missing protected configuration: " + ", ".join(missing))
    if env.get("GITHUB_REF") != "refs/heads/main":
        raise ValueError("Native publication is restricted to main")
    if not re.fullmatch(r"[a-fA-F0-9]{32}", env["R2_ACCOUNT_ID"]):
        raise ValueError("Invalid R2_ACCOUNT_ID")
    try:
        key = base64.b64decode(env["TAURI_SIGNING_PRIVATE_KEY_B64"], validate=True).decode()
    except (ValueError, UnicodeError):
        raise ValueError("Invalid signing key encoding") from None
    if not key.strip():
        raise ValueError("Empty signing key")


def tools():
    # Skeleton generation makes no network request and verifies the installed API model.
    result = subprocess.run(["aws", "s3api", "put-object", "--generate-cli-skeleton", "input"],
                            capture_output=True, text=True, check=True)
    model = json.loads(result.stdout)
    if not {"IfMatch", "IfNoneMatch"} <= model.keys():
        raise ValueError("AWS CLI must support conditional PutObject writes")


def signing():
    from components import sign_bytes, verify_signature
    challenge = b"NyaTerm native component signing preflight v1\n"
    verify_signature(challenge, sign_bytes(challenge))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("configuration", "tools", "signing"))
    args = parser.parse_args()
    try:
        if args.stage == "configuration": configuration(os.environ)
        elif args.stage == "tools": tools()
        else: signing()
    except (ValueError, OSError, subprocess.SubprocessError):
        # Never print subprocess command lines or signing-tool diagnostics.
        raise SystemExit(f"Native {args.stage} preflight failed; check protected configuration and tools") from None
