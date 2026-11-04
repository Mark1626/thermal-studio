# thermal-studio

Open-source tools for printing Markdown notes and photos on the **Seznik MiniX** pocket thermal printer (a rebadged
LuckJingle S1 using 58 mm paper). It includes a desktop app, a command-line tool and an Android app, all built on the
same Rust core.

For how the printer's Bluetooth protocol was worked out, see [`RESEARCH.md`](RESEARCH.md).

## Getting started

You need a recent Rust toolchain (1.85 or newer).

### Desktop app

```sh
cargo run --release -p thermal-studio              # opens with a sample note
cargo run --release -p thermal-studio -- photo.jpg # or open an image (drag and drop also works)
```

Write Markdown or load a photo, adjust it with a live preview, then press **Print**.

### Command line

```sh
# Save a preview as PNG
cargo run --release -p thermal-cli -- markdown examples/sample.md -o note.png
cargo run --release -p thermal-cli -- image photo.jpg -o preview.png

# Print over Bluetooth
cargo run --release -p thermal-cli -- markdown note.md --print
cargo run --release -p thermal-cli -- image photo.jpg --print

# List nearby printers and the available dithering algorithms
cargo run --release -p thermal-cli -- printers
cargo run --release -p thermal-cli -- algorithms
```

Images use the `photo` preset by default. Use `--preset line-art` for logos and text, or `--preset sketch` for a
pen-drawing look. Run any command with `--help` to see all options.

### Android

See [`android/README.md`](android/README.md).

## Printing

Turn the printer on. There's no need to pair it. The apps only connect while they're printing and disconnect
afterwards, so the printer stays free for other devices.

- **Desktop app:** the first time you press **Print**, pick your printer from the list. It's remembered for next
  time. Density and paper feed are under the **Printer** menu.
- **Command line:** add `--print` to any `image` or `markdown` command. Use `--printer <name>` if yours isn't called
  `MiniX`, and `--density light|normal|dark` to change how dark it prints.

The printer only accepts one connection at a time, so close the phone app (or anything else connected to it) first.

On macOS, run the tools from a terminal that has Bluetooth access.

## Project layout

| Path | What it is |
|---|---|
| [`apps/thermal-studio`](apps/thermal-studio) | Desktop app ([egui](https://github.com/emilk/egui)) |
| [`crates/thermal-cli`](crates/thermal-cli) | The `thermal` command-line tool |
| [`android/`](android) | Android app (Jetpack Compose) |
| [`crates/thermal-raster`](crates/thermal-raster) | Image preparation and dithering |
| [`crates/thermal-markdown`](crates/thermal-markdown) | Markdown rendering, with bundled English and Tamil fonts |
| [`crates/thermal-proto`](crates/thermal-proto) | Printer protocol and print flow (no I/O) |
| [`crates/thermal-ble`](crates/thermal-ble) | Bluetooth LE connection ([btleplug](https://github.com/deviceplug/btleplug)) |
| [`crates/thermal-ffi`](crates/thermal-ffi) | Kotlin/Swift bindings ([UniFFI](https://mozilla.github.io/uniffi-rs/)) used by the Android app |

### How it fits together

- **One image format throughout.** Everything renders to a 1-bit bitmap stored exactly as the printer expects it,
  so it can be sent without conversion.
- **One print flow everywhere.** `thermal-proto` describes each step of a print job without doing any I/O itself.
  The desktop tools and the Android app just carry out those steps over their own Bluetooth stacks.
- **Same output on every machine.** Fonts are bundled, so a note looks the same wherever it's rendered.

### Dithering

Thermal paper smears single isolated dots into grey, so the defaults favour clean, connected marks:

- **Photo** (default): Atkinson dithering with a slight gamma boost to make up for dots spreading on paper.
- **Line art:** a simple threshold, best for text, logos and QR codes.
- **Sketch:** redraws a photo as outlines and cross-hatching, like a pen drawing.

There are 15 algorithms in total. `examples/test-chart.png` is a tone chart for comparing them.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p thermal-studio -- --screenshot shot.png   # render one frame of the app to PNG and exit
```

## License

The code is released under the [MIT License](LICENSE).

The bundled fonts in `crates/thermal-markdown/fonts/` (Atkinson Hyperlegible, Fira Mono and Noto Sans Tamil) are
under the [SIL Open Font License 1.1](https://openfontlicense.org). Their licence texts are next to the font files.
