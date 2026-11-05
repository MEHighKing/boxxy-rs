# boxxy

boxxy is a small terminal frame. It takes over the screen, draws a border that does not scroll, and runs your shell in the interior. The shell is a normal process on a PTY, so vim, neovim, htop, and the rest keep working. Their output is parsed and painted only inside the frame.

boxxy is not a terminal emulator you configure, a multiplexer, a session manager, or a plugin host. It draws a frame and hosts a shell.

## Run

```sh
cargo build --release
./target/release/boxxy --generate-config
./target/release/boxxy
```

The shell is `$SHELL` when that file exists, otherwise `/bin/sh`. Leave boxxy by exiting the shell (`exit` or Ctrl-D). Ctrl-C is delivered to the shell, not intercepted.

Configuration is only `~/.config/boxxycli/boxxy.toml`. `XDG_CONFIG_HOME` and other environment variables are not read. `--generate-config` creates the file and refuses to replace it unless you also pass `--force`.

```toml
[frame]
thickness = 1
radius = "rounded"
color = "#89DCEB"

[brand]
text = "BOXXY"
```

| Key | Values |
| --- | --- |
| `frame.thickness` | `1` or `2` |
| `frame.radius` | `"square"` or `"rounded"` |
| `frame.color` | `#RRGGBB` |
| `brand.text` | up to 32 characters |

Unknown keys, missing keys, and invalid values stop startup. Nothing is rewritten or defaulted at runtime.

```
Error:
Unknown configuration key: frame.padding
```

If the file is absent:

```
No configuration found.
Run 'boxxy --generate-config' to create one.
```

Thickness `1` is a light stroke. Thickness `2` keeps the same one-cell inset and uses a heavier box-drawing stroke, so the shell size does not change when you switch weight.

```
╭─ BOXXY ────────────────╮
│                        │
╰────────────────────────╯
```

```
+- BOXXY ----------------+
|                        |
+------------------------+
```

```
╔═ BOXXY ════════════════╗
║                        ║
╚════════════════════════╝
```

## Develop

Rust 1.88 or newer.

```sh
cargo test
cargo run -- --generate-config
cargo run
```

Dependencies are crossterm, ratatui, portable-pty, vt100, and toml. vt100 is the screen parser that keeps full-screen programs inside the border.
