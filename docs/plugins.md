# Writing plugins

A plugin adds actions to Ulanzi Deck without changing the app: a counter, a chat bot, a smart-home
bridge. It is a small WebAssembly program that the service (`ulanzi-deckd`) runs in a sandbox. It can do
only what its manifest asks for and you allow. For how the rest of the app is built, see the
[technical guide](technical.md).

**Contents**
[What a plugin is](#what-a-plugin-is) ·
[The contract](#the-contract) ·
[The manifest](#the-manifest) ·
[Permissions](#permissions) ·
[The SDK](#the-sdk) ·
[Limits](#limits) ·
[Secrets](#secrets) ·
[Building and installing](#building-and-installing) ·
[Example: hello](#example-hello) ·
[The OBS Studio plugin](#the-obs-studio-plugin) ·
[The Home Assistant plugin](#the-home-assistant-plugin) ·
[Managing plugins](#managing-plugins) ·
[Later contract versions](#later-contract-versions)

---

## What a plugin is

A folder with three things:

```text
plugin.toml     name, version, permissions and the actions it provides
plugin.wasm     the program (a WebAssembly component)
assets/         optional: pictures and other files for the plugin
```

Installed plugins live in `~/.local/share/ulanzi-deck/plugins/<id>/` (it follows `XDG_DATA_HOME`).
The service compiles each `plugin.wasm` once and keeps the result in `~/.cache/ulanzi-deck/plugins/`.

The sandbox is strict:

- No files, an empty environment (no variables are set), no `wasi:sockets`, no `wasi:http`. Standard output and error go to the
  service log. Clocks and random numbers work.
- Every connection goes through the service, which checks it against the plugin's permissions first.
- The plugin gets 64 MiB of memory and 500 ms for each call. It cannot run in the background between calls
  except through timers and sockets the service owns.
- A plugin that crashes, runs out of memory or takes too long is stopped. The deck and other plugins
  keep working.

A plugin starts the first time one of its actions is pressed, or when the daemon starts if a profile
uses it or its manifest says `background = true`. The deck never waits for plugins: they load in the
background after it starts, and a plugin key pressed before that reports "plugins are still starting".

## The contract

The interface between plugin and service is written in WIT:
[`crates/ulanzi-plugin/wit/plugin.wit`](../crates/ulanzi-plugin/wit/plugin.wit), package
`ulanzi:plugin@1.1.0`. The SDK wraps it, so most plugins never touch it directly.

What the service calls in your plugin (the exports):

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

What your plugin can call (the imports):

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

Calls never wait. `ws-open`, `tcp-open` and `fetch` return an id at once; the answer comes later as an
event (`on-open`, `on-closed`, `on-http`). Events for one plugin arrive one at a time, in order.
A dial turn is an ordinary `on-action` for the action assigned to that direction (left, right, press
or hold).

### Pickers

Contract 1.1 adds `list-items`, so the user picks a scene or an entity from a list instead of typing
its name. Declare each list in the manifest and point `item` fields at it:

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

Answer `list-items` from what the plugin already knows (the scenes it got when it connected, the entity
states it keeps up to date). Never wait for the network in it: it has the same 500 ms budget as every
call, and the app waits for the answer. The service starts a plugin that is not running yet when its
picker opens. It keeps at most 5000 entries and drops entries whose id is empty or longer than 1024
bytes. It cuts `label`, `subtitle`, `group` and `state` to 256 bytes, `icon` to 128 and an error text to
4096. A picker the manifest does not declare is refused before the plugin is asked. A plugin built for
1.0 has no `list-items`: its pickers answer "<name> has no list to pick from".

To offer different lists to different actions (say lights for Toggle and scripts for Run), declare one
picker for each.

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
| `description`, `icon`, `categories`, `tested_on_hardware` | Optional, for the app's lists. |
| `background` | Optional, default `false`. |
| `[permissions]` | Optional, see below. Nothing is allowed that is not listed. |
| `[[settings]]` | Optional. The plugin's settings page in the app, one field each; see [Settings and parameters](#settings-and-parameters). |
| `[[actions]]` | The actions the plugin offers: `id` and `name` (required, ids unique), `icon`, and `params` (fields, as below). |
| `[[pickers]]` | Contract 1.1. Lists that `item` fields are picked from: `id` (required, unique), `style`, `groups`, `multi`; see [Pickers](#pickers). |

A key uses an action like this in `profile.toml`:

```toml
[pages.keys.1.tap]
type = "plugin"
plugin = "hello"
action = "greet"
params = { name = "Ana" }
```

Values in `params` and in `[plugins.<id>.settings]` that have no JSON form (dates and times) reach the plugin as strings.

**Contract versions.** `contract = "1.0"` loads if the major version equals the service's (1) and the
minor version is not newer than the service's (1 today). A plugin that asks for more shows as
`incompatible` and the message "needs a newer Ulanzi Deck". Minor versions only add things, so a plugin
written for 1.0 keeps working on every later 1.x.

The manifest may also contain tables this version does not read yet (key faces, widgets, variables,
detection, sign-in). They are accepted and ignored. A manifest is refused when a picker is declared
twice, when an `item` field names a picker that is not declared, or when an action param uses a `multi`
picker.

### Settings and parameters

`[[settings]]` entries and the entries of an action's `params` are fields. Each has a `key` (required,
unique in its list), a `kind` (default `text`), and optionally `label` (the row's title; the key when
missing), `help` (shown under the row or as its tooltip), `default`, `min`, `max`, `step` and `choices`.
The app builds one row per field: the plugin's page under Settings → Integrations for `[[settings]]`, and
the rows under the action in the key editor for `params`.

| `kind` | Row in the app | Value written |
|---|---|---|
| `text` | Text entry, saved with its apply button | string; an empty entry removes the key |
| `number` | Number with − and + (`min`, `max`, `step`) | integer when at least one of `min`, `max`, `step`, `default` is given and all given are whole numbers, else a float |
| `toggle` | Switch | boolean |
| `choice` | Drop-down of `choices` | the chosen value (string) |
| `color` | Colour button | `"#rrggbb"` |
| `json` | Text entry that must hold valid JSON | the JSON text, as a string |
| `item` | With a `picker`: the picked entry, from the plugin's list (contract 1.1, see [Pickers](#pickers)). Without one: a text entry | the entry's id (string); a list of ids for a `multi` picker |
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

Without permissions a plugin can compute, keep state, use timers and secrets, and nothing else.

| Permission | Allows |
|---|---|
| `network = ["host", …]` | Connections to exactly these host names. `*.example.com` matches every subdomain (`a.example.com`, `a.b.example.com`) but not `example.com` itself; list both if you need both. A wildcard needs at least two labels after the `*.`, so `*.com` is refused. Names are not case sensitive. |
| `lan = true` | Connections to the local network: `localhost`, names ending in `.local`, IP addresses in local ranges, and any other name that resolves only to local addresses. |

Local ranges are loopback (`127.0.0.0/8`, `::1`), private IPv4 (`10.0.0.0/8`, `172.16.0.0/12`,
`192.168.0.0/16`), link-local (`169.254.0.0/16`, `fe80::/10`) and IPv6 unique-local (`fc00::/7`).
An IPv4 address written as IPv6 (`::ffff:192.168.1.5`) is judged as the IPv4 address.

How the checks work:

- The service checks a name before connecting. For `lan`, it resolves the name once and connects to
  exactly the addresses it checked, so a name cannot answer "local" for the check and "public" for the
  connection. A name that resolves to even one public address is refused under `lan`.
- A host in the `network` list is allowed whatever it resolves to.
- HTTP redirects are not followed. A 3xx response goes to the plugin, which can then ask again. A
  plugin therefore cannot be sent to a host outside its permissions.
- The error for a refused connection is `not allowed: <host>`.

`browser`, `dbus` and `programs` are accepted in the manifest and have no effect yet; they arrive with
the contract versions that use them.

Permissions in this version are granted by installing. The Settings page that shows them before you
install is planned.

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

`Cx` is your handle to the host: `set_timer`, `cancel_timer`, `set_state`, `secret`, `set_secret`,
`delete_secret`, `set_status`, `log`, `ws_open`, `tcp_open`, `send`, `close` and `fetch`. It holds no
data; every call goes straight to the host.

### Testing without a deck

On your own machine (not WebAssembly) the same calls go to a fake host, so plugin logic runs as an
ordinary `cargo test`. `ulanzi_plugin::testing::Harness` drives your plugin and shows what it did:

- Drive it: `init`, `action`, `list_items`, `open`, `message`, `closed`, `http`, `fire` (a timer).
- Look at the result: `state`, `status`, `logs`, `timers`, `opened` (sockets the plugin opened),
  `sent` (what it wrote to a socket), `fetched` (requests it made) and `secrets` (read or preload them).

The Harness does not enforce permissions or limits; try the plugin in the real service for those.

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

Use `secret-set` for passwords, tokens and keys, never `set-state` or a settings value. Secrets are
stored for you in the system keyring (Secret Service: KWallet, GNOME Keyring), which the service
first asks when a plugin starts. If it does not answer within 3 seconds, they go to
`~/.config/ulanzi-deck/plugin-secrets.toml` (mode 0600) instead. They are never logged, never sent from the service over D-Bus (the app sends them in with `PluginSetSecret`) and not part of profile exports or
backups. Removing a plugin deletes its secrets; reinstalling it from a folder keeps them.

To let the user type a secret in the app, declare it as a `kind = "secret"` setting. Its `key` is the
secret's name in the store, so the plugin reads it with `secret("<key>")`. The row is write-only: the
app never shows the value, only `<label> · saved` and a Clear button once one is stored. Saving a new
value writes it straight into the plugin's secret store, removes the same key from
`[plugins.<id>.settings]` in `settings.toml` (so an old plain-text copy cannot win) and restarts the
plugin with the new value; a plugin that is turned off or stopped working is not started. Over D-Bus, `GetSettings` shows a secret setting's value as `••••`, and
settings exports leave it out.

Without the app, a plugin can still take a secret from its settings table, as the OBS and Home Assistant
plugins do: `init` finds `password` or `token` in `[plugins.<id>.settings]`, saves it with `set_secret`
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
ulanzictl plugin dev-mode on        # once: allow installing from a folder
ulanzictl plugin install ./hello    # install, or replace and restart if already installed
ulanzictl plugin list
```

Installing from a folder is refused with "Turn on Developer mode to install plugins from a folder"
until developer mode is on. The installer checks `plugin.toml` and compiles `plugin.wasm` before
copying. The source folder can have any name: the plugin is installed in a folder named after the `id`
in its `plugin.toml`. The service tells you in `plugin list` if anything is wrong. A broken plugin never stops the others:
it is listed as `incompatible` with the reason.

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
scripts/package-plugin.sh plugins/obs     # dist/plugins/obs-1.0.0.ulanziplugin
ulanzictl plugin install dist/plugins/obs-1.0.0.ulanziplugin
```

Packages are not signed, so installing one needs Developer mode, like installing from a folder
("Turn on Developer mode to install unverified plugins"). Inspecting one (the check an installer
shows before it asks) works without. The service unpacks the file into a hidden temporary folder next
to the installed plugins, checks it like a plugin folder and installs it; the temporary folder is
removed whatever the outcome. A package is refused, with the reason, when:

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

## The OBS Studio plugin

[`plugins/obs`](../plugins/obs/README.md) is a larger, real-world plugin: the OBS integration as a sandboxed
plugin. It shows what the small hello example leaves out:

- A WebSocket client (`network = ["127.0.0.1", "localhost"]`) that signs in with obs-websocket 5 and
  never blocks: every handler sends and returns, answers arrive as events.
- The password in the secret store. The user types it under Settings → Integrations → OBS Studio, which
  writes it into the store. A `password` line under `[plugins.obs.settings]` still works: `init` moves it
  into the store and warns until the line is deleted.
- Reconnect with timers (1 s, doubling to 10 s) and `background = true`, so labels stay live without a
  key press. Events from an old socket are ignored.
- Live labels: 16 fixed keys such as `{obs.scene}` and `{obs.rec_time}` plus `{obs.mute.<input>}`, all
  set back to `no` or empty when the connection drops. The host's 256-key cap counts every key ever set, so
  the plugin tracks what it has published and stops adding new `mute.…`/`visible.…` keys at the cap.
- Bursts of scene or input events are coalesced into one reload per tick, so they cannot flood the host's
  event queue; a send that fails on a live socket is treated as a lost connection.

Its tests ([`plugins/obs/tests/plugin.rs`](../plugins/obs/tests/plugin.rs)) are a model for `Harness` tests of a
plugin with a network connection: they open and close sockets, deliver messages and fire timers by hand,
with no real OBS and no waiting.

## The Home Assistant plugin

[`plugins/ha`](../plugins/ha/README.md) is the second real plugin. It talks to Home Assistant over one WebSocket
with a filtered `subscribe_entities`, so only the entities listed in its settings are sent (never the whole
install), and shows them as `{ha.<entity_id>}` labels. The access token goes into the secret store like the OBS
password, and `lan = true` limits it to Home Assistant on the local network.

## Managing plugins

In the app: Settings → Integrations lists the installed plugins with their status and an on/off switch.
Each plugin's page shows its status (with a banner and Restart when it needs attention or stopped
working), Turned On, Restart, its settings from `[[settings]]`, and Remove… (which also deletes its
secrets; keys that use it keep their settings and show "Needs the “<id>” plugin"). With Developer Mode on,
Install from Folder… installs a plugin folder after showing its permissions. Plugin actions appear in the
key editor's action picker, one section per plugin, with rows for their `params`; keys whose plugin is
missing, turned off, stopped working or cannot be loaded show a warning badge.

From the command line:

```text
ulanzictl plugin list                 id, name, version, status and the reason when there is one
ulanzictl plugin install <file|folder>  a .ulanziplugin file or a plugin folder; needs developer mode
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

## Later contract versions

Contract 1.0 is the base: timers, state, secrets, status, logging, WebSockets, TCP/TLS and HTTP.
Contract 1.1 adds `list-items` and `[[pickers]]`. A component built against 1.0 still loads (its
`ulanzi:plugin/core@1.0.0` imports are served by 1.1), it just has no lists. More arrives as minor
versions (1.2, …) when the features that need them are built, and they only add, so older plugins keep
working:

- Interfaces for discovering devices on the network, sign-in flows, listening for connections, D-Bus
  and running programs.
- New exports: `widget-data` and `render` (widgets and key faces), `on-auth`, `on-discovered` and
  `on-context`.
- State-driven key faces in the app.
- A single-file package format, the marketplace and a `cargo generate` starter. Until then, plugins are
  installed from a folder in developer mode.
