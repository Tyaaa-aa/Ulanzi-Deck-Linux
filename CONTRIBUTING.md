# Contributing

Contributions are welcome, including bug reports, feature requests, templates, documentation and code.

- [Reporting a bug](#reporting-a-bug)
- [Asking for a feature](#asking-for-a-feature)
- [A deck or firmware that doesn't work](#a-deck-or-firmware-that-doesnt-work)
- [Questions and sharing setups](#questions-and-sharing-setups)
- [Security](#security)
- [Contributing code](#contributing-code)
- [Plugins](#plugins)
- [Labels](#labels)

## Reporting a bug

1. Check the [troubleshooting guide](docs/troubleshooting.md) and
   [search existing issues](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues?q=is%3Aissue). If
   the problem is already reported, add a reaction to that issue rather than a "me too" comment.
2. Confirm the problem occurs on the latest release.
3. Open a [bug report](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new?template=bug_report.yml)
   with:
   - what happened and what you expected;
   - numbered steps to reproduce, from a known starting point;
   - the output of `ulanzictl diagnostics` (versions, desktop, device state and recent log lines).
     It contains no personal files, but log lines may name applications you use, so review it
     before posting;
   - screenshots for visual problems.

Report one problem per issue, with a title that describes the symptom, for example "Volume overlay
stays on screen after unplugging the deck".

Put long logs in a collapsed block:

````markdown
<details><summary>Service log</summary>

```text
paste here
```
</details>
````

## Asking for a feature

Open a [feature request](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new?template=feature_request.yml).
Describe the problem you are trying to solve before the solution you have in mind; that makes it
easier to find the right approach. Mock-ups are welcome.

## A deck or firmware that doesn't work

Use the [device support form](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new?template=device_support.yml)
with the model, the firmware version (from `ulanzictl status`) and the device's `lsusb` line.
[docs/hardware-notes.md](docs/hardware-notes.md) documents the protocol.

## Questions and sharing setups

Use [Discussions](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/discussions) for usage questions and
for sharing setups and templates (`.ulanzideck` files created with **Save as Template**).

## Security

Do not report security vulnerabilities in public issues. See [SECURITY.md](SECURITY.md).

## Contributing code

1. For anything larger than a small fix, open an issue first to agree on the approach.
2. Read the [technical guide](docs/technical.md) for the architecture, file layout, D-Bus API and
   build instructions.
3. Work on a branch. Keep commits focused and use
   [Conventional Commits](https://www.conventionalcommits.org) messages, such as `fix(deckd): …`,
   `feat(gui): …` or `docs: …`.
4. Run `scripts/test.sh`, which checks formatting, runs clippy with warnings as errors, runs all
   tests and performs an end-to-end run. New behaviour needs a test that fails without the change.
   The key editor layout test only reports sizes by default, because they depend on installed
   fonts; with `ULANZI_LAYOUT_TEST=1` it fails when an editor section would need to scroll.
5. Test manually with `scripts/sandbox.sh --demo`, which runs the app with a simulated deck and
   temporary profiles.
6. Open a pull request using the template. Include light and dark screenshots for UI changes, and
   add an entry to [CHANGELOG.md](CHANGELOG.md) under *Unreleased*.

### Style

- Code is formatted with `cargo fmt` and must pass clippy with the repository's settings. No
  `unsafe`, and no `unwrap()` outside tests.
- UI text follows the GNOME HIG: plain language, sentence case for messages, title case for buttons
  and headings. Error messages state what happened and what to do, not error codes.
- Built-in templates stay generic, without application or brand names.

## Plugins

Plugins are developed in [Ulanzi-Deck-Plugins](https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins). See
the [plugin guide](docs/plugins.md) for the contract and SDK, and that repository's README for how
plugins are tested, signed and published.

## Labels

| Label | Meaning |
|---|---|
| `bug` | Something doesn't work as documented |
| `enhancement` | A new feature or an improvement |
| `device support` | A deck model or firmware version |
| `needs triage` | Not yet reviewed |
| `needs info` | Waiting for details from the reporter |
| `good first issue` | Small, well-defined tasks suited to new contributors |
| `help wanted` | Pull requests are welcome |
