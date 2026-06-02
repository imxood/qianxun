#!/usr/bin/env python3
"""Mint a HS256 JWT for daemon E2E tests.

Usage: mint_jwt.py [sub] [secret]
Output: <jwt_string>
"""
import sys
import json
import base64
import hmac
import hashlib
import time


def b64url(d: bytes) -> str:
    return base64.urlsafe_b64encode(d).rstrip(b"=").decode()


def main():
    sub = sys.argv[1] if len(sys.argv) > 1 else "test_e2e"
    secret = sys.argv[2] if len(sys.argv) > 2 else "test-jwt-secret-2026-stage8a"
    header = {"alg": "HS256", "typ": "JWT"}
    payload = {
        "sub": sub,
        "exp": int(time.time()) + 3600,
        "iat": int(time.time()),
    }
    h = b64url(json.dumps(header, separators=(",", ":")).encode())
    p = b64url(json.dumps(payload, separators=(",", ":")).encode())
    sig = b64url(hmac.new(secret.encode(), f"{h}.{p}".encode(), hashlib.sha256).digest())
    print(f"{h}.{p}.{sig}")


if __name__ == "__main__":
    main()
