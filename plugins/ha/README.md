# Home Assistant plugin

Toggles, runs and sets Home Assistant entities from your keys, calls any service, and shows live entity state
on key labels. It is a sandboxed plugin (contract 1.0) and talks to Home Assistant over its WebSocket API.
It needs Home Assistant 2022.4 or newer.

The plugin only talks to Home Assistant on your local network (`lan = true`). That allows `localhost`, names
ending in `.local`, local-range IP addresses (127/8, 10/8, 172.16/12, 192.168/16, 169.254/16, `::1`,
`fe80::/10`, `fc00::/7`) and any name that resolves only to local addresses. A name that resolves to even one
public address is refused (`not allowed: <host>`). The permission does not limit which local host or port the
plugin dials: it connects to whatever you type in `url`. If Home Assistant is not reachable it retries quietly
(1 s, doubling to 10 s) and the status says what to check. After "Home Assistant 2022.4 or newer needed." or
"Home Assistant must be on your local network …" it does not retry by itself: change a setting or run the
`reconnect` action. The log records why a connection failed (DNS, TLS, message too large), once per reason.

## Build and install

```sh
cargo build --release --target wasm32-wasip2 -p ulanzi-plugin-ha
mkdir -p ~/ha-plugin
cp plugins/ha/plugin.toml ~/ha-plugin/
cp target/wasm32-wasip2/release/ulanzi_plugin_ha.wasm ~/ha-plugin/plugin.wasm
ulanzictl plugin dev-mode on
ulanzictl plugin install ~/ha-plugin
```

Create a long-lived access token in Home Assistant: open your profile page (your name, bottom left), scroll
to the bottom, "Long-lived access tokens", Create token, and copy it. Home Assistant shows it only once.

## Settings

Edit `~/.config/ulanzi-deck/settings.toml`, then run `systemctl --user reload ulanzi-deckd`:

```toml
[plugins.ha]
enabled = true

[plugins.ha.settings]
url = "homeassistant.local:8123"
token = "..."                                   # only for the first start, see below
entities = ["light.desk", "sensor.temp"]        # up to 40
attributes = ["climate.living:current_temperature"]   # optional, up to 40
```

| Setting | Meaning |
|---|---|
| `url` | Required. `192.168.1.5`, `homeassistant.local:8123`, `http://host:8123`, `https://host` (`ws://` and `wss://` work too, and a pasted `/api/websocket` ending is removed). No scheme means `http`; no port means 8123 for http and 443 for https; a path is kept as a prefix (`/ha` becomes `/ha/api/websocket`). Credentials, a query or a fragment are refused. |
| `token` | The long-lived access token. |
| `entities` | The entities whose state you want on key labels. A list, or one comma-separated string. Ids are lowercase (`light.desk`); an entry that is not a valid entity id is skipped with a warning. |
| `attributes` | Extra attribute values as `entity:attribute`. Optional. |

Only the listed entities are subscribed (Home Assistant sends just those, never the whole install). Actions can
target any entity id, listed or not. An entity used only in `attributes` is added to the subscription and
counts toward the 40-entity limit.

### The token

The token is saved in the system keyring on the first start. After that, delete the `token` line and reload
again. The plugin logs a warning on every start that still finds it. Until you delete the line, the token sits
in plain text in `settings.toml`, can be read over D-Bus (`GetSettings`) and appears in settings exports and
snapshots (`ulanzictl export --settings` does not strip it). The `forget_token` action deletes the saved token
(delete the `token` line first, otherwise the next start saves it again);
`ulanzictl plugin remove ha` does too.

With an `http://` address (the default) or `ws://` the token travels unencrypted on your local network. That is
how most Home Assistant installs are reached; use `https://` if your certificate is valid for the name you
type.

## Actions

Use them in `profile.toml` as `type = "plugin"`, `plugin = "ha"`, `action = "<id>"`, `params = { ... }`.
An action returns at once; Home Assistant's answer arrives later, and a refusal shows up in `{ha.last_error}` and
the log.

| Action | Params | Home Assistant service |
|---|---|---|
| `toggle`, `turn_on`, `turn_off` | `entity_id` | `homeassistant.toggle` / `turn_on` / `turn_off` |
| `run` | `entity_id` (a scene, script, automation, button or input_button) | `scene.turn_on`, `script.turn_on`, `automation.trigger`, `button.press`, `input_button.press` |
| `set_level` | `entity_id`; `op` = `set` (default) / `up` / `down`; `percent` (0-100, for `set`); `step` (1-50, default 5) | see below |
| `call_service` | `domain`, `service`, optional `entity_id`, optional `data` (a JSON object, as a table or a JSON string, up to 4096 bytes) | `<domain>.<service>` with the entity as target |
| `reconnect` | none | |
| `forget_token` | none | |

