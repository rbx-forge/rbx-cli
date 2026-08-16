# Third-party notices

This project is licensed under [MPL-2.0](./LICENSE). Some files in it are
**adapted from third-party sources under different licenses**, which carry
their own attribution requirements. Those licenses are reproduced in full
below, with the derived file named for each.

This file covers code and data that live *in this repository*. It is not a
dependency manifest — the licenses of crates pulled in by Cargo are recorded in
`Cargo.lock` and in each dependency's own repository.

---

## Asphalt — MIT

- Upstream: <https://github.com/jackTabsCode/asphalt>
- Derived file in this repository: `crates/rbx-core/src/image/alpha_bleed.rs`

The alpha-bleed implementation (the flood fill that rewrites fully transparent
pixels to the color of their nearest opaque neighbor) is adapted from Asphalt's
version. Asphalt itself adapted it from Tarmac; see the next section.

```
MIT License

Copyright (c) 2024 Jack Taylor

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

## Tarmac — MIT

- Upstream: <https://github.com/Roblox/tarmac>
- Derived file in this repository: `crates/rbx-core/src/image/alpha_bleed.rs`
  (indirectly, through Asphalt)

Tarmac is where the alpha-bleed algorithm originates. The chain is
Tarmac → Asphalt → this repository, so both notices apply to the same file.

```
MIT License

Copyright (c) 2020 Roblox Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

## Roblox Creator Docs OpenAPI document — CC BY 4.0

`spec/openapi.json` is a vendored, unmodified snapshot of the OpenAPI document
Roblox publishes in [`Roblox/creator-docs`](https://github.com/Roblox/creator-docs),
licensed **CC BY 4.0** rather than MPL-2.0. Its attribution, the pinned upstream
commit, and the reasoning behind vendoring it are in
[`spec/NOTICE.md`](./spec/NOTICE.md).

---

## Not derived code

Listed here because they are credited elsewhere in the project and the
distinction matters: [ROpen](https://github.com/Barocena/ROpen) (MPL-2.0)
inspired the `roblox-studio:` URI dispatch in `rbx open`,
[edit-roblox-place](https://github.com/rojo-rbx/edit-roblox-place) (MIT) sent
the same URI from a Rust CLI six years before either, and
[Mantle](https://github.com/blake-mealey/mantle) and
[rbxcloud](https://github.com/Sleitnick/rbxcloud) are prior art in this
category. **No source code from any of the four is reused**, so no notice is
owed — see the README's "Prior art and thanks" section.
