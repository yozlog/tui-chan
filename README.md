# tui-chan

An Imageboard Terminal User Interface.
Currently supports only 4chan.

![demo](docs/demo.gif)

## Installation
Download the [latest release][latest-releases]. The binary executable is `tui-chan`. Put it in your PATH so that you can execute it from everywhere.

Then run it from the command line.
```shell
tui-chan
```

An imageboard name may be specified as an argument, the default one is `4chan`.

## Building from source
If your architecture is not supported by the pre-built binaries you can build the application from the source code yourself.
Make sure you have [Rust][rust-installation-url] installed.

```shell
git clone https://github.com/tuqqu/tui-chan.git
cd tui-chan
cargo install --path . # copies binary to /.cargo/bin/

# to uninstall run
cargo uninstall
```

## Configuration

Settings can be configured in `~/.config/tui-chan/settings.conf`

- `render_images`: Master switch to enable terminal image previews (`true` or `false`). Defaults to `false`. This setting can be toggled at runtime using the `toggle_image_previews` keybind listed in the controls table below.
- `image_layout`: Layout mode for image rendering. Evaluated only if `render_images` is set to `true`:
  - `inline` (Default): Displays 4chan-style left thumbnails inside the thread and post lists. Automatically collapses vertical space to fit the actual thumbnail size and uses a clean pointer (`▶ `) to highlight selections.
  - `split`: Splits the active panel horizontally and displays a larger image preview on the right (60% list, 40% preview).
  - `hybrid`: Combines both inline list thumbnails and right-side split previews simultaneously.
- `image_renderer`: Terminal graphics protocol/backend. Evaluated only if `render_images` is set to `true`:
  - `unicode` (Default): Uses character half-blocks (`▄`) to render low-resolution pixelated previews. Works in any terminal.
  - `iterm2`: Uses the iTerm2 inline image protocol to display high-resolution lossless previews. Supported by iTerm2, WezTerm, and other compatible terminals.
  - `kitty`: Uses the Kitty graphics protocol to display high-resolution lossless previews. Supported by Ghostty, WezTerm, Kitty, and modern iTerm2.
> [!NOTE]
> The below two settings are highly beneficial for Vim users to leverage prefix count multipliers (e.g. `3j`, `5k`) for efficient, rapid list navigation. If you are a Vim user, you can also customize `keybinds.conf` to map standard navigation from `wasd` to `hjkl` (and map quick navigation keys to `Ctrl` + `hjkl` accordingly).

- `board_line_numbers`: Enable/disable line numbers in the board list (`true` or `false`). Defaults to `false`.
- `board_relative_line_numbers`: Enable/disable relative line numbers in the board list (`true` or `false`). Evaluated only if `board_line_numbers` is set to `true`. Defaults to `false`.

## Controls

Controls can be configured in `~/.config/tui-chan/keybinds.conf`

### Default controls

Press `h` to show / hide help bar to look up controls.
Use `d` to open board or thread and `a` to return to the previous panel.

| Description                                          | Keys                          |
|------------------------------------------------------|-------------------------------|
| Move around                                          | `w`,`a`,`s`,`d`               |
| Move quickly                                         | control + `w`,`a`,`s`,`d`     |
| Toggle image previews                                | `i`                           |
| Toggle help bar                                      | `h`                           |
| Next page                                            | `p`                           |
| Previous page                                        | control + `p`                 |
| Reload page                                          | `r`                           |
| Toggle fullscreen for the selected panel             | `z`                           |
| Copy the direct url to the selected thread or post   | `c`                           |
| Copy the selected post media (image/webm) url        | control + `c`                 |
| Open the selected thread or post in browser          | `o`                           |
| Open the selected post media (image/webm) in browser | control + `o`                 |
| Quit                                                 | `q`                           |

[latest-releases]: https://github.com/tuqqu/tui-chan/releases
[rust-installation-url]: https://www.rust-lang.org/tools/install
