# rbx servers

Live and terminated servers for an experience, and what one of them logged before it stopped.

Roblox keeps a rolling **30-day window** of terminated servers, then discards them. That window is the whole argument for pulling this on a schedule rather than looking at it after something has already gone wrong.

See [ops.md](../ops.md) for install, keys and the safety model. Everything here needs only `universe:read`.

## Finding a version first

`ListGameServers` takes a place version in its path and offers no "all versions" form, so you cannot query it without knowing a version number. `versions` is how you find one:

```sh
rbx servers versions --env prod
```

```text
place versions with servers (newest first)
  * 412
    407
```

The `*` marks the default `list` uses when you do not pass `--version`.

### `--json`

```sh
rbx servers versions --env prod --json
```

```json
{
  "schema_version": 1,
  "default_place_version": "412",
  "place_versions": ["412", "407"]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |
| `default_place_version` | string | The version `list` and `logs` use without `--version`, i.e. the one marked `*`. **Absent** when no version has servers |
| `place_versions` | array of strings | Every version that has servers, newest first |

The default is named rather than left as "element 0", so a script does not have to know the ordering to pick the right one:

```sh
version=$(rbx servers versions --env prod --json | jq -r '.default_place_version // empty')
[ -n "$version" ] && rbx servers list --env prod --version "$version" --json
```

An experience that has run nothing in thirty days gives an empty `place_versions` and no `default_place_version`, with the sentence explaining it on stderr. Exit 0 either way: nothing has run is not a failure.

## Listing servers

```sh
rbx servers list --env prod
```

```text
STATUS      JOB                  UPTIME   MEMORY     FPS  PLAYERS
active      9680282c                16s   569 MB      60  2/7
active      9622e310                28s   574 MB      60  2/7
active      d093f1bf                42s   572 MB      60  1/7

