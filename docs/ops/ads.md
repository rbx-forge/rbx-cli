# rbx ads

Launch and steer Roblox ad campaigns.

See [ops.md](../ops.md) for install, keys and the safety model.

This page describes `main`. Check `rbx --version` against what your `rokit.toml` pins.


**This command spends real money.** Every write is dry-run by default and needs `--apply`, and `launch` prints what a run will cost before it asks.

**Roblox ships this API as an experiment.** Its announcement says request and response shapes can change and that it should not carry production-critical automation yet. The `rbx-spec-drift` test is the alarm for the day a path moves.

## What it cannot do

Read results. `/ads-management/v1` has no reporting endpoint, and that is deliberate rather than an oversight:

> Reporting is deliberately not in v1 of the Ads API. That will be available soon, for now Ads Manager remains the place to read campaign performance. You'll get it soon through the Analytics API.

So impressions, clicks and spend are read in Ads Manager. When Roblox delivers reporting through the Analytics API, it lands in [`analytics`](./analytics.md) next door rather than here.

That constraint shapes everything below.

## Testing an icon or a thumbnail

The reason this exists. One command creates one campaign per image, identical in every other respect.

```sh
rbx ads launch \
  --creative 18234567890 --creative 18234567891 --creative 18234567892 \
  --name "icon test" --budget 25 --days 14 --env prod
```

That prints three campaigns and a total, and sends nothing. Add `--apply` to create them.

Each campaign is named `icon test [18234567890]`. The asset id is in the name on purpose: the numbers are read by a human in Ads Manager, and the name is the only thread tying a row in that page back to the image it carried.

### Why not one campaign with three images

A campaign accepts up to ten creatives and Roblox distributes them evenly across players, which is a fair experiment. But Ads Manager reports **per campaign**, so three images in one campaign give you one number for the three. You would know the campaign's click-through rate and not which image earned it.

One campaign per image is the only shape whose results can be told apart.

| | Internal competition | Numbers per image |
| --- | --- | --- |
| One campaign, up to 10 creatives | none | no |
| One campaign per creative | some | yes |

### What that costs

Identical campaigns chase the same impressions, so they compete with each other and each dollar buys a little less. Worth knowing, and not worth avoiding: the inflation hits every variant equally, so the ranking survives even when the absolute numbers suffer. A ranking is what a test is for.

Do not try to dodge it by splitting the targeting, one image on phones and another on desktop. That removes the overlap and destroys the test: you would be comparing two audiences, not two images.

### Before you spend anything on thumbnails

Roblox has a free A/B test for **thumbnails** in the Creator Dashboard, measured on the real discovery surface with QPTR. If thumbnails are what you are testing, use that instead: it is free, and an ad's click-through rate is not your game tile's click-through rate.

There is no such test for the **icon**, which is what makes paid campaigns a reasonable workaround there.

## Spending twice by accident

Roblox requires an `x-idempotency-key` on create, and this tool derives it from the campaign's own definition rather than at random. Two consequences, both wanted:

- A retry cannot double-charge. The HTTP layer resends on a timeout or a 429, and every resend carries the same key.
- Running the same `launch` twice resolves to the campaigns that already exist instead of buying a second set.

To deliberately run a second test with the same images, change `--name`. You would want to anyway, to tell the two apart in Ads Manager.

## Commands

| Command | What it does |
| --- | --- |
| `launch` | One campaign per `--creative`, identical otherwise. Needs `--apply`. |
| `list` | Campaigns on the account, every page of them. `--json` available. |
| `get <id>` | One campaign in full. `--json` available. |
| `status <id>...` | Serving, in review, or blocked, for several ids in one call. `--json` available. |
| `pause [id]` | Stop campaigns. Needs `--apply`. |
| `resume [id]` | Start them again. Needs `--apply`. |
| `cancel [id]` | End them for good. Needs `--apply`. |
| `budget [id] --amount` | Change the budget. Needs `--apply`. |
| `rename <id> --name` | Change the name. Needs `--apply`. |
| `creatives` | Images available as creatives, and their moderation state. `--archived` lists archived ones instead of live. |
| `universes` | Experiences this account may advertise. |
| `accounts` | Billing accounts this key can spend from. |
| `options` | Formats, objectives, payment types, targeting, and eligibility. |

## Naming a campaign, and not remembering its id

`--name` on `launch` is free text. `icon test` is only the example above; whatever you pass becomes `<your name> [<asset id>]` on every campaign of the group.

