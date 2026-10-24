# OBS Studio plugin

Controls OBS Studio from your keys and shows its live state on key labels. It is the built-in OBS
integration rewritten as a sandboxed plugin (obs-websocket 5, OBS 28 or newer). It runs beside the
built-in integration: obs-websocket accepts several clients, and the label variables do not clash
(`{obs_scene}` is the built-in one, `{obs.scene}` is this plugin's).

The plugin only talks to OBS on this computer (`network = ["127.0.0.1", "localhost"]`). If OBS is not
running it retries quietly (1 s, doubling to 10 s) and the status says what to do.

## Build and install

```sh
cargo build --release --target wasm32-wasip2 -p ulanzi-plugin-obs
mkdir -p ~/obs-plugin
cp plugins/obs/plugin.toml ~/obs-plugin/
cp target/wasm32-wasip2/release/ulanzi_plugin_obs.wasm ~/obs-plugin/plugin.wasm
ulanzictl plugin dev-mode on
ulanzictl plugin install ~/obs-plugin
```

In OBS, turn on Tools → WebSocket Server Settings → Enable WebSocket server. Show Connect Info gives the
port and password.

## Settings

Edit `~/.config/ulanzi-deck/settings.toml`, then run `systemctl --user reload ulanzi-deckd`:

```toml
[plugins.obs]
enabled = true

[plugins.obs.settings]
host = "localhost"      # or "127.0.0.1"; anything else is refused
port = 4455
password = "..."        # only for the first start, see below
```

The password is saved in the system keyring on the first start. After that, delete the `password` line
and reload again: the plugin logs a warning on every start that still finds it, and
`ulanzictl export --settings` does not strip it. The `forget_password` action deletes the saved password;
`ulanzictl plugin remove obs` does too.

## Actions

Use them in `profile.toml` as `type = "plugin"`, `plugin = "obs"`, `action = "<id>"`, `params = { ... }`.

| Action | Params |
|---|---|
| `set_scene` | `scene`; `preview = true` sets the preview scene (studio mode) |
| `toggle_scenes` | `a`, `b`: switch to `b` when `a` is live, otherwise to `a` |
| `toggle_source` | `scene`, `source`: show or hide |
| `toggle_record`, `start_record`, `stop_record`, `pause_record` | none (`pause_record` pauses or resumes) |
| `toggle_stream`, `start_stream`, `stop_stream` | none |
| `toggle_replay`, `save_replay` | none |
| `toggle_virtual_cam` | none |
| `toggle_mute` | `input` |
| `input_volume` | `input`, `op` = `up` / `down` / `mute` / `set`; `percent` (0-100, for `set`); `step` in dB (1-20, default 2) |
| `toggle_filter` | `source`, `filter` |
| `studio_transition` | none |
| `set_collection` | `name` |
| `set_profile` | `name` |
| `screenshot` | none |
| `reconnect` | none |
| `forget_password` | none |

```toml
[pages.keys.7.look]
label = { text = "{obs.rec_time}", visible = true }

[pages.keys.7.tap]
type = "plugin"
plugin = "obs"
action = "toggle_record"

[pages.keys.8.look]
label = { text = "{obs.scene}", visible = true }

[pages.keys.8.tap]
type = "plugin"
plugin = "obs"
action = "set_scene"
params = { scene = "Gameplay" }
```

## Live state

Use these in any key label as `{obs.<key>}`. Yes/no values are `yes` or `no`; times are `mm:ss` or
`h:mm:ss`. When OBS is not connected every value except `connected` is empty.

| Key | Value |
|---|---|
| `connected` | `yes` / `no` |
| `scene`, `preview` | live scene; preview scene (studio mode only) |
| `studio_mode` | `yes` / `no` |
| `scene_count` | number of scenes |
| `recording`, `rec_paused`, `streaming`, `replay`, `vcam` | `yes` / `no` |
| `rec_time`, `stream_time` | time running; empty when the output is off |
| `fps`, `cpu`, `dropped` | from OBS stats, refreshed every 2 s; `dropped` is a percentage like `0.4%` |
| `last_error` | OBS's refusal of the last action, empty after the next success |
| `mute.<input>` | `yes` / `no` for each audio input |
| `visible.<scene>/<source>` | `yes` / `no` for each source in a scene |

There are at most 200 `mute.…` and `visible.…` keys; beyond that the plugin logs one warning and leaves
the rest out.

## Not in this plugin (yet)

| Built-in feature | Why not |
|---|---|
| Find OBS settings, auto-detect, enable server | Plugins have no file access |
| Telling "not installed", "not running", "server off" and "wrong port" apart | The plugin only sees that the connection failed: one combined message |
| Connecting the moment OBS starts | No process watching: retries at most every 10 s |
| OBS on another computer | Network permission is local only |
| Profiles following scene or input renames | Plugins cannot edit profiles |
| OBS's refusal as the key's error | An action returns before OBS answers: see `{obs.last_error}` and the log |
| Wrong-password message | The host does not pass the close code, so a close during sign-in reads "OBS closed the connection during sign-in" |
| Key colours and icons that follow state, scene and input pickers, the wide-screen widget, `ObsList`, D-Bus `ObsStatus` | Planned for later plugin milestones |

## Tests

`cargo test -p ulanzi-plugin-obs` runs the plugin natively against `Harness` ([tests/plugin.rs](tests/plugin.rs));
`cargo test -p ulanzi-deckd --test obs_plugin` runs the built wasm in the host against a mock OBS.