3 rows for place version 412, 24610 exist (--limit 3 reached)
```

| Flag | Meaning |
| --- | --- |
| `--version <n>` | A specific place version. Defaults to the newest that has servers. |
| `--status <s>` | Only this status. |
| `--limit <n>` | Rows to fetch. Default 50. |
| `--full` | Show whole job ids instead of the first eight characters. |
| `--csv` | CSV instead of a table. |
| `--json` | One JSON document instead of a table. Rejected together with `--csv` or `--full`. |

The default limit is small on purpose. A busy experience can have **tens of thousands of rows for one place version**, which at the maximum page size is hundreds of requests. A command that quietly does that is not one you can run casually.

Statuses: `active`, `shut_down`, `restarted`, `roblox_restarted`, `crashed`, `out_of_memory`, `moderated`.

`crashed` and `out_of_memory` mean something went wrong; they are highlighted and counted separately. The others are normal lifecycle.

### `--json`

```sh
rbx servers list --env prod --version 412 --json
```

One JSON document on stdout, nothing else. Warnings — the partial-page one below in particular — stay on stderr, so a monitoring script's input parses even on the run where something was wrong.

```json
{
  "schema_version": 1,
  "place_version": "412",
  "partial": false,
  "limit": 50,
  "limit_reached": false,
  "totals": { "returned": 3, "failed": 1, "available": 24610 },
  "servers": [
    {
      "job_id": "aba9aeae-bc55-49c8-bb0e-6363ee6ba820",
      "status": "crashed",
      "failure": true,
      "place_id": "234567890123456",
      "place_version": "412",
      "engine_version": "0.700.0.7000000",
      "create_time": "2025-08-14T02:53:11Z",
      "termination_time": "2025-08-14T13:46:53Z",
      "uptime_seconds": 39222,
      "memory_bytes": 1165064601,
      "frame_rate": 60.0,
      "occupancy": 0,
      "max_occupancy": 7,
      "full": false,
      "shut_down": true,
      "type": 1,
      "player_count": 0,
      "player_ids": []
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `place_version` | string | The version the rows are for. **Absent** only when no version has servers at all |
| `partial` | boolean | Roblox answered `200` while reporting a fetch error for one of its sources. Rows are missing; any rate is a lower bound |
| `limit` | integer | The `--limit` in force |
| `limit_reached` | boolean | The run stopped at `--limit`, not at the end of the data. Raise it to see the rest |
| `totals.returned` | integer | Rows in `servers`, after `--status` filtering |
| `totals.failed` | integer | How many of those ended in a crash or out-of-memory |
| `totals.available` | integer | How many exist for this version before `--status` and `--limit`. **Absent** when Roblox did not say |
| `servers` | array of objects | One per server, in the order Roblox returned them |
| `servers[].job_id` | string | Full job id — what `servers logs` takes. Never truncated here |
| `servers[].status` | string | `active`, `shut_down`, `restarted`, `roblox_restarted`, `crashed`, `out_of_memory`, `moderated`, or `unknown` for a status this build has not seen. Same spelling `--status` takes |
| `servers[].failure` | boolean | True for `crashed` and `out_of_memory`, so a consumer does not keep its own list |
| `servers[].place_id` / `.place_version` / `.engine_version` | string | As Roblox sends them. Ids stay strings: they exceed 2^53 and a JSON number would round |
| `servers[].create_time` / `.termination_time` | string | Timestamps. `termination_time` is absent on a live server |
| `servers[].uptime_seconds` | integer | Seconds, not the `00:05:02.0020000` .NET text Roblox sends |
| `servers[].memory_bytes` | integer | Memory in use |
| `servers[].frame_rate` | number | **Absent** when Roblox reported none. A present `0` means measured zero |
| `servers[].occupancy` / `.max_occupancy` | integer | Players now, and the cap |
| `servers[].full` / `.shut_down` | boolean | As reported |
| `servers[].type` | integer | Spec enum `0..5`, named nowhere. Passed through raw rather than guessed at |
| `servers[].player_count` | integer | Length of `player_ids` |
| `servers[].player_ids` | array of integers | The ids themselves, which CSV drops for width. **Absent** when Roblox sent no list |

Optional fields are omitted rather than emitted as `null`, so `has("frame_rate")` distinguishes "never measured" from "measured zero" — the same distinction the table draws with `-`. Every row is an object keyed by name, never a positional array.

A version with no servers is an empty `servers` array and exit 0, not an error and not silence: `.servers | length` answers either way.

```sh
# crashed servers in the last window, newest first
rbx servers list --env prod --limit 500 --json \
  | jq -r '.servers[] | select(.failure) | "\(.termination_time) \(.job_id)"' | sort -r

# refuse to compute a rate off a page Roblox admits is incomplete
rbx servers list --env prod --limit 500 --json > servers.json
jq -e '.partial | not' servers.json > /dev/null \
  && jq '.totals.failed / .totals.returned' servers.json
```

## Investigating a crash

Two steps. Find the server, then read what it was doing.

```sh
rbx servers list --env prod --status crashed --limit 500 --full
```

```text
STATUS      JOB                                     UPTIME   MEMORY     FPS  PLAYERS
crashed     aba9aeae-bc55-49c8-bb0e-6363ee6ba820   10h 53m  1111 MB      60  0/7
crashed     05c2e867-9226-4123-a30f-aa168ede611e   15h 20m  1353 MB      60  5/7

2 rows for place version 407, 1830 exist
2 ended in a crash or out-of-memory
```

`--full` because the next command needs the whole job id, and the truncated form only exists because a uuid per row makes the table unreadable.

```sh
rbx servers logs aba9aeae-bc55-49c8-bb0e-6363ee6ba820 --version 407 --env prod
```

```text
13:46:53  error    ServerScriptService.Gameplay.RoundService:88: attempt to index nil with 'Name'
              Stack Begin
              Script 'ServerScriptService.Gameplay.RoundService', Line 88
              Stack End
13:59:21  error    ServerScriptService.Gameplay.RoundService:88: attempt to index nil with 'Name'
```

| Flag | Meaning |
| --- | --- |
| `--version <n>` | The version the server ran. Required by the API; defaults to the newest. |
| `--severity <s>` | `output`, `info`, `warn`, `error`. |
| `--limit <n>` | Lines to fetch. Default 200. |
| `--csv` | CSV instead of formatted lines. Stack traces are quoted, so the newlines survive. |
| `--json` | One JSON document instead of formatted lines. Rejected together with `--csv`. |

The version has to match the row the job id came from. A job id from version 407 queried against 412 returns nothing, with no error to say why, which is why an empty result says so explicitly.

Stack traces are never truncated. After a crash they are the entire reason for running this.

### `--json`

**One document per run, not one object per line.** This is the only command here where you might reasonably expect the other thing, so it is worth being explicit: `rbx servers logs --json` emits a single JSON document, the same as every other `--json` in the tool.

That is a choice about what this command is. It reads a **bounded slice of a log Roblox has already finished writing** — there is no `--follow`, the server is usually one that stopped hours ago, and nothing can be printed until pagination has stopped at `--limit`. Streaming would therefore produce no output any earlier, and would cost the envelope: which job id, which place version, which severity filter, and whether `--limit` cut the answer short are facts about the run that a line has nowhere to carry. So `jq` reads it like every other document here, and `jq -c '.lines[]'` turns it into JSON Lines if that is what you are feeding:

```sh
rbx servers logs <jobId> --version 407 --env prod --json | jq -c '.lines[]' >> logs.ndjson
```

The day a `--follow` mode exists, JSON Lines is what it should emit. That would be a new mode, not a change to this one.

```sh
rbx servers logs aba9aeae-bc55-49c8-bb0e-6363ee6ba820 --version 407 --env prod --json
```

```json
{
  "schema_version": 1,
  "job_id": "aba9aeae-bc55-49c8-bb0e-6363ee6ba820",
  "place_version": "407",
  "limit": 200,
  "limit_reached": false,
  "totals": { "returned": 2, "errors": 1 },
  "lines": [
    {
      "time": "2025-08-14T13:46:53.481Z",
      "severity": "error",
      "severity_code": 3,
      "error": true,
      "message": "ServerScriptService.Gameplay.RoundService:88: attempt to index nil with 'Name'",
      "stack_trace": "Stack Begin\nScript 'ServerScriptService.Gameplay.RoundService', Line 88\nStack End",
      "job_id": "aba9aeae-bc55-49c8-bb0e-6363ee6ba820",
      "place_version": "407"
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |
| `job_id` | string | The server asked about, in full. Present even when nothing came back |
| `place_version` | string | The version the logs were read from, whether given or defaulted |
| `severity_filter` | string | The `--severity` in force, canonicalised: `output`, `info`, `warn`, `error`. **Absent** when none was asked for |
| `limit` | integer | The `--limit` in force |
| `limit_reached` | boolean | The run stopped at `--limit`, not at the end of the log. The last line you have is then not the last line there was |
| `totals.returned` | integer | Rows in `lines`, after `--severity` filtering |
| `totals.errors` | integer | How many of those are errors |
| `lines` | array of objects | One per line, in the order Roblox returned them |
| `lines[].time` | string | Timestamp as Roblox sends it, RFC 3339 despite the `messageTimestampMs` name on the wire. **Absent** when a line carried none |
| `lines[].severity` | string | `output`, `info`, `warn`, `error`, or `unknown`. Same spelling `--severity` takes |
| `lines[].severity_code` | integer | The raw code. **Absent** when the line carried none, which tells "Roblox added a severity" apart from "this line had none" |
| `lines[].error` | boolean | True for `error` alone, so a consumer does not keep its own list |
| `lines[].message` | string | The line itself. **Absent** when there was none |
| `lines[].stack_trace` | string | Real newlines inside the JSON string, never truncated. **Absent** when the line carried none |
| `lines[].job_id` / `.place_version` | string | As Roblox reports them per line, the same columns CSV carries |

A server with no logs is an empty `lines` array and exit 0, with the "check the job id and the version" advice on stderr. The document still names the job id and version it answered about, which is what makes the empty answer readable.

```sh
# every stack trace from a crash, in order
rbx servers logs <jobId> --version 407 --env prod --severity error --json \
  | jq -r '.lines[] | select(has("stack_trace")) | .stack_trace'

# refuse to conclude anything from a slice cut short by --limit
rbx servers logs <jobId> --version 407 --env prod --json > logs.json
jq -e '.limit_reached | not' logs.json > /dev/null || echo "raise --limit"
```

## Keeping the data

Roblox discards a terminated server after thirty days and nothing brings it
back, so anything you want history for has to be exported before then.

```sh
rbx servers list --env prod --version 412 --limit 500 --csv > servers.csv
rbx servers logs <jobId> --version 412 --csv --env prod > logs.csv
```

CSV carries **every field Roblox returns**, not the six the table shows:
`engineVersion`, `createTime`, `terminationTime`, `type`, `full`, `playerCount`
and the rest. Two deliberate choices in the conversion:

- `uptimeSeconds` is a number, not the `00:05:02.0020000` text Roblox sends. A
  spreadsheet can total seconds and cannot total a .NET TimeSpan.
- `frameRate` is left **empty** when Roblox reported nothing, rather than `0`,
  so the difference between "never measured" and "measured zero" survives the
  export too.

`--json` carries the same fields, plus the player ids CSV drops for width and
the page-level facts a flat row cannot hold (`partial`, `limit_reached`,
`totals`). Use CSV for a spreadsheet and JSON for anything that pipes.

The same holds for logs: `--json` is a superset of `--csv` there too, and a
stack trace keeps real newlines inside a JSON string instead of the quoted
multi-line cell CSV has to make of it.

```sh
rbx servers logs <jobId> --version 412 --json --env prod > logs.json
```

## Reading the output honestly

Two columns mean less than they appear to.

**`FPS` shows `-`, not `0`, when Roblox reported nothing.** A server too young to have measured a frame rate reports `null`; a stopped server reports a real `0`. Those are different facts and the tool refuses to conflate them.

**A warning above the table is not cosmetic.** Roblox can answer `200 OK` while telling you, in two fields of the response body, that it failed to fetch one of its two data sources. The page is then a partial slice, so any rate computed from it is wrong. The warning is printed before the numbers rather than after.

It goes to stderr in every format, `--json` included, where the same fact is also in the document as `partial`. A script that never reads stderr still has no excuse for computing a rate off half a page.
