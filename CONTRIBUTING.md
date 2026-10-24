# Contributing

Thanks for helping make Ulanzi Deck better. Bug reports, ideas, templates, docs and code are all welcome.

- [Reporting a bug](#reporting-a-bug)
- [Asking for a feature](#asking-for-a-feature)
- [A deck or firmware that doesn't work](#a-deck-or-firmware-that-doesnt-work)
- [Questions and sharing setups](#questions-and-sharing-setups)
- [Contributing code](#contributing-code)
- [Labels](#labels)

## Reporting a bug

1. Check the [troubleshooting guide](docs/troubleshooting.md) and
   [search the issues](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues?q=is%3Aissue): a fix or
   an open report may exist already. Add a 👍 to an existing report instead of a "me too" comment.
2. Update to the latest release and check it still happens.
3. Open a [bug report](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new?template=bug_report.yml).
   The form asks for:
   - **What happened** and what you expected, in a sentence or two.
   - **Steps to reproduce**, numbered, from a known starting point.
   - **Diagnostics**: paste the output of `ulanzictl diagnostics` (versions, desktop, device state,
     recent log lines). Read it first; it contains no personal files, but log lines can name the
     apps you use.
   - **Screenshots** for anything visual. Drag them into the form.

**One problem per issue.** A good title says what goes wrong where:
*"Volume dial overlay stays on screen after unplugging"*, not *"bug"* or *"doesn't work"*.

For long logs, use a collapsed block so the issue stays easy to read:

````markdown
<details><summary>Service log</summary>

```text
paste here
```
</details>
````

## Asking for a feature

Open a [feature request](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new?template=feature_request.yml).
Describe **what you're trying to do** first, then the solution you imagine. It's easier to find
the best way when the goal is clear. Mock-ups are welcome.

## A deck or firmware that doesn't work

Use the [device support form](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new?template=device_support.yml)
with the model, the firmware version (`ulanzictl status`) and the `lsusb` line.
[docs/hardware-notes.md](docs/hardware-notes.md) explains what we know about the protocol.

## Questions and sharing setups

Use [Discussions](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/discussions) for "how do I…?",
showing off your deck, and sharing templates (`.ulanzideck` files from **Save as Template**).

## Security

Please don't open public issues for security problems; see [SECURITY.md](SECURITY.md).

## Contributing code

1. Open an issue first for anything bigger than a small fix, so we can agree on the approach.
2. Read the [technical guide](docs/technical.md) (architecture, files, D-Bus API, building).
3. Make your change on a branch. Keep commits focused, with messages in the
   [Conventional Commits](https://www.conventionalcommits.org) style: `fix(deckd): …`, `feat(gui): …`,
   `docs: …`.
4. Run `scripts/test.sh`: format, clippy (warnings count as errors), every test and an end-to-end
   run. New behaviour needs a test that fails without it. The key editor's layout test only prints
   its sizes by default, since they depend on the fonts installed; `ULANZI_LAYOUT_TEST=1
   scripts/test.sh` makes it fail when a section of the editor would need scrolling.
5. Try it: `scripts/sandbox.sh --demo` runs the app with a pretend deck on throwaway profiles.
6. Open a pull request using the template; add screenshots (light and dark) for UI changes and a line
   in [CHANGELOG.md](CHANGELOG.md) under *Unreleased*.

**Style**
- Code: `cargo fmt` and clippy with the repo's settings. No `unsafe`, no `unwrap()` outside tests.
- Text in the app: short, plain words, sentence case for messages, Title Case for buttons and headings
  (GNOME HIG). Say what happened and what to do, not error codes.
- Built-in templates stay generic (no app or brand names).

## Labels

| Label | Meaning |
|---|---|
| `bug` | Something doesn't work as documented |
| `enhancement` | A new feature or an improvement |
| `device support` | A deck model or firmware version |
| `needs triage` | Not looked at yet |
| `needs info` | Waiting for details from the reporter |
| `good first issue` | Small and well described: a good start for new contributors |
| `help wanted` | We'd welcome a pull request |
