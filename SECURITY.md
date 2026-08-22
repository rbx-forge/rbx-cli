# Security policy

`rbx` handles credentials: Open Cloud API keys, and (on the commands Open
Cloud does not cover) the Roblox Studio session cookie. That makes the
threat model worth stating plainly rather than leaving to inference.

## Reporting a vulnerability

**Use GitHub private security advisories:**
<https://github.com/rbx-forge/rbx-cli/security/advisories/new>

Do not open a public issue for a vulnerability. Do not paste a real API key,
cookie, or universe id you care about into a report: a redacted reproduction
is always enough.

What helps: the `rbx --version`, the command you ran, what you expected, what
happened, and why it is a security problem rather than a bug. A proof of
concept is welcome but never required.

**What to expect.** This is a solo-maintained project. An acknowledgement
within a week is the realistic commitment; a fix lands as fast as the severity
warrants. You will be credited in the advisory and the changelog unless you
ask otherwise. There is no bounty program.

## Supported versions

Only the latest release. At 0.x, fixes go out as a new patch or minor release
rather than being backported.

## What counts

In scope, roughly in order of how much it would matter:

- A credential (API key, cookie) written to disk, logged, printed, or sent
  anywhere other than the Roblox host the command is talking to.
- A credential leaking into a place where it outlives the process: shell
  history, a lockfile, a generated Luau module, an error message copied into a
  bug report.
- Command output that discloses more of a credential than the fixed prefix
  already shown deliberately.
- A path traversal or arbitrary write from remote-controlled data: asset names,
  place names, or product names that end up as filenames during `pull` and
  `import`.
- Dependency vulnerabilities with a plausible path to any of the above.

Out of scope, because it is documented behavior rather than a defect:

- **Cookie auto-detection.** `rbx` reads the local Studio `.ROBLOSECURITY`
  cookie for the endpoints Open Cloud does not expose, and never writes it to
  disk. `--no-auto-cookie` opts out; `RBX_COOKIE` overrides. The ergonomics of
  making that consent explicit are tracked in the issue tracker, in the open.
- **An API key doing what its scopes allow.** `rbx` cannot grant itself more
  than the key carries; a key scoped too broadly is a configuration problem the
  `rbx doctor` and `rbx apikey` commands exist to surface.
- **`rbx doctor --check-ip` contacting a third party.** Comparing a key's IP
  allowlist against this machine needs the machine's public address, which
  nothing here can read off its own interfaces. The flag is opt-in, off by
  default, makes exactly one request to `https://api.ipify.org`, and names that
  service on the line reporting the address. Without the flag no packet leaves
  for anyone but Roblox.
- Anything requiring an attacker who already has your credentials, or write
  access to your machine or repository.
- Rate limits, quotas, and Roblox-side API behavior. Report those to Roblox.

## Handling credentials yourself

Two habits cover most of the risk: keep keys in `RBX_API_KEY` or your CI's
secret store rather than in a file, and scope each key to the environment it
serves. `rbx apikey` and `rbx doctor` exist to make the second one checkable:
`docs/ops.md` covers the reasoning.
