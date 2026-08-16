#!/usr/bin/env python3
"""Tests for the one function in this repository that decides whether real
player data reaches a tracked file.

`capture_fixtures.py` records live Open Cloud responses as test fixtures and
strips what identifies a person on the way through. Everything else here is
Rust and is covered; this script was the only place a silent bug publishes
personal data, and it had no test at all.

    python scripts/test_capture_fixtures.py

Standard library only, like the script it tests, so it needs nothing installed.

Every id below is invented. A test for a scrubber is the last file that should
carry a real account or universe id, and the first place one gets pasted in
while reproducing a leak.
"""

from __future__ import annotations

import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from capture_fixtures import SYNTHETIC_TOKEN, TOKEN_FIELDS, sanitise  # noqa: E402


def check(name, got, want):
    if got != want:
        print(f"FAIL {name}\n  got:  {got!r}\n  want: {want!r}")
        return 1
    print(f"ok   {name}")
    return 0


def main() -> int:
    bad = 0

    # Player ids are the whole point: they name real accounts.
    bad += check(
        "playerIds are replaced, and the count survives",
        sanitise({"playerIds": [4820175513, 156, 1]}, [0]),
        {"playerIds": [1000000000, 1000000001, 1000000002]},
    )
    bad += check(
        "an empty player list stays empty rather than gaining an entry",
        sanitise({"playerIds": []}, [0]),
        {"playerIds": []},
    )

    # A job id identifies one live server.
    out = sanitise({"jobId": "aba9aeae-bc55-49c8-bb0e-6363ee6ba820"}, [0])
    bad += check(
        "jobId is replaced with a synthetic uuid",
        out["jobId"],
        "00000000-0000-4000-8000-000000000001",
    )

    # Two servers must not collapse into one id: a fixture with two identical
    # job ids would test pagination against a case that cannot occur.
    two = sanitise({"a": {"jobId": "x"}, "b": {"jobId": "y"}}, [0])
    bad += check(
        "two job ids stay distinct",
        two["a"]["jobId"] != two["b"]["jobId"],
        True,
    )

    # The trap that was missed on the first pass: a pagination token is an
    # opaque blob with a real job id packed inside it, so replacing only the
    # jobId field left a copy of it in the cursor.
    for field in TOKEN_FIELDS:
        bad += check(
            f"{field} is replaced",
            sanitise({field: "eyJMYXN0R2FtZUlkIjoi..."}, [0])[field],
            SYNTHETIC_TOKEN,
        )

    # Nesting is where a walker usually leaks: a value one level deeper than
    # the author pictured.
    bad += check(
        "nested and inside arrays",
        sanitise({"data": [{"servers": [{"playerIds": [7]}]}]}, [0]),
        {"data": [{"servers": [{"playerIds": [1000000000]}]}]},
    )

    # The analytics operation path embeds a query hash.
    bad += check(
        "an operation path is replaced",
        sanitise({"path": "v1/universes/1234567890/operations/abc"}, [0])["path"],
        "v1/universes/000/operations/metrics/sanitised",
    )

    # Everything else is kept byte for byte. That is the point of recording
    # rather than hand-writing: the fixtures carry the details the spec gets
    # wrong, and a sanitiser that tidied them would destroy their value.
    keep = {
        "uptime": "1.02:03:04.5000000",
        "frameRate": None,
        "nextPageToken": "",
        "memory": 1165064601,
        "engineVersion": "0.700.0.7000000",
    }
    bad += check("unrelated fields are untouched", sanitise(dict(keep), [0]), keep)

    # An empty token is Roblox saying "no more pages" on cloud/v2. Replacing it
    # with a synthetic token would turn "done" into "fetch page one for ever".
    bad += check(
        "an empty token stays empty",
        sanitise({"nextPageToken": ""}, [0])["nextPageToken"],
        "",
    )

    print()
    print("FAILED" if bad else "all good")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
