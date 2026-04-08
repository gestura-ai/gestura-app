#!/usr/bin/env python3
"""Sign a Windows executable or installer using SSL.com eSigner via Jsign."""

from __future__ import annotations

import base64
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

TSA_URL = "http://ts.ssl.com"
CHOCOLATEY_JSIGN_JAR = Path(r"C:\ProgramData\chocolatey\lib\jsign\tools\jsign.jar")
BASE32_PATTERN = re.compile(r"[A-Z2-7]+=*")


def require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise SystemExit(f"Missing required environment variable: {name}")
    return value


def normalize_totp_secret(secret: str) -> str:
    compact = "".join(secret.split())
    if BASE32_PATTERN.fullmatch(compact):
        padded = compact + "=" * (-len(compact) % 8)
        raw = base64.b32decode(padded, casefold=True)
        return base64.b64encode(raw).decode("ascii")

    try:
        raw = base64.b64decode(compact, validate=True)
    except Exception as exc:  # pragma: no cover - defensive failure path
        raise SystemExit(
            "ESIGNER_TOTP_SECRET must be base64 or base32 encoded"
        ) from exc

    return base64.b64encode(raw).decode("ascii")


def jsign_command_prefix() -> list[str]:
    if CHOCOLATEY_JSIGN_JAR.is_file():
        return ["java", "-jar", str(CHOCOLATEY_JSIGN_JAR)]

    jsign = shutil.which("jsign")
    if jsign:
        return [jsign]

    raise SystemExit(
        "Jsign was not found. Install it before attempting Windows eSigner signing."
    )


def main() -> int:
    if len(sys.argv) != 2:
        print("Usage: sign-windows-esigner.py <file>", file=sys.stderr)
        return 2

    target = Path(sys.argv[1]).resolve()
    if not target.is_file():
        raise SystemExit(f"File to sign was not found: {target}")

    username = require_env("ESIGNER_USERNAME")
    password = require_env("ESIGNER_PASSWORD")
    credential_id = require_env("ESIGNER_CREDENTIAL_ID")
    totp_secret = normalize_totp_secret(require_env("ESIGNER_TOTP_SECRET"))

    command = [
        *jsign_command_prefix(),
        "--storetype",
        "ESIGNER",
        "--storepass",
        f"{username}|{password}",
        "--alias",
        credential_id,
        "--keypass",
        totp_secret,
        "--alg",
        "SHA-256",
        "--tsaurl",
        TSA_URL,
        "--tsmode",
        "RFC3161",
        str(target),
    ]

    subprocess.run(command, check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())