# Third-party notices

This project is licensed under [MPL-2.0](./LICENSE). Some files in it are
**adapted from third-party sources under different licenses**, which carry
their own attribution requirements. Those licenses are reproduced in full
below, with the derived file named for each.

This file covers code and data that live *in this repository*. It is not a
dependency manifest: the licenses of crates pulled in by Cargo are recorded in
`Cargo.lock` and in each dependency's own repository.

The alpha bleed applied to icons and thumbnails before upload is adapted from
Asphalt and Tarmac (both MIT) and lives in
[rbx-core](https://github.com/rbx-forge/rbx-core), a dependency, whose
[THIRD-PARTY-NOTICES.md](https://github.com/rbx-forge/rbx-core/blob/main/THIRD-PARTY-NOTICES.md)
carries both notices.

---

## Roblox Creator Docs OpenAPI document: CC BY 4.0

`spec/openapi.json` is a vendored, unmodified snapshot of the OpenAPI document
Roblox publishes in [`Roblox/creator-docs`](https://github.com/Roblox/creator-docs),
licensed **CC BY 4.0** rather than MPL-2.0. Its attribution, the pinned upstream
commit, and the reasoning behind vendoring it are in
[`spec/NOTICE.md`](./spec/NOTICE.md).

---

## Not derived code

Listed here because they are credited elsewhere in the project and the
distinction matters: [Mantle](https://github.com/blake-mealey/mantle) and
[rbxcloud](https://github.com/Sleitnick/rbxcloud) are prior art in this
category. **No source code from either is reused**, so no notice is owed. See
the README's "Prior art and thanks" section.
