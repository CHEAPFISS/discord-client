# Legal notes

This document is a plain-language summary, not legal advice. It explains why the
project uses the MIT license, what that license does and does not cover, and how
Discord's Terms of Service apply to this client.

## TL;DR

- The **source code** is licensed under **MIT** (see [../LICENSE](../LICENSE)).
- The **MIT license is not a permission slip for Discord's service**. Using this
  client with a Discord account is still governed by Discord's Terms of Service.
- This client authenticates with a **user token**. Discord prohibits automating
  user accounts ("self-bots") and using unauthorized third-party clients. Doing
  so can get an account **permanently terminated**.
- The project is **not affiliated with, endorsed by, or supported by Discord
  Inc.** "Discord" is a trademark of Discord Inc.
- No warranty. You use it at your own risk.

## Why MIT?

MIT is a short, permissive license that:

1. **Is compatible with the vendored dependencies.** `vendor/oto` is dual
   licensed MIT OR Apache-2.0, and its bundled `davey` is MIT. MIT lets us
   combine and redistribute all of it without adding copyleft obligations.
2. **Lets people actually contribute and reuse the code.** Forking, modifying,
   shipping a patched build and using parts in other projects are all allowed,
   as long as the copyright notice and license text are preserved.
3. **Keeps the legal surface small.** There is no patent grant (as in
   Apache-2.0) and no copyleft (as in GPL). For an experimental, AI-generated
   client, a simple "as is, no warranty" license is the honest choice.

Alternatives considered:

- **Apache-2.0** — also compatible with oto and a good choice, but longer and
  adds patent/notice obligations that this project does not need yet.
- **GPL-3.0** — would force every distributed derivative to be GPL. That is a
  valid philosophical choice, but it would not change the Discord ToS problem
  and would make reuse harder for contributors.
- **No license** — by default, copyright law forbids copying and modifying the
  code. That would make the repository "publicly visible" but not open source.
  This is why a license was added before inviting contributors.

## What MIT covers — and what it does not

MIT covers **copyright** in the source code: copying, modifying, merging,
publishing, distributing, sublicensing and selling copies, provided the notice
is kept. It includes a warranty disclaimer and a limitation of liability.

MIT does **not**:

- grant rights to Discord's trademarks, logos, artwork, emojis or other
  intellectual property;
- grant access to Discord's servers or APIs;
- override a contract between you and Discord (the Terms of Service);
- make a ToS violation lawful;
- guarantee that the software works, is secure, or will not get an account
  banned.

In short: the license governs the **code**; Discord's terms govern the
**service**. They are independent legal instruments.

## Discord Terms of Service

Discord's Terms of Service (https://discord.com/terms) prohibit, among other
things:

- using self-bots or otherwise automating a user account;
- using unauthorized third-party clients or modified clients;
- circumventing technical measures or rate limits;
- sharing account credentials or tokens.

This client connects with a **user token** obtained from the official client. It
does not use a bot token. From Discord's point of view that is an unauthorized
client / self-bot pattern, regardless of how the code is licensed. Discord can
detect unusual clients and terminate accounts. The project's README and sign-in
screen say this explicitly.

Using the official OAuth2 flow (client id + client secret from your own Discord
application) is technically a different authentication path, but it does not
automatically make a third-party client compliant: Discord's terms and the
application review process still apply. The OAuth flow here is intended for
personal experimentation.

**Do not**:

- run this as a public service or sell access to it;
- use it to spam, scrape, mass-DM or evade moderation;
- distribute it as an official Discord app or use Discord branding;
- include other people's tokens, messages or personal data in the repository.

## Distribution

If you distribute binaries or source forks:

- keep [LICENSE](../LICENSE) and [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md);
- do not bundle Discord assets (icons, sounds, fonts) unless their licenses allow it;
- do not ship a built-in token or someone else's OAuth client secret;
- make the ToS warning visible to your users;
- remember that the MIT warranty disclaimer protects the authors, not you from
  Discord's enforcement.

## Third-party components

`vendor/oto` includes its own `LICENSE-MIT`, `LICENSE-APACHE` and
`THIRD_PARTY_NOTICES.md`; `davey` is MIT. System libraries (OpenH264,
libjpeg-turbo, GStreamer, WebKitGTK, GTK) are dynamically linked and covered by
their own licenses. See [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) for
the summary.

## Privacy

- The token and OAuth config are stored locally in
  `~/.config/discord-client/` with `0600` permissions.
- The app talks to Discord's REST, Gateway and voice endpoints. No analytics or
  telemetry are sent anywhere else.
- Logs can contain channel ids, user ids and (at `RUST_LOG=trace`) authorization
  headers. Redact logs before publishing them.

## Questions

If you are unsure whether your use is allowed, ask Discord — not the maintainers
of this repository. The maintainers cannot grant permission to use Discord's
service.