That name then does two jobs.

**It finds the group again.** `pause`, `resume` and `cancel` take `--name` and act on every campaign whose name starts with it, which is how you stop the five you started with one command:

```sh
rbx ads pause --name "icon test" --apply
```

Campaigns already cancelled are left out rather than asked to cancel twice.

**It is how you recognise a campaign you are about to change.** Give `pause`, `resume`, `cancel` or `budget` no id at all, on a terminal, and they list the campaigns and let you pick:

```
? Which campaign should I pause?
> icon test [18234567890] · ACTIVE · SERVING · $25.00
  icon test [18234567891] · ACTIVE · IN_REVIEW · $25.00
  summer push · PAUSED · NOT_SERVING · $100.00
```

The same applies to the confirmation: it names the campaign and its state rather than echoing an id back at you. `c_8f3a91` is not something anyone can check.

Off a terminal, a missing id is an error naming the flags to pass, not a prompt a script would hang on.

Got the name wrong at launch? `rename` fixes it without touching delivery or budget.

### Picking the images

Omit `--creative` on a terminal and `launch` asks, rather than making you copy asset ids out of `ads creatives`:

```
? Which images should this test compare? (space to pick, enter to confirm)
> [x] 18234567890 · 512x512 · APPROVED · icon-red
  [x] 18234567891 · 512x512 · APPROVED · icon-blue
  [ ] 18234567892 · 512x512 · PENDING_REVIEW · icon-green
```

Fewer than two picks is refused: one image compared against nothing is not a test.

### launch

| Flag | Meaning |
| --- | --- |
| `--creative <ASSET_ID>` | Image to test. Repeat once per variant. Listing one twice is refused. |
| `--name` | Base name. Each campaign gets it plus its asset id. |
| `--budget` | Dollars **per campaign**. Five variants at 25 is 125 dollars. |
| `--budget-type` | `DAILY` or `LIFETIME`. |
| `--days` | How long to run. |
| `--start` | RFC 3339. Defaults to as soon as review clears. |
| `--payment` | `CREDIT_CARD`, `ADS_CREDIT` or `INVOICE`. See `ads options`. |
| `--country`, `--age`, `--device` | Narrow the audience. Repeatable. |
| `--apply` | Actually create them. |
| `--yes` / `-y` | Skip the confirmation. See below before you use it. |

### `--yes` on a command that spends money

`--apply` is what makes a write real; `--yes` is what removes the question in front of it. Every write here takes both — `launch`, `pause`, `resume`, `cancel`, `budget` and `rename` — and only `launch` prints a cost first.

Together they are an unattended charge. That is a reasonable thing to want in a scheduled job whose budget was decided when the job was written, and it is not a reasonable default for a terminal: the prompt is the last place a mistyped `--budget` is catchable, and a campaign cannot be un-bought. `--apply` without `--yes` is the ordinary way to run these by hand.

Budgets are typed in dollars and sent in micro-USD. `25.50` becomes `25500000`, read digit by digit rather than through a float, because `0.07` has no exact binary representation and a budget arriving as 69999 micros is the kind of defect nobody looks for.

Objective and bid strategy are not flags. The API accepts one value for each today, `ENGAGEMENT` and `AUTOMATED`, so there is nothing to choose.

### Money that moves later

An increase to a budget takes effect immediately. A decrease on a running campaign lands at the next midnight in the account's time zone, and the campaign keeps spending the higher figure until then. `budget` says so after it applies one.

## `--json`

`list`, `get` and `status` take `--json` and write **one JSON document to stdout and nothing else**. Notes — "No campaigns on this account." among them — go to stderr, so a pipeline's input parses on every run.

The write commands have no `--json`. They spend money, they are dry-run by default, and several of them ask a question when you leave the id out; a command that prompts is not one to hand a pipeline.

### list

```sh
rbx ads list --json
```

