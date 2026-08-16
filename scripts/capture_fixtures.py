#!/usr/bin/env python3
"""Record real Open Cloud responses as test fixtures, with player data removed.

Why record rather than hand-write: every fixture here encodes a detail the
OpenAPI spec gets wrong or omits. `nextPageToken` is `""` on cloud/v2 and
`null` on server-management. `uptime` is a .NET TimeSpan with an optional
fractional part. `frameRate` is `null` on a server that just started and `0`
on one that has stopped. A hand-written fixture would encode what the spec
says, and the tests would agree with the bug.

Run it when Roblox changes something and a test starts failing, to see what
actually moved:

    python scripts/capture_fixtures.py

Requires the two keys created by `rbx apikey create` in testenv/ and
prodread/. Never writes: every request it makes is a GET.

Both directories need their `rbxapikey.toml` and `rbxplace.toml`, which are
gitignored: copy the `.example` next to each and fill in the real values from
`.local/real-ids.toml`. See docs/ops.md.

SANITISING: `playerIds` holds real Roblox user ids and `jobId` identifies a
real server. Both are replaced with deterministic synthetic values. Everything
else is kept byte for byte, because the point is to preserve reality.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "crates" / "rbx-servers" / "tests" / "fixtures"

PROD_PLACE = "55443322110099"
TEST_PLACE = "77889900112233"

ANALYTICS_WINDOW = {
    "granularity": "OneDay",
    "startTime": "2026-07-28T00:00:00Z",
    "endTime": "2026-08-01T00:00:00Z",
}

# (filename, working directory, env name, path, body or None, what it pins down)
ANALYTICS_CAPTURES = [
    (
        "analytics_metrics.json",
        "prodread",
        "prod_readonly",
        "analytics-query-api/v1/universes/{universe}/metrics",
        {"metric": "DailyActiveUsers", **ANALYTICS_WINDOW},
        "a completed query: `done` true with the result inline, so the async "
        "polling path is not always taken",
    ),
    (
        "analytics_error.json",
        "prodread",
        "prod_readonly",
        "analytics-query-api/v1/universes/{universe}/metrics",
        {"metric": "NoSuchMetric", **ANALYTICS_WINDOW},
        "a failed query: HTTP 400, but the body is still the operation "
        "envelope, with `error` where `response` would be",
    ),
]

# (filename, working directory, env name, path, what it pins down)
CAPTURES = [
    (
        "game_servers_active.json",
        "prodread",
        "prod_readonly",
        f"server-management/v1/universes/{{universe}}/places/{PROD_PLACE}"
        f"/versions/3991/game-servers?MaxPageSize=5",
        "live servers: frameRate null on a fresh one, uptime with no fraction, "
        "terminationTime null, and a real non-empty nextPageToken",
    ),
    (
        "game_servers_terminated.json",
        "prodread",
        "prod_readonly",
        f"server-management/v1/universes/{{universe}}/places/{PROD_PLACE}"
        f"/versions/3982/game-servers?MaxPageSize=5",
        "stopped servers: terminationTime set, frameRate 0 rather than null, "
        "uptime with a 7-digit fraction, shutDown true",
    ),
    (
        "filter_options.json",
        "prodread",
        "prod_readonly",
        f"server-management/v1/universes/{{universe}}/places/{PROD_PLACE}"
        f"/game-servers:filter-options",
        "the PlaceVersion list a caller must read before it can name a version",
    ),
    (
        "game_servers_empty.json",
        "testenv",
        "ops",
        f"server-management/v1/universes/{{universe}}/places/{TEST_PLACE}"
        f"/versions/1/game-servers",
        "no servers at all: every pagination field null, totalCount 0",
    ),
    (
        "user_restrictions_empty.json",
        "testenv",
        "ops",
        "cloud/v2/universes/{universe}/user-restrictions",
        'cloud/v2 signals "no more pages" with an empty string, not null. A '
        "client that treats it as a token paginates forever",
    ),
]


# Pagination cursors are base64 blobs, and Roblox packs real data inside them:
# decoding one from server-management yields a query hash, live row counts, and
# a `LastGameId` that is a real production job id. Replacing only the `jobId`
# field left that copy behind, so the whole token is replaced instead.
#
# The tests only ever ask "is there a token", never what is in it, so an opaque
# placeholder preserves everything they check.
TOKEN_FIELDS = {"nextPageToken", "previousPageToken", "pageToken"}
SYNTHETIC_TOKEN = "c2FuaXRpc2VkLXBhZ2UtdG9rZW4"


def sanitise(node, counter):
    """Walk the response, replacing anything that identifies a real person."""
    if isinstance(node, dict):
        out = {}
        for key, value in node.items():
            if key == "playerIds" and isinstance(value, list):
                out[key] = [1000000000 + i for i in range(len(value))]
            elif key == "jobId" and isinstance(value, str):
                counter[0] += 1
                out[key] = f"00000000-0000-4000-8000-{counter[0]:012d}"
            elif key in TOKEN_FIELDS and isinstance(value, str) and value:
                out[key] = SYNTHETIC_TOKEN
            elif key == "path" and isinstance(value, str):
                # The analytics operation path embeds a query hash.
                out[key] = "v1/universes/000/operations/metrics/sanitised"
            else:
                out[key] = sanitise(value, counter)
        return out
    if isinstance(node, list):
        return [sanitise(item, counter) for item in node]
    return node


def resolve_key(directory: str, name: str) -> str:
    result = subprocess.run(
        ["cargo", "run", "-q", "-p", "rbx", "--manifest-path", "../Cargo.toml",
         "--", "apikey", "resolve", name],
        cwd=ROOT / directory, capture_output=True, text=True, check=True,
    )
    return result.stdout.strip()


def probe(directory: str, env: str, path: str, api_key: str, body=None) -> str:
    command = ["cargo", "run", "-q", "-p", "rbx-ops", "--manifest-path", "../Cargo.toml",
               "--", "probe", path, "-e", env]
    if body is not None:
        # The analytics query is a POST that reads nothing but queries: probe
        # gates every non-GET behind --apply, which is the right default even
        # though this particular call cannot change anything.
        command += ["-X", "POST", "-d", json.dumps(body), "--apply"]
    result = subprocess.run(
        command,
        cwd=ROOT / directory, capture_output=True, text=True,
        env={**__import__("os").environ, "RBX_API_KEY": api_key},
    )
    # A deliberately failing capture (analytics_error) exits non-zero with the
    # body on stderr, which is exactly the body we want to record.
    return result.stdout if result.stdout.strip() else result.stderr


def main() -> int:
    FIXTURES.mkdir(parents=True, exist_ok=True)
    keys = {
        "prodread": resolve_key("prodread", "viewer"),
        "testenv": resolve_key("testenv", "readonly"),
    }

    jobs = [(f, d, e, p, None, pins) for f, d, e, p, pins in CAPTURES]
    jobs += ANALYTICS_CAPTURES

    for filename, directory, env, path, request_body, pins in jobs:
        raw = probe(directory, env, path, keys[directory], request_body)
        # An error capture arrives wrapped in the CLI's message; take the JSON.
        start = min((i for i in (raw.find("{"), raw.find("[")) if i != -1), default=-1)
        if start == -1:
            print(f"  ! {filename}: no JSON in response\n{raw[:400]}")
            return 1
        try:
            payload = json.loads(raw[start:])
        except json.JSONDecodeError:
            print(f"  ! {filename}: response was not JSON\n{raw[:400]}")
            return 1

        cleaned = sanitise(payload, [0])
        target = FIXTURES / filename
        target.write_text(json.dumps(cleaned, indent=2) + "\n", encoding="utf-8")
        print(f"  wrote {filename:34} {pins}")

    print(f"\n{len(jobs)} fixtures written to {FIXTURES.relative_to(ROOT)}")
    print("Player ids and job ids are synthetic. Everything else is as returned.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
