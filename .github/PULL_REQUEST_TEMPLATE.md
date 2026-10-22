## Summary

<!-- What does this PR change and why? Link the issue if there is one. -->

Fixes #

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor (no behaviour change)
- [ ] Documentation
- [ ] CI / build / tooling
- [ ] Translation / i18n

## How was it tested?

<!--
Be concrete: distro/session, commit, voice/stream scenario, language switch,
commands run. "Compiles" is not enough for voice or video changes.
-->

- [ ] `npm run check`
- [ ] `cd src-tauri && cargo test`
- [ ] Manual test (describe below)

## AI assistance

- [ ] No AI assistance
- [ ] AI-assisted (describe which parts and how you verified them)

We allow AI-assisted contributions, but you are responsible for understanding
and testing the code. See
[docs/AI_DISCLOSURE.md](https://github.com/CHEAPFISS/discord-client/blob/master/docs/AI_DISCLOSURE.md).

## Checklist

- [ ] The change is focused and does not mix refactoring/formatting with behaviour
- [ ] New UI strings are added to **both** `src/locales/en.json` and `src/locales/ru.json`
- [ ] `src/app.js` was rebuilt (`npm run fe:build`) and committed
- [ ] Rust command signatures and TypeScript types/events stay in sync
- [ ] No tokens, session files, `config.json` or unredacted logs are included
- [ ] I can license this contribution under the project's MIT license
- [ ] Documentation is updated if behaviour or setup changed

## Notes for reviewers

<!-- Known risks, areas you are unsure about, follow-ups. -->