`set_level` by domain:

| Domain | `set` | `up` / `down` |
|---|---|---|
| `light` | `light.turn_on` with `brightness_pct` | `light.turn_on` with `brightness_step_pct` (plus or minus `step`) |
| `media_player` | `media_player.volume_set` (35 becomes 0.35) | `volume_up` / `volume_down` (`step` ignored) |
| `fan` | `fan.set_percentage` | `fan.increase_speed` / `decrease_speed` with `percentage_step` |
| `cover` | `cover.set_cover_position` | `set_cover_position` from the tracked position plus or minus `step`; the cover must be listed under `entities` |

A dial turn is an ordinary action in contract 1.0: assign `set_level` with `op = "up"` or `"down"` to it.

```toml
[pages.keys.7.look]
label = { text = "{ha.light.desk}", visible = true }

[pages.keys.7.tap]
type = "plugin"
plugin = "ha"
action = "toggle"
params = { entity_id = "light.desk" }

[pages.keys.8.look]
label = { text = "{ha.sensor.temp}{ha.sensor.temp.unit}", visible = true }

[pages.keys.9.look]
label = { text = "{ha.light.desk.pct}%", visible = true }

[pages.keys.9.tap]
type = "plugin"
plugin = "ha"
action = "set_level"
params = { entity_id = "light.desk", op = "up", step = 10 }
```

## Live state

Use these in any key label as `{ha.<key>}`.

| Key | Value |
|---|---|
| `connected` | `yes` / `no` |
| `version` | Home Assistant's version; empty when offline |
| `last_error` | Home Assistant's refusal of the last action or subscription, empty after the next success |
| `entity_count` | how many of the listed entities Home Assistant knows |
| `<entity_id>` | the state (`on`, `off`, `21.5`, ...); `offline` when not connected; `missing` when Home Assistant has no such entity; the keys of an entity you removed from `entities` stay and show `offline` until the plugin restarts |
| `<entity_id>.name` | friendly name |
| `<entity_id>.unit` | unit of measurement |
| `<entity_id>.pct` | 0-100 whole number from light brightness, media_player volume, fan percentage or cover position; empty for other entities |
| `<entity_id>.attr.<attribute>` | the attributes you listed under `attributes`; numbers without a trailing `.0`, structures as compact JSON |

When Home Assistant is not connected, `connected` is `no`, every entity state is `offline`, and all other
values are empty (`last_error` keeps its last text). Values over 1 KiB are cut.

Key budget: the host allows a plugin 256 different state keys for as long as it runs, with no way to remove
one. The plugin uses 4 fixed keys, 4 per listed entity and 1 per attribute: at most 40 entities and 40
attributes, so at most 204 keys. Every key the plugin has published counts, so editing `entities` many times
in one run uses keys up even though the old ones go empty. Once all 256 are used, existing keys keep
updating, new ones get no value and the plugin logs one warning. Disable and enable the plugin (or restart
`ulanzi-deckd`) to start fresh. Keys over 256 bytes are skipped.

## Not in this plugin (yet)

| Item | Why / when |
|---|---|
| Discovery (`homeassistant.local` probe, mDNS) | no discovery interface in contract 1.0; a later 1.x |
| Browser sign-in (OAuth), token refresh | needs the `auth` and `listener` interfaces (later contract) |
| Item picker with areas, icons and pictures; "What should this key do?" dialog | GUI building blocks (plugin milestone 2) and a `list-items` export |
| Entity registry, areas, device names, following renames | would need the large `config/entity_registry/list` and pickers; only the friendly name is used |
| State-driven key colours and icons, key faces, widgets (tile, climate, group), sparkline | key faces and widget templates are later contract versions |
| Dial adjust | contract 1.0 only has `on-action`; a dial turn works as `set_level` with `op = up` / `down` |
| Nabu Casa and other HTTPS remote URLs | `wss` to a public host needs a `network` entry you cannot edit yet; only `lan = true` ships. HTTPS on your LAN works if the certificate is valid for the name you use |
| Home Assistant's error as the key's error | the action returns before Home Assistant answers: see `{ha.last_error}` and the log |
| Service responses, `get_states`, `fire_event`, templates | not needed for control and live labels |
| Camera pictures, entity images | need `http` and a canvas |
| Marketplace listing, screenshots, polish | marketplace milestone |

## Tests

`cargo test -p ulanzi-plugin-ha` runs the plugin natively against `Harness` ([tests/plugin.rs](tests/plugin.rs))
and tests the protocol model ([tests/protocol.rs](tests/protocol.rs));
`cargo test -p ulanzi-deckd --test ha_plugin` runs the built wasm in the host against a mock Home Assistant.
