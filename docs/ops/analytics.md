# rbx analytics

Your own metrics: players, retention, revenue per payer.

See [ops.md](../ops.md) for install, keys and the safety model.

The Creator Dashboard already charts these. The reasons to pull them through an API are the three things it cannot do: keep history past Roblox's window, join the numbers with data of your own, and alert on a change without a human looking at a page.

## Which metrics exist

Roblox publishes no list anywhere. These were confirmed by querying a live experience:

```sh
rbx analytics metrics
```

```text
Visits                       Sessions started
DailyActiveUsers             Distinct players per day
MonthlyActiveUsers           Distinct players over the trailing month
D1Retention                  Share of new players returning the next day
D7Retention                  Share of new players returning within seven days
D30Retention                 Share of new players returning within thirty days
AverageRevenuePerPayingUser  ARPPU, in Robux
```

`--metric` accepts any string, so a metric Roblox adds later works without waiting for a release. An unknown name comes back as a readable error naming it.

`--json` writes the same list as one document on stdout:

```json
{
  "schema_version": 1,
  "exhaustive": false,
  "metrics": [
    { "name": "Visits", "description": "Sessions started" },
    { "name": "DailyActiveUsers", "description": "Distinct players per day" }
  ]
}
```

`exhaustive` is always `false`, and it is in the document rather than left to this page: the list is what was confirmed by probing, not what exists. Do not validate a metric name against it.

## Querying

```sh
rbx analytics query --metric DailyActiveUsers --days 7 --env prod
```

```text
DailyActiveUsers (total)
  2026-07-27       1200.00
  2026-07-28       1245.00
  2026-07-29       1310.00
  2026-07-31       1180.00
  2026-08-02       1402.00
```

| Flag | Meaning |
| --- | --- |
| `--metric <name>` | Required. |
| `--days <n>` | How far back. Default 30. |
| `--granularity <g>` | `one-minute`, `half-hour`, `one-hour`, `one-day` (default), `one-week`, `one-month`, `none`. |
| `--breakdown <dim>` | Split into series by a dimension. Repeatable. |
| `--filter <Dim=v[,v]>` | Narrow to particular values. Repeatable. |
| `--csv` | CSV instead of a table. |
| `--json` | One JSON document instead of a table. Rejected together with `--csv`. |

A wide range comes back queued rather than answered — Roblox hands over an operation to poll. That is handled: the command says so and waits. It gives up after a minute and tells you to narrow `--days`.

### `--json`

```sh
rbx analytics query --metric DailyActiveUsers --days 7 --env prod --json
```

One JSON document on stdout, nothing else. The waiting note above and every warning stay on stderr, so a scheduled job's input parses even on the run where Roblox queued the query.

```json
{
  "schema_version": 1,
  "metric": "DailyActiveUsers",
  "granularity": "one-day",
  "days": 7,
  "start_time": "2026-07-27T09:12:04Z",
  "end_time": "2026-08-03T09:12:04Z",
  "breakdown": [],
  "filters": [],
  "queued": false,
  "totals": { "series": 1, "points": 3, "missing": 1 },
  "series": [
    {
      "label": "total",
      "dimensions": {},
      "points": [
        { "time": "2026-07-27T00:00:00+00:00", "value": 0 },
        { "time": "2026-07-28T00:00:00+00:00" },
        { "time": "2026-07-30T00:00:00+00:00", "value": 1288 }
      ]
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `metric` | string | The metric asked for, as `--metric` spelled it |
| `granularity` | string | The bucket size in the spelling `--granularity` takes (`one-day`), not the `OneDay` form sent on the wire, so it goes straight back onto a command line |
| `days` | integer | The `--days` in force |
| `start_time` / `end_time` | string | The range actually queried, RFC 3339 UTC. Start inclusive, end exclusive |
| `breakdown` | array of strings | The dimensions `--breakdown` asked for, in order. Empty when none |
| `filters` | array of objects | The `--filter` clauses, parsed: `dimension`, `operation`, `values` |
| `queued` | boolean | Roblox did not answer inline and handed back an operation to poll. The same fact the waiting note reports, kept here because that note is on stderr |
| `totals.series` | integer | Entries in `series` |
| `totals.points` | integer | Points across every series — what Roblox returned, not what a dense range would hold |
| `totals.missing` | integer | How many of those came back with no value. Non-zero means the series has holes that are not zeros |
| `series` | array of objects | One per series, in the order Roblox returned them. A single series when nothing was broken down |
| `series[].label` | string | The short label the table and the CSV print: `total`, or the dimension values joined with ` / ` |
| `series[].dimensions` | object | The dimension values identifying this series, keyed by dimension name rather than positional. Empty without a `--breakdown` |
| `series[].points` | array of objects | The buckets Roblox returned, in its order. Empty is a real answer |
| `series[].points[].time` | string | Start of the bucket. **Absent** in the one case Roblox sends a point with no time, which the table prints as `-` |
| `series[].points[].value` | number | The measurement. **Absent** when the bucket carries none |

Every row is an object keyed by name, never a positional array, and every `--json` field name here is the compatibility surface.

#### Holes in a series

Series are not dense, and three things that look alike have to stay apart. An alert that reads a hole as a zero reports "nobody played" when what happened is "the pipeline stopped reporting".

| What you see | What it means |
| --- | --- |
| `"value": 0` | Measured zero |
| no `value` key | Roblox returned the bucket and put no number in it |
| no point for that timestamp | Roblox returned nothing for that bucket at all |

`has("value")` is the test, in line with the rule the other `--json` commands follow: an optional field is omitted rather than emitted as `null`. The `-` the table prints covers the first two cases; the document does not.

Missing buckets are never synthesised. The CLI does not know Roblox's calendar for every granularity — funnel metrics accept `--granularity none` only, and a breakdown can be ragged across series — so an invented bucket would be a guess presented as data. Reindex against `start_time` and `end_time`, which the document carries for exactly that:

```sh
# refuse to average a series that has holes in it
rbx analytics query --metric DailyActiveUsers --days 30 --env prod --json > dau.json
jq -e '.totals.missing == 0' dau.json > /dev/null \
  && jq '[.series[].points[].value] | add / length' dau.json

