# Security policy

## Supported versions

Only the latest release gets fixes.

## Reporting a vulnerability

Please report privately through GitHub:
[**Security → Report a vulnerability**](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/security/advisories/new).
Don't open a public issue. Include what an attacker could do, how to reproduce it, and the version.

You'll get an answer within a week. Once fixed, the advisory is published with credit, unless you'd
rather stay anonymous.

## What's in scope

- The `ulanzi-deck --install-rules` helper, which runs as root through pkexec to install the udev rule.
- Importing `.ulanzideck` files (they come from other people).
- The session D-Bus API and the OBS connection (stored password).
- Anything that runs commands: Run Command, Command Output widget.

Keys run what their owner set up; a profile you import can contain commands, which the import
preview shows before anything is added.
