# Notice — vendored Roblox OpenAPI document

`spec/openapi.json` is an **unmodified byte-for-byte snapshot** of the Roblox
Creator Docs OpenAPI document. It is vendored here so the API-drift test can
run offline and deterministically, without a network call in CI.

- **Upstream repository:** <https://github.com/Roblox/creator-docs>
- **Upstream document:** `content/en-us/reference/cloud/openapi.json`
- **Pinned commit:** see [`source.json`](source.json) for the exact 40-character
  commit SHA and date this copy was taken from, and for the permalink to that
  revision — `https://github.com/{repository}/blob/{commit}/{document}`.

This file deliberately names no SHA of its own. It used to carry a permalink
with the commit written into it, which went stale the first time the
`update-openapi` workflow refreshed the snapshot: the workflow rewrites
`source.json` and has no reason to know this file exists. Two records of one
fact, one of them maintained. Since the attribution below rests on naming the
right revision, the copy that nobody updates is the one to remove.

## License

`Roblox/creator-docs` publishes under the **Creative Commons Attribution 4.0
International (CC BY 4.0)** license — confirmed both by the GitHub license API
(`spdx_id: CC-BY-4.0`) and by the text of the repository's own `LICENSE` file,
which is the standard CC BY 4.0 legal code:

- Upstream license file:
  <https://github.com/Roblox/creator-docs/blob/main/LICENSE>
- License text: <https://creativecommons.org/licenses/by/4.0/>

CC BY 4.0 permits redistribution, including verbatim redistribution such as
this snapshot, provided attribution is given. This file, together with
`source.json`, is that attribution: the material is © Roblox Corporation,
sourced from the repository named above at the commit `source.json` pins, and
is redistributed here without modification.

Note that the rest of this repository is licensed under MPL-2.0 (see the
top-level `LICENSE`). The CC BY 4.0 terms apply only to `spec/openapi.json`.

## Do not hand-edit

Do not edit `spec/openapi.json` by hand, and do not reformat or pretty-print
it. It is stored exactly as served by upstream so that diffs between refreshes
show real Roblox API changes and nothing else. Refresh it with the
`update-openapi` workflow (`.github/workflows/update-openapi.yml`), which
re-fetches the file and updates `source.json` in the same commit.
