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

Open Settings → Integrations → OBS Studio in the app and fill in the address, port and password. The password
goes straight into the plugin's secret store.

### Without the app

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
and reload again: the plugin logs a warning on every start that still finds it. Until you delete the line,
or type a password under Settings → Integrations → OBS Studio (which removes the line), the password sits
in plain text in `settings.toml` and in the local backups of it. D-Bus `GetSettings` shows it as `••••`
and settings exports leave it out. The `forget_password` action deletes the saved password;
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

## Pickers

Action params that name something in OBS are picked from a list in the app instead of typed. The lists come
from what the plugin already knows about OBS (nothing is asked of OBS when a list opens), so they need OBS to
be connected: otherwise the picker says "Not connected to OBS" and you can still type a name.

| Picker | Used by | Entries |
|---|---|---|
| `scenes` | `set_scene.scene`, `toggle_scenes.a` / `b`, `toggle_source.scene` | scenes; "Live" for the program scene, "Preview" for the preview scene |
| `audio_inputs` | `toggle_mute.input`, `input_volume.input` | inputs that have audio; "Muted" when muted |
| `scene_sources` | `toggle_source.source` | each source in each scene, grouped by scene |
| `sources` | `toggle_filter.source` | every source that appears in a scene |
| `filters` | `toggle_filter.filter` | filters, grouped by source; "On" or "Off" |
| `collections` | `set_collection.name` | scene collections; "Current" for the active one |
| `profiles` | `set_profile.name` | OBS profiles; "Current" for the active one |

Searching matches part of a name, ignoring case. Filter lists are fetched for at most 200 sources (25
requests at a time) and refresh when OBS reports a change. They use no state keys.

## Live state

Use these in any key label as `{obs.<key>}`. Yes/no values are `yes` or `no`; times are `mm:ss` or
`h:mm:ss`. When OBS is not connected the yes/no values are `no` and the names, times, stats, `mute.…` and
`visible.…` values are empty (`last_error` is empty too: going offline forgets it).

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

`visible.<scene>/<source>` is ambiguous when a scene or source name contains `/` (`a/b` + `c` and `a` +
`b/c` give the same key), and a source that appears twice in one scene shares one key (`no` if any copy is
hidden).

Limits: at most 200 `mute.…` and `visible.…` keys at a time, and the host allows a plugin 256 different
state keys for as long as it runs, with no way to remove one. Every scene, source or input name the plugin
has published counts, so renaming things or switching scene collections many times uses keys up, even
though the old ones go empty. Once all 256 are used, new names get no value and the plugin logs one
warning; disable and enable the plugin (or restart `ulanzi-deckd`) to start fresh. Keys over 256 bytes
are skipped and values over 1 KiB are cut.

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
| Key colours and icons that follow state, the wide-screen widget, `ObsList`, D-Bus `ObsStatus` | Planned for later plugin milestones |

## Tests

`cargo test -p ulanzi-plugin-obs` runs the plugin natively against `Harness` ([tests/plugin.rs](tests/plugin.rs));
`cargo test -p ulanzi-deckd --test obs_plugin` runs the built wasm in the host against a mock OBS.
