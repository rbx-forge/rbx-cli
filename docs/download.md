# rbx download

Download Roblox assets by id: images, audio, meshes, models, animations, places, and more.

## Features

- **Two backends** - Public `assetdelivery` endpoint by default; Open Cloud automatically when an API key is available
- **Version pinning** - `--version <n>` fetches a specific asset version (Open Cloud)
- **Batch downloads** - Pass several ids positionally or read them from a file with `--file`
- **Type-aware filenames** - Asset type is resolved from economy metadata to pick the right extension (`.png`, `.ogg`, `.rbxm`, ...)
- **Animation unwrapping** - Animation wrappers are dereferenced to their KeyframeSequence (disable with `--raw`)

## Quick start

```bash
# Download one asset into ./downloads
rbx download 123456789

# Several at once, into a chosen folder
rbx download 123 456 789 -o assets

# From a file (whitespace/comma separated, # comments allowed)
rbx download --file ids.txt
```

## Usage

```
rbx download [IDS]... [OPTIONS]
```

### Options

- `-f, --file <path>` - Read additional asset ids from a file
- `-o, --output <dir>` - Output directory (default: `downloads`)
- `--public` - Force the public assetdelivery backend even when an API key is set
- `--type <id|alias>` - Skip the economy metadata lookup by giving the asset type yourself. Aliases: `image`, `audio`, `mesh`, `lua`, `place`, `model`, `animation`, `video`, `font`
- `--version <n>` - Pin a specific asset version (Open Cloud only; requires an API key and exactly one id)
- `--raw` - Don't dereference Animation wrappers to their KeyframeSequence

## Backend selection

| Situation | Backend |
| --- | --- |
| No API key | Public `assetdelivery.roblox.com` (optional Studio cookie, opt-in — see [cookie.md](./cookie.md#auto-detection-is-opt-in)) |
| `--api-key` / `RBX_API_KEY` set | Open Cloud `asset-delivery-api` |
| `--version <n>` given | Open Cloud (requires an API key) |
| `--public` given | Public, always |

The public backend can fetch most publicly available assets without auth; a `.ROBLOSECURITY` cookie (`--cookie`, or a local Studio install once you have said yes to it) extends reach to assets your account can access. The Open Cloud backend uses your API key's permissions and is the only one supporting `--version`.

Only the public backend ever sends the cookie, and it sends it to two hosts: `assetdelivery` for the bytes and `economy` for the asset type behind the filename. Pass `--no-auto-cookie` to fetch strictly what is public. [docs/cookie.md](./cookie.md) has the trust model: resolution order, the stderr notice, and why the cookie is never written to disk.

## Filenames

Files are saved as `<id>_<sanitized name>.<ext>` (or `<id>.<ext>` when the name is unavailable). The extension comes from the asset's `AssetTypeId` via the economy details endpoint, unless `--type` was given, in which case no metadata lookup happens at all.