```json
{
  "schema_version": 1,
  "totals": { "returned": 2, "active": 1 },
  "campaigns": [
    {
      "id": "c_8f3a91",
      "name": "icon test [18234567890]",
      "status": "ACTIVE",
      "delivery_status": "SERVING",
      "delivery_status_reasons": [],
      "budget": { "amount_micros": "25500000", "amount_usd": "25.50", "type": "DAILY" },
      "target_universe_id": "5544332211",
      "creative_asset_ids": ["18234567890"]
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |
| `totals.returned` | integer | Rows in `campaigns` |
| `totals.active` | integer | How many are `ACTIVE`. Not the same as serving, see below |
| `campaigns[].id` | string | Campaign id |
| `campaigns[].name` | string | Free text, and load-bearing: `launch` writes the asset id into it, and it is the only thread from an Ads Manager row back to the image it carried |
| `campaigns[].status` | string | What the campaign was asked to do: `ACTIVE`, `PAUSED`, `CANCELLED` |
| `campaigns[].delivery_status` | string | What it is actually doing: `SERVING`, `IN_REVIEW`, `NOT_SERVING`, `REJECTED` |
| `campaigns[].delivery_status_reasons` | array of strings | Why, when Roblox says. Always present, empty when it said nothing |
| `campaigns[].budget` | object | **Absent** when Roblox reported none, which is not a budget of zero |
| `campaigns[].budget.amount_micros` | string | Micro-USD exactly as Roblox sent it. The authoritative figure |
| `campaigns[].budget.amount_usd` | string | The same amount in dollars, truncated to the cent. **Absent** when the micros are not a number |
| `campaigns[].budget.type` | string | `DAILY` or `LIFETIME` |
| `campaigns[].target_universe_id` | string | The experience advertised. A string: universe ids exceed 2^53 |
| `campaigns[].creative_asset_ids` | array of strings | Normally one entry, since `launch` creates one campaign per creative |

**Money is never a JSON number.** Both forms are strings, for the same reason budgets are parsed digit by digit rather than through an `f64`: `0.07` has no exact binary representation, and a budget read back as 24.999999 is the kind of defect nobody looks for. Compute on `amount_micros` and print `amount_usd`.

**`status` is not `delivery_status`.** A campaign can be `ACTIVE` and still `IN_REVIEW`, which means it is spending nothing. An alert that reads only `status` reports a test that never started as running:

```sh
# campaigns that were meant to be running and are not
rbx ads list --json \
  | jq -r '.campaigns[] | select(.status == "ACTIVE" and .delivery_status != "SERVING")
           | "\(.id) \(.delivery_status) \(.name)"'

# total daily exposure, in micros, so nothing rounds
rbx ads list --json \
  | jq '[.campaigns[] | select(.status == "ACTIVE" and .budget.type == "DAILY")
         | .budget.amount_micros | tonumber] | add'
```

An account with no campaigns is an empty `campaigns` array and exit 0.

### get

```sh
rbx ads get c_8f3a91 --json
```

The campaign sits under `campaign`, which is the same object `list` puts in `campaigns`, so one filter reads either:

```json
{ "schema_version": 1, "campaign": { "id": "c_8f3a91", "…": "…" } }
```

### status

```sh
rbx ads status c1 c2 c3 --json
```

```json
{
  "schema_version": 1,
  "totals": { "requested": 3, "returned": 2, "failed": 1 },
  "statuses": [
    { "id": "c1", "status": "ACTIVE", "delivery_status": "SERVING", "delivery_status_reasons": [] },
    {
      "id": "c2",
      "status": "ACTIVE",
      "delivery_status": "REJECTED",
      "delivery_status_reasons": ["creative violates policy"]
    }
  ],
  "failures": [{ "id": "c3", "reason": "campaign not found" }]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `totals.requested` | integer | Ids given on the command line |
| `totals.returned` | integer | Ids Roblox answered for: rows in `statuses` |
| `totals.failed` | integer | Ids it refused: rows in `failures` |
| `statuses[]` | array of objects | `id`, `status`, `delivery_status`, `delivery_status_reasons` |
| `failures[]` | array of objects | `id` and the `reason` Roblox gave. Always present, empty when every id came back |

Roblox answers `200` with both lists in one body, so an id it could not read is not a failure of the whole call. The two stay in **separate arrays** on purpose: folded together, an id nobody answered for would read as a campaign that is not serving.

```sh
# fail a deploy check if any id went unanswered
rbx ads status c1 c2 c3 --json | jq -e '.totals.failed == 0' > /dev/null
```

## Scopes

`ad.campaign:read`, `ad.campaign:write` for everything that touches campaigns, and `ad.billing:read` for `accounts`. A key without them gets an error naming the scope rather than a bare 403.

## After a launch

Campaigns come back as `IN_REVIEW`: queued for ad-policy review, not yet serving.

```sh
rbx ads status c1 c2 c3
```

`REJECTED` comes with reasons attached, which is the one piece of feedback this API does return.