# the days that reported nothing, as opposed to the days that reported zero
jq -r '.series[].points[] | select(has("value") | not) | .time' dau.json
```

An empty range is an empty document and exit 0, not an error and not silence: `.totals.points` answers either way, and the "no data points" line goes to stderr.

## Breakdown or filter

They are not two spellings of the same thing, and Roblox enforces the difference:

- **`--breakdown`** splits one answer into several series, one per value.
- **`--filter`** narrows to the values you name, keeping a single series.

Some dimensions are **filter-only**. Ask to break down by one and Roblox refuses outright:

```text
Dimension FunnelName is filter-only for metric FunnelUserTotalCount and cannot be
used as a breakdown. Please use dimension-values to obtain available values.
```

Filtering to one platform, where the point is the size of the gap rather than either number:

```sh
rbx analytics query --metric DailyActiveUsers --filter Platform=Console --days 5 --env prod
```

```text
DailyActiveUsers (total)
  2026-08-01         48.00     # against 1390.00 unfiltered
```

## Finding the values to filter on

You cannot filter on a funnel name you do not know. `dimensions` lists what a dimension actually contains:

```sh
rbx analytics dimensions --metric DailyActiveUsers --dimension Platform --days 14 --env prod
```

```text
Platform
  Phone
  Tablet
  Computer
  Console
  VR
  TV
```

Where a value is an opaque id — funnel steps are — the readable label is printed beside the raw value, and the raw one is what `--filter` takes.

## Tutorial funnels

Roblox exposes the funnels your game logs with `AnalyticsService:LogFunnelStepEvent`, so "how many players reached step 3" is answerable here. The metrics are `FunnelUserTotalCount`, `FunnelUserStepCompletionRate`, `FunnelUserChurnRate`, `FunnelUserOverallCompletionRate` and their session-level twins (`FunnelStep*`), plus the cohort ones.

Two steps, because `FunnelName` is filter-only:

```sh
# 1. which funnels does this game log?
rbx analytics dimensions --metric FunnelUserTotalCount --dimension FunnelName --days 90 --env prod

# 2. players per step of one of them
rbx analytics query --metric FunnelUserTotalCount \
  --filter FunnelName=Tutorial --breakdown FunnelStep \
  --granularity none --days 90 --env prod
```

Two constraints worth knowing before you debug an empty result. Funnel metrics accept **`--granularity none` only**, which is why wide ranges are their normal case and why the queued-query handling above matters. And an empty answer usually means the game never logged the events: nothing appears here that `LogFunnelStepEvent` did not put there.

## Do not build charts here

```sh
rbx analytics query --metric DailyActiveUsers --days 90 --csv --env prod > dau.csv
```

```text
time,series,metric,value
2026-07-30T00:00:00+00:00,total,DailyActiveUsers,1288
```

Point a dashboard tool or a spreadsheet at that file. An evening of that beats weeks of writing a dashboard, and the result is better.

`--csv` and `--json` answer two different questions and the command rejects them together rather than picking one. CSV is for whatever reads a file: a spreadsheet, a charting tool, a load into a table. JSON is for whatever makes a decision: it carries the query alongside the numbers, keeps a breakdown as series instead of flattening it into a column, and distinguishes a hole from a zero, which the CSV cannot — an empty last field there is both.

The other thing worth automating is the alert, not the chart: a scheduled job that queries `D1Retention`, compares it against the previous week, and posts to Discord when it moves. A dashboard you have to remember to open tells you nothing.

---
