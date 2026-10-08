# Security Policy

[简体中文](SECURITY.zh-CN.md)

mouseshare forwards keyboard and mouse input between computers, so its security
matters. Thank you for helping keep users safe.

## Supported versions

Security fixes are released for the latest minor version. Older versions get
fixes only for critical issues at the maintainers' discretion.

| Version | Supported |
|---|---|
| 0.2.x | ✅ |
| 0.1.x | ❌ — unauthenticated and unencrypted; please upgrade |

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Report it privately through GitHub:
**[Report a vulnerability](https://github.com/PelerYuan/mouseshare/security/advisories/new)**
(Security tab → *Report a vulnerability*).

Please include what you found, how to reproduce it, the affected version and
platform, and the impact you think it has. A proof of concept helps but is not
required.

### What to expect

- Acknowledgement within **3 days**.
- An initial assessment within **7 days**, and regular updates after that.
- A fix and a coordinated advisory; you are credited unless you prefer not to
  be. We ask for up to **90 days** before public disclosure so a fix can reach
  users.

## Scope

In scope: the authentication and encryption design, key handling,
`pairing.toml` handling, anything that lets an unauthenticated network peer
inject input, read the clipboard, or crash a target, and privilege problems in
the installers.

Out of scope: attacks that need prior control of a participating computer or
knowledge of the pairing code, denial of service by a network-level attacker,
and issues in unsupported versions.

## Security design in brief

- Pairing code (50 bits) as a SPAKE2 password: no offline guessing; each guess is
  one live attempt, and the target allows 5 failed attempts per source address
  per minute.
- ChaCha20-Poly1305 with per-direction keys and counter nonces.
- Pairing codes are stored in `~/.config/mouseshare/pairing.toml` with mode
  `0600`, separately from the shareable `settings.toml`.
- The target accepts one authenticated controller at a time.

Full details: [docs/PROTOCOL.md](docs/PROTOCOL.md#security-design).
