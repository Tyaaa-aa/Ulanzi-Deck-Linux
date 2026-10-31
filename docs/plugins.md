# Writing plugins

Plugins extend Ulanzi Deck with new actions and integrations. A plugin is a WebAssembly component
that the service (`ulanzi-deckd`) runs in a sandbox, with access limited to the permissions declared
in its manifest. This document describes the plugin contract, the manifest format, the Rust SDK and
the tooling for building, testing and publishing plugins. For the architecture of the app itself,
see the [technical guide](technical.md).

## Contents

- [Plugin structure](#plugin-structure)
- [The contract](#the-contract)
- [The manifest](#the-manifest)
- [Permissions](#permissions)
- [The SDK](#the-sdk)
- [Limits](#limits)
- [Secrets](#secrets)
- [Building and installing](#building-and-installing)
- [Example: hello](#example-hello)
- [The plugin store](#the-plugin-store)
- [Publishing a plugin](#publishing-a-plugin)
- [Reference plugins](#reference-plugins)
- [Managing plugins](#managing-plugins)
- [Contract versions](#contract-versions)

## Plugin structure

A plugin is a folder containing:

```text
plugin.toml     name, version, permissions and the actions it provides
plugin.wasm     the program (a WebAssembly component)
assets/         optional: pictures and other files for the plugin
```

Installed plugins are stored in `~/.local/share/ulanzi-deck/plugins/<id>/` (following
`XDG_DATA_HOME`). The service compiles each `plugin.wasm` once and caches the result in
`~/.cache/ulanzi-deck/plugins/`.

Sandbox rules:

- No file system access, an empty environment, and no `wasi:sockets` or `wasi:http`. Standard output
  and standard error go to the service log. Clocks and random numbers are available.
- All network connections go through the service, which checks them against the plugin's
  permissions.
- Each plugin has 64 MiB of memory and 500 ms per call. Between calls, a plugin runs only in response
  to timers and sockets owned by the service.
- A plugin that crashes, runs out of memory or exceeds its time limit is stopped. The deck and other
  plugins are not affected.

A plugin starts when one of its actions is first used, or when the service starts if a profile uses
it or its manifest sets `background = true`. Plugins load in the background after the deck is ready;
a plugin key pressed before then reports "plugins are still starting".

## The contract

The interface between plugin and service is written in WIT:
[`crates/ulanzi-plugin/wit/plugin.wit`](../crates/ulanzi-plugin/wit/plugin.wit), package
`ulanzi:plugin@1.1.0`. Most plugins use it through the SDK rather than directly.

Exports (called by the service):

| Export | When |
|---|---|
| `init(settings)` | At start and when the plugin's settings change. `settings` is the `[plugins.<id>.settings]` table from `settings.toml` as JSON. If it returns an error, the error is logged. |
| `on-action(action, params)` | A key or dial runs one of your actions. `params` is the action's parameters as JSON. An error is shown to the user. |
| `on-open(socket)` | A socket you opened is connected. |
| `on-message(socket, data)` | Data arrived on a socket (text or binary). |
| `on-closed(socket, reason)` | A socket closed or failed to open. |
| `on-http(request, response)` | A `fetch` finished, with the response or an error text. |
| `on-timer(timer)` | A timer fired. |
| `list-items(picker, query, group)` | Contract 1.1. The user opened one of your [pickers](#pickers). Return its entries that match `query` (what the user typed, `""` for all), only those in `group` unless it is `""`. An error (for example "not connected") is shown with Retry and "Enter manually…". |

Imports (called by the plugin):

| Interface | Function | Does |
|---|---|---|
| `core` | `set-timer(ms, repeat)`, `cancel-timer(id)` | Timers. Repeating timers fire at most every 10 ms. |
| `core` | `set-state(key, value)` | Live values shown on keys, see [Showing state on keys](#showing-state-on-keys) |
| `core` | `secret-get`, `secret-set`, `secret-delete` | Private storage, see [Secrets](#secrets) |
| `core` | `set-status(status, message)` | `ok`, `connecting` or `problem`, with a short text shown next to the plugin |
| `core` | `log(level, message)` | A line in the service log, tagged with the plugin id |
| `net` | `ws-open(url)` | A WebSocket (`ws://` or `wss://`) |
| `net` | `tcp-open(host, port, tls)` | A TCP socket, optionally TLS |
| `net` | `send(socket, data)`, `close(socket)` | Write to or close a socket |
| `net` | `fetch(request)` | An HTTP request |

All imports are non-blocking. `ws-open`, `tcp-open` and `fetch` return an id immediately, and the
result arrives later as an event (`on-open`, `on-closed`, `on-http`). A plugin receives its events
one at a time, in order. A dial turn is delivered as an `on-action` for the action assigned to that
direction (left, right, press or hold).

### Pickers

Contract 1.1 adds `list-items`, which lets users pick a scene or entity from a list instead of
typing its name. Declare each list in the manifest and reference it from `item` fields:

```toml
contract = "1.1"

[[pickers]]
id = "scenes"
style = "grid"      # list (default) | grid
groups = true       # entries have groups; the app offers a group filter
multi = false       # true: several entries; settings only, the value is a list of ids

[[actions]]
id = "set_scene"
name = "Switch scene"
params = [{ key = "scene", kind = "item", picker = "scenes", label = "Scene" }]
```

Each entry (`item` in the WIT, `Item` in the SDK) has an `id` (what is stored), a `label` (what the user
sees) and optionally a `subtitle`, a `group`, an `icon` (an icon name or short text) and a `state` (live
state shown dimmed next to it, for example "on").

Answer `list-items` from data the plugin already holds, such as the scenes received on connect or
the entity states it keeps current. Do not wait for the network: the call has the same 500 ms
budget as any other, and the app waits for the result. The service starts a plugin that is not running yet when its
picker opens. It keeps at most 5000 entries and drops entries whose id is empty or longer than 1024
bytes. It cuts `label`, `subtitle`, `group` and `state` to 256 bytes, `icon` to 128 and an error text to
4096. A picker the manifest does not declare is refused before the plugin is asked. A plugin built for
1.0 has no `list-items`: its pickers answer "<name> has no list to pick from".

To offer different lists to different actions (for example lights for Toggle and scripts for Run),
declare a separate picker for each.

### Showing state on keys

`set-state("count", "3")` makes `{hello.count}` available in any key label (`{<plugin id>.<key>}`).
The key redraws when the value changes. When the plugin stops, its values are forgotten.

## The manifest

`plugin.toml`:

```toml
id = "hello"                 # a-z, 0-9 and _, starting with a letter, 2 to 32 characters
name = "Hello"
version = "1.0.0"            # MAJOR.MINOR.PATCH
contract = "1.0"             # the contract version it was written for
description = "A minimal example: a counter and a greeting."
background = false           # true: start with the service even if no profile uses it

[permissions]
network = ["api.example.com", "*.example.net"]
lan = true

[[settings]]
key = "server"
kind = "text"
label = "Server"
default = "api.example.com"

[[settings]]
key = "api_key"
kind = "secret"
label = "API Key"
help = "From your account page"

[[actions]]
id = "greet"
name = "Greet"
params = [{ key = "name", kind = "text", label = "Name" }]
```

| Field | Meaning |
|---|---|
| `id`, `name`, `version`, `contract` | Required. The folder name must equal the id. |
| `description`, `categories`, `tested_on_hardware` | Optional, shown in the plugin manager and the store. |
| `author` | Optional. Shown as "by *author*" in the plugin manager and the store. |
| `min_app` | Optional. The oldest Ulanzi Deck version the plugin supports; see below. |
| `icon`, `symbolic_icon`, `color` | Optional, the plugin's look; see [Icons](#icons). |
| `background` | Optional, default `false`. |
| `[permissions]` | Optional, see below. Nothing is allowed that is not listed. |
| `[[settings]]` | Optional. The plugin's settings page in the app, one field each; see [Settings and parameters](#settings-and-parameters). |
| `[[actions]]` | The actions the plugin offers: `id` and `name` (required, ids unique), `description` (one line on what it does, shown when pointing at it in the action list; older apps ignore it), `icon` (see [Icons](#icons)), and `params` (fields, as below). |
| `[[pickers]]` | Contract 1.1. Lists that `item` fields are picked from: `id` (required, unique), `style`, `groups`, `multi`; see [Pickers](#pickers). |
| `[[key_sets]]` | Contract 1.1. Ready-made keys for items the user picks; see [Ready-made keys](#ready-made-keys). |

A key uses an action like this in `profile.toml`:

```toml
[pages.keys.1.tap]
type = "plugin"
plugin = "hello"
action = "greet"
params = { name = "Ana" }
```

Values in `params` and in `[plugins.<id>.settings]` that have no JSON form (dates and times) reach the plugin as strings.

**Contract versions.** A plugin loads if its contract's major version equals the service's (1) and
its minor version is not newer than the service's (currently 2). Otherwise it is listed as
`incompatible` with the message "needs a newer Ulanzi Deck". Minor versions are additive, so a
plugin written for 1.0 works with every later 1.x release.

The manifest may also contain tables this version does not read yet (key faces, widgets, variables,
detection, sign-in). They are accepted and ignored. From contract 1.1 on, a manifest is refused when a
picker is declared twice, when an `item` field names a picker that is not declared, or when an action
param uses a `multi` picker. A `contract = "1.0"` manifest keeps the 1.0 rules: its `[[pickers]]` tables
(whatever their shape) and a field's `picker` are accepted and ignored, so its `item` fields are text
entries. In the same way a manifest older than 1.2 keeps its fields but the 1.2 ones (`symbolic_icon`,
`color`, `[[key_sets]]`, an action's `states`, `labels` and `track`, a field's `show_if`) are ignored.

**Minimum app version.** `min_app = "2.1.0"` sets the oldest Ulanzi Deck release the plugin
supports. Older releases refuse to install it and report "<name> needs Ulanzi Deck 2.x.y or newer"
in the app and in `ulanzictl plugin list`. The contract version covers the features a plugin uses;
`min_app` also covers bug fixes it depends on. Set both.

### Icons

Plugins ship their own icons in `assets/`, so the app, the plugin store and the deck display them
consistently without any icons built into the app:

| Field | File | Used for |
|---|---|---|
| `icon` | `assets/….svg` or `.png`, full colour, square (at least 128 px for a PNG) | The plugin in lists, its settings page, the category menu and the plugin store. |
| `symbolic_icon` | `assets/….svg`, one colour | Small places that follow the theme's colours. |
| `color` | `#rrggbb` | The colour the plugin's action icons get on fresh keys (white when missing). |
| an action's `icon` | `assets/….svg`, one colour | The action in the action list and on a fresh key. |
| a key set key's `icon` | `assets/….svg`, one colour | That key (see [Ready-made keys](#ready-made-keys)). |

One-colour SVGs are drawn in the key's icon colour: use `currentColor` for strokes and fills, or leave
them black (the app fills the root with `currentColor`). Keep each under 512 KiB. Paths are relative to
the plugin folder, start with `assets/` and may not hold `..`; installing refuses a plugin whose manifest
names an icon that is not in it. A value without `assets/` (such as `mdi:lightbulb`) is still read as an
icon theme name, as before. A key showing a plugin icon stores it as `source = { kind = "plugin",
plugin = "<id>", icon = "assets/…" }`, so it changes when the plugin's file does.

```toml
icon = "assets/icon.svg"
symbolic_icon = "assets/symbolic.svg"
color = "#18bcf2"

[[actions]]
id = "toggle"
name = "Toggle an entity"
icon = "assets/actions/toggle.svg"
```

### Ready-made keys

A `[[key_sets]]` entry lets users create many keys at once. The key set appears at the top of the
plugin's section in the action list. The user picks items from a multi picker, and each item gets
the keys of the first variant whose `groups` contain the item's group (a variant without `groups`
matches any item). The app removes a shared prefix from names ("Bedroom Lamp" and "Bedroom Fan"
become "Lamp" and "Fan"), asks whether to place the keys on the current page or a new one, and keeps
each item's keys together on one page.

| Field | Meaning |
|---|---|
| `id`, `name`, `picker` | Required. `picker` names a `[[pickers]]` entry; make it `multi = true`. |
| `description`, `icon` | Shown in the action list. |
| `track` | Settings each picked item is added to, so the keys get live state: `{ setting, value, groups }`. A multi `item` setting gets the value as a list entry (up to its `max`), a `text` setting as a comma-separated entry. |
| `[[key_sets.variants]]` | `groups` (optional) and `keys`: up to 15, each with `action` (declared), `params`, `icon`, `label` and `extra`. |

In `params`, `label` and a track's `value`, `$id` becomes the picked item's id and `$name` its (short)
name. Keys with `extra = true` (brightness, speed) can be left out by the user, so each item gets only its
main key. From the Home Assistant plugin:

```toml
[[key_sets]]
id = "devices"
name = "Keys for your devices"
picker = "devices"
track = [{ setting = "entities", value = "$id" }]

[[key_sets.variants]]
groups = ["Lights"]
keys = [
  { action = "toggle", params = { entity_id = "$id" }, icon = "assets/keys/light.svg", label = "$name" },
  { action = "set_level", params = { entity_id = "$id", op = "up", step = 10 }, icon = "assets/keys/brighter.svg", label = "{ha.$id.pct}%", extra = true },
]
```

A key set key can also have `color` (its icon colour) and `states` (see below; the action's own when
missing).

### Live looks and label values

An action can define how its keys look in each live state and which live values they can display.
Users then pick a value such as "Room temperature" from a menu instead of typing
`{ha.climate.ac.attr.current_temperature}`:

| Field of `[[actions]]` | Meaning |
|---|---|
| `states` | Looks for live states, first match wins: `when = { value, is, not }`, then `icon` (one-colour `assets/…` SVG), `color`, `background`, `label`. `value` is a label text such as `{ha.$entity_id}`; `is` lists values that match (any of them), `not` values that don't. An empty value (nothing published yet) never matches. |
| `labels` | Values the key's label can show: `name` (shown in the app's Label Shows menu), `text` and optional `track`. |
| `track` | Settings to add the key's parameters to whenever a key uses the action, so the plugin publishes its state. |

In all of them `$<param>` is the key's own action parameter, filled in when the key is drawn, so a key
keeps working when its entity changes. Keys store the states in their look
(`[[pages.keys.N.look.states]]`) and use them while Show Live State is on. From the Home Assistant
plugin:

```toml
[[actions]]
id = "climate"
labels = [
  { name = "Room temperature", text = "{ha.$entity_id.attr.current_temperature}°", track = [{ setting = "attributes", value = "$entity_id:current_temperature" }] },
]
track = [{ setting = "entities", value = "$entity_id" }]
states = [
  { when = { value = "{ha.$entity_id}", is = ["cool"] }, icon = "assets/keys/cooling.svg", color = "#4fc3f7" },
]
```

Picker searches should match every word of the query anywhere in the name, id or group, in any
order. `ulanzi_plugin::matches_query` implements this.

### Settings and parameters

`[[settings]]` entries and the entries of an action's `params` are fields. Each has a `key` (required,
unique in its list), a `kind` (default `text`), and optionally `label` (the row's title; the key when
missing), `help` (shown under the row or as its tooltip), `default`, `min`, `max`, `step` and `choices`.
The app builds one row per field: the plugin's page in the Plugins window for `[[settings]]`, and
the rows under the action in the key editor for `params`.

| `kind` | Row in the app | Value written |
|---|---|---|
| `text` | Text entry, saved with its apply button | string; an empty entry removes the key |
| `number` | Number with − and + (`min`, `max`, `step`) | integer when at least one of `min`, `max`, `step`, `default` is given and all given are whole numbers, else a float |
| `toggle` | Switch | boolean |
| `choice` | Drop-down of `choices` | the chosen value (string) |
| `color` | Colour button | `"#rrggbb"` |
| `json` | Text entry that must hold valid JSON | the JSON text, as a string |
| `item` | With a `picker`: a row showing the current value that opens the plugin's list (search, a group filter when the picker has `groups`, ticks and Done for `multi`, at most `max` ticked when the field gives one; saved on the pick). A stored id the list lacks stays shown as the current value. Typing an id by hand is offered only when the plugin can't list (an error, an empty list, or a 1.0 plugin). Without a picker: a text entry | the entry's id (string); a list of ids for a `multi` picker (an old comma-separated string is read as a list and saved as one on the next pick) |
| `key_combo`, `file` | Text entry | string |
| `secret` | Password entry | nothing in `settings.toml`: see [Secrets](#secrets). Only in `[[settings]]`, never in `params`. |

`choices` is a list of plain values (`["up", "down"]`, shown as "Up", "Down") or of
`{ value = "up", label = "Louder" }` tables. A kind the app does not know is kept and shown as a text
entry. A value the row cannot edit (a table, for example) is shown read-only and left as it is; a list in
a `text` field is shown joined by `, ` and only replaced when the user edits it.

A setting's `default` is shown in the row. Before every `init` the host fills each non-secret setting the
user has not set with its manifest default, so `init` sees the default without the plugin repeating it.
A new key action starts with every `params` entry as its row shows it: the `default`, else the first
choice of a `choice`, else the number a spin row falls back to (0 within `min` and `max`).

## Permissions

Without permissions, a plugin can compute, keep state, and use timers and secrets. Everything else
requires a permission.

| Permission | Allows |
|---|---|
| `network = ["host", …]` | Connections to exactly these host names. `*.example.com` matches every subdomain (`a.example.com`, `a.b.example.com`) but not `example.com` itself; list both if you need both. A wildcard needs at least two labels after the `*.`, so `*.com` is refused. Names are not case sensitive. |
| `lan = true` | Connections to the local network: `localhost`, names ending in `.local`, IP addresses in local ranges, and any other name that resolves only to local addresses. |

Local ranges are loopback (`127.0.0.0/8`, `::1`), private IPv4 (`10.0.0.0/8`, `172.16.0.0/12`,
`192.168.0.0/16`), link-local (`169.254.0.0/16`, `fe80::/10`) and IPv6 unique-local (`fc00::/7`).
An IPv4 address written as IPv6 (`::ffff:192.168.1.5`) is judged as the IPv4 address.

Enforcement:

- The service checks a name before connecting. For `lan`, it resolves the name once and connects to
  exactly the addresses it checked, so a name cannot answer "local" for the check and "public" for the
  connection. A name that resolves to even one public address is refused under `lan`.
- A host in the `network` list is allowed whatever it resolves to.
- HTTP redirects are not followed. A 3xx response goes to the plugin, which can then ask again. A
  plugin therefore cannot be sent to a host outside its permissions.
- The error for a refused connection is `not allowed: <host>`.

`browser`, `dbus` and `programs` are accepted in the manifest and have no effect yet; they arrive with
the contract versions that use them.

Permissions are granted at installation. Before installing, the app lists the plugin's permissions
and asks for confirmation; unsigned plugins are labelled "Unverified plugin".

## The SDK

The `ulanzi-plugin` crate ([`crates/ulanzi-plugin`](../crates/ulanzi-plugin)) wraps the contract in
plain Rust. A plugin is a type that implements `Plugin`:

```rust
pub trait Plugin: Default + 'static {
    fn init(&mut self, cx: &mut Cx, settings: Value) -> Result<(), String> { Ok(()) }
    fn action(&mut self, cx: &mut Cx, action: &str, params: Value) -> Result<(), String>;
    fn opened(&mut self, cx: &mut Cx, socket: Socket) {}
    fn message(&mut self, cx: &mut Cx, socket: Socket, data: Payload) {}
    fn closed(&mut self, cx: &mut Cx, socket: Socket, reason: &str) {}
    fn http(&mut self, cx: &mut Cx, request: Request, response: Result<HttpResponse, String>) {}
    fn timer(&mut self, cx: &mut Cx, timer: Timer) {}
    fn list_items(&mut self, cx: &mut Cx, picker: &str, query: &str, group: &str)
        -> Result<Vec<Item>, String> { Err("no list to pick from".into()) }
}
```

Only `action` is required. Build entries with `Item::new(id, label)` and, as needed,
`.subtitle(…)`, `.group(…)`, `.icon(…)` and `.state(…)`. `Value` and `json!` are re-exported from `serde_json`. Call
`ulanzi_plugin::export_plugin!(MyPlugin);` once in the crate root.

`Cx` is the handle to the host: `set_timer`, `cancel_timer`, `set_state`, `secret`, `set_secret`,
`delete_secret`, `set_status`, `log`, `ws_open`, `tcp_open`, `send`, `close` and `fetch`. It holds no
data; every call goes straight to the host.

### Testing without a deck

When compiled for the host platform instead of WebAssembly, the same calls go to a mock host, so
plugin logic can be tested with `cargo test`. `ulanzi_plugin::testing::Harness` drives the plugin
and records its effects:

- Input: `init`, `action`, `list_items`, `open`, `message`, `closed`, `http`, `fire` (a timer).
- Inspection: `state`, `status`, `logs`, `timers`, `opened` (sockets opened), `sent` (data written
  to sockets), `fetched` (HTTP requests) and `secrets` (read or preloaded).

The harness does not enforce permissions or limits. Test those in the real service.

## Limits

The service enforces these for every plugin.

| What | Limit |
|---|---|
| Time per call into the plugin | 500 ms, waiting included (`std::thread::sleep` counts); the call is then ended and the plugin counts as crashed |
| Memory | 64 MiB per plugin |
| Events waiting for the plugin | 1024; when full, new events are dropped and one warning is logged per overflow. A key pressed meanwhile, or one whose action is not answered within 2 s, reports "<name> is busy" |
| Timers | 4096 per plugin; repeating timers at most every 10 ms |
| Open sockets | 64; requests in flight: 64 |
| Message size | 1 MiB per WebSocket message |
| HTTP response body | 8 MiB |
| Unsent data per socket | 8 MiB |
| Connect / HTTP timeout | 15 s / 30 s |
| State | 256 keys; key up to 256 bytes, value up to 1 KiB (it is label text). Anything over is ignored with a log line |
| Secrets | 256 keys; key up to 256 bytes, value up to 64 KiB. Anything over is ignored with a log line |
| Log line, status message | 4096 bytes; longer ones are cut |
| Picker entries (`list-items`) | 5000 entries; id 1 to 1024 bytes (others dropped); label, subtitle, group, state 256 bytes, icon 128, error text 4096 (longer ones are cut) |

Headers `Host`, `Connection`, `Transfer-Encoding` and `Content-Length` are set by the service; a request
that includes one fails with `header not allowed: <name>`. HTTP requests use no proxy.

**Crashes.** If a plugin traps, runs out of memory or uses more than its time, the service restarts it
once. A second failure within 5 minutes leaves it `failed` ("<name> stopped working") until you restart
it.

**Messages users see.** A key whose plugin is not installed says "the <id> plugin is not installed";
turned off, "the <name> plugin is turned off"; failed, "<name> stopped working"; needing a newer app,
"<name> needs a newer Ulanzi Deck". Profiles keep the action and its parameters in all of these cases, so
installing or enabling the plugin later brings the key back.

## Secrets

Store passwords, tokens and keys with `secret-set`, never with `set-state` or in settings. Secrets
are kept in the system keyring through the Secret Service API (KWallet or GNOME Keyring), which the
service contacts when a plugin starts. If the keyring does not respond within 3 seconds, secrets are
stored in
`~/.config/ulanzi-deck/plugin-secrets.toml` (mode 0600) instead. They are never logged, never sent from the service over D-Bus (the app sends them in with `PluginSetSecret`) and not part of profile exports or
backups. Removing a plugin deletes its secrets; reinstalling it keeps them.

To let the user type a secret in the app, declare it as a `kind = "secret"` setting. Its `key` is the
secret's name in the store, so the plugin reads it with `secret("<key>")`. The row is write-only: the
app never shows the value, only `<label> · saved` and a Clear button once one is stored. Saving a new
value writes it straight into the plugin's secret store, removes the same key from
`[plugins.<id>.settings]` in `settings.toml` (so an old plain-text copy cannot win) and restarts the
plugin with the new value; a plugin that is turned off or stopped working is not started. Over D-Bus, `GetSettings` shows a secret setting's value as `••••`, and
settings exports leave it out.

For configuration without the app, a plugin can also accept a secret from its settings table, as
the OBS and Home Assistant plugins do: `init` finds `password` or `token` in `[plugins.<id>.settings]`, saves it with `set_secret`
and logs a warning until the line is deleted. Until then it sits in plain text in `settings.toml`.

## Building and installing

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

Set the crate type to a dynamic library and depend on the SDK:

```toml
[lib]
crate-type = ["cdylib", "rlib"]     # cdylib for the .wasm, rlib so `cargo test` works

[dependencies]
ulanzi-plugin = { path = "../../../crates/ulanzi-plugin" }
```

Copy the build output (`target/wasm32-wasip2/release/<crate_name>.wasm`) to `plugin.wasm` next to
your `plugin.toml`, then install the folder:

```sh
ulanzictl plugin dev-mode on        # once: allow installing unsigned plugins
ulanzictl plugin install ./hello    # install, or replace and restart if already installed
ulanzictl plugin list
```

Installing from a folder is refused with "Turn on Developer mode to install plugins from a folder"
until developer mode is on. The installer validates `plugin.toml` and compiles `plugin.wasm` before copying.
The source folder can have any name; the plugin is installed in a folder named after its `id`.
Problems are reported by `plugin list`. A plugin that fails to load is listed as `incompatible` with
the reason, and does not affect other plugins.

### Packages (`.ulanziplugin`)

A plugin can also be shipped as one file, `<id>-<version>.ulanziplugin`. It is a ZIP file with
`plugin.toml` and `plugin.wasm` at the top and, if the plugin has them, an `assets/` folder:

```text
plugin.toml
plugin.wasm
assets/...        optional
```

Build one from a plugin folder with the script (it runs the release build for `wasm32-wasip2`,
needs `zip` or `python3`, and prints the path of the package, written to `dist/plugins/`):

```sh
scripts/package-plugin.sh examples/plugins/hello   # dist/plugins/hello-1.0.0.ulanziplugin
ulanzictl plugin install dist/plugins/hello-1.0.0.ulanziplugin
```

Packages from the plugin store come with a minisign signature (`<file>.minisig`). If the signature
verifies against the key built into the app and its signed comment names the package's id and
version, the package installs without Developer Mode and is shown as "Verified plugin". A signature
that fails verification is rejected, even in Developer Mode. Unsigned packages require Developer
Mode, as folder installs do ("Turn on Developer mode to install unverified plugins"); inspecting a
package does not. The service unpacks the file into a hidden temporary folder next to the installed
plugins, validates it like a plugin folder and installs it. The temporary folder is always removed.
A package is rejected, with the reason, when:

- the file is larger than 64 MiB, holds more than 1000 entries or unpacks to more than 64 MiB (the
  size is counted while unpacking, not read from the file's headers);
- an entry name is absolute, holds `..`, a backslash or a NUL, or is listed twice;
- an entry is a link (symlink) or anything but a file or folder;
- an entry is not `plugin.toml`, `plugin.wasm` or under `assets/` (a package made by zipping the plugin
  folder itself, with everything one level down, is refused for that reason);
- `plugin.toml` or `plugin.wasm` is missing, or the plugin does not pass the checks of a plugin folder.

## Example: hello

[`examples/plugins/hello`](../examples/plugins/hello) is the smallest useful plugin: a counter and a
greeting. Its manifest declares two actions, `count` and `greet` (which takes a `name`), and no
permissions.

```rust
use ulanzi_plugin::{Cx, Plugin, Value};

#[derive(Default)]
pub struct Hello {
    count: u64,
}

impl Plugin for Hello {
    fn init(&mut self, cx: &mut Cx, settings: Value) -> Result<(), String> {
        self.count = settings["start"].as_u64().unwrap_or(0);
        cx.set_state("count", self.count.to_string());
        Ok(())
    }

    fn action(&mut self, cx: &mut Cx, action: &str, params: Value) -> Result<(), String> {
        match action {
            "count" => {
                self.count += 1;
                cx.set_state("count", self.count.to_string());
                Ok(())
            }
            "greet" => {
                let name = params["name"].as_str().ok_or("greet needs a name")?;
                cx.set_state("greeting", format!("Hello, {name}!"));
                Ok(())
            }
            other => Err(format!("unknown action: {other}")),
        }
    }
}

ulanzi_plugin::export_plugin!(Hello);
```

Walk-through:

1. `init` reads the optional `start` value from `[plugins.hello.settings]` in `settings.toml` and publishes the
   count.
2. `count` adds one and publishes it. A key labelled `Count: {hello.count}` shows the new number.
3. `greet` reads its parameter and publishes `{hello.greeting}`. A missing name returns an error, which
   the user sees as the key's error.
4. Its test ([`tests/hello.rs`](../examples/plugins/hello/tests/hello.rs)) uses the `Harness` and runs
   with `cargo test -p ulanzi-plugin-hello`.

## The plugin store

The store is a signed `index.json` published on the `index` release of
[Ulanzi-Deck-Plugins](https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins). The service:

- verifies the index signature against the built-in key before using it;
- keeps the last valid copy, so the store can be browsed offline;
- installs a plugin only if the download matches the SHA-256 listed in the index and the package's
  own signature verifies.

`PluginStoreIndex`, `PluginStoreInstall` and `PluginVerified` on D-Bus correspond to
`ulanzictl plugin store` and `ulanzictl plugin get`. The app holds a list of trusted keys, so a
replacement key can be shipped in an update before the current one is retired.

## Publishing a plugin

All published plugins, including OBS Studio and Home Assistant, are maintained in
[Ulanzi-Deck-Plugins](https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins). Its CI tests each plugin in
the real plugin host and, on `main`, signs and releases each new version and rebuilds the index. This
repository only contains the hello example and the host's test plugins. To publish a plugin, open a
pull request in the plugins repository.

## Reference plugins

### OBS Studio

[`plugins/obs`](https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins/blob/main/plugins/obs/README.md)
implements the OBS integration as a sandboxed plugin. It covers several things the hello example
does not:

- A WebSocket client (`network = ["127.0.0.1", "localhost"]`) that signs in with obs-websocket 5 and
  never blocks: every handler sends and returns, answers arrive as events.
- The password in the secret store. The user types it under Plugins → OBS Studio, which
  writes it into the store. A `password` line under `[plugins.obs.settings]` still works: `init` moves it
  into the store and warns until the line is deleted.
- Reconnect with timers (1 s, doubling to 10 s) and `background = true`, so labels stay live without a
  key press. Events from an old socket are ignored.
- Live labels: 16 fixed keys such as `{obs.scene}` and `{obs.rec_time}` plus `{obs.mute.<input>}`, all
  set back to `no` or empty when the connection drops. The host's 256-key cap counts every key ever set, so
  the plugin tracks what it has published and stops adding new `mute.…`/`visible.…` keys at the cap.
- Bursts of scene or input events are coalesced into one reload per tick, so they cannot flood the host's
  event queue; a send that fails on a live socket is treated as a lost connection.

Its [tests](https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins/blob/main/plugins/obs/tests/plugin.rs)
show how to test a networked plugin with `Harness`: they open and close sockets, deliver messages
and fire timers directly, without a running OBS instance.

### Home Assistant

[`plugins/ha`](https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins/blob/main/plugins/ha/README.md) talks to Home Assistant over one WebSocket
with a filtered `subscribe_entities`, so only the entities listed in its settings are sent (never the whole
install), and shows them as `{ha.<entity_id>}` labels. The access token goes into the secret store like the OBS
password, and `lan = true` limits it to Home Assistant on the local network. Its "Keys for your devices" key
set makes keys for lights (on/off, dimmer, brighter), fans (on/off, slower, faster), air conditioners (on/off,
cooler, warmer, with the `climate` action), covers, media players, scripts, scenes, buttons and switches.

## Managing plugins

### In the app

The **Plugins** button in the header bar opens the plugin manager, which has two pages:

- **Discover** shows the plugin store: search, category filters, and a card for each plugin with
  Install, Update or Open. Plugins that need a newer app show "Needs Ulanzi Deck x.y.z".
- **Installed** lists installed plugins with their status and an on/off switch. Verified plugins
  have a check mark.

A plugin's page shows its status (with a banner and a Restart button when it needs attention), an
on/off switch, its `[[settings]]` fields, and **Remove…**. Removing a plugin also deletes its
secrets. Keys that use it keep their configuration and show "Needs the “<id>” plugin".

**Install Plugin…** opens a `.ulanziplugin` file and shows the plugin's details and permissions,
marked "Verified plugin" for a signed package or "Unverified plugin" otherwise. Unverified plugins
can only be installed with Developer Mode on, which also enables **Install from Folder
(developers)…** for a folder containing `plugin.toml`.

Plugin actions appear in the action list under their own category, with one section per plugin in
alphabetical order, showing the plugin's logo and each action's icon (see [Icons](#icons)). Key sets
appear at the top of the plugin's section. The key editor shows a row for each of the action's
`params`; `item` params open the plugin's list (see [Pickers](#pickers)). Keys whose plugin is
missing, turned off, failed or incompatible show a warning badge.

### From the command line

```text
ulanzictl plugin list                 id, name, version, status and the reason when there is one
ulanzictl plugin store                the plugin store's list, with what is installed or has an update
ulanzictl plugin get <id>             install or update a plugin from the store (signed: no developer mode)
ulanzictl plugin install <file|folder>  a .ulanziplugin file (signed ones without developer mode) or a folder
ulanzictl plugin remove <id>          deletes the plugin and its secrets (settings are kept)
ulanzictl plugin enable <id>          ulanzictl plugin disable <id>
ulanzictl plugin restart <id>         stop and start again; also clears a "failed" status
ulanzictl plugin dev-mode on|off
ulanzictl plugin items <id> <picker> [query] [--group G]   what a picker offers (starts the plugin)
```

Statuses: `off` (turned off), `ready` (loaded, starts on first use), `starting`, `running`, `problem`
(running, with the plugin's own message), `failed` (stopped working; restart it) and `incompatible`
(cannot be loaded; the reason follows). The same operations exist on D-Bus, see the
[technical guide](technical.md#d-bus-api). Each plugin has a `[plugins.<id>]` table in `settings.toml` with `enabled` and a
`[plugins.<id>.settings]` subtable; the subtable reaches `init` as JSON. Editing it by hand (then
`systemctl --user reload ulanzi-deckd`) is the way to change settings without the app; secret settings
are set in the app or with the plugin's own fallback (see [Secrets](#secrets)):

```toml
[plugins.hello]
enabled = true

[plugins.hello.settings]
start = 10        # whatever the plugin reads in init()
```

## Contract versions

| Version | Adds |
|---|---|
| 1.0 | Timers, state, secrets, status, logging, WebSockets, TCP/TLS and HTTP. |
| 1.1 | `list-items` and `[[pickers]]`. Components built against 1.0 still load (their `ulanzi:plugin/core@1.0.0` imports are served by 1.1) but have no lists. |
| 1.2 | Manifest only, no interface changes: plugin icons and colour, ready-made keys (`[[key_sets]]`), live looks and label values (`states`, `labels`, `track`), `show_if`, `author` and `min_app`. |

Future features will arrive as additive minor versions (1.3 and later), so existing plugins keep
working. Planned:

- Interfaces for network device discovery, sign-in flows, listening sockets, D-Bus and running
  programs.
- New exports: `widget-data` and `render` (widgets and key faces), `on-auth`, `on-discovered` and
  `on-context`.
- State-driven key faces.
- A `cargo generate` template for new plugins.
