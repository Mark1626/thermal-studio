# thermal-studio

Open-source tools for printing Markdown notes, photos and QR codes on the **Seznik MiniX** pocket thermal printer (a
rebadged LuckJingle S1 using 58 mm paper). It includes a desktop app, a command-line tool and an Android app, all built
on the same Rust core.

For how the printer's Bluetooth protocol was worked out, see [`RESEARCH.md`](RESEARCH.md).

## Getting started

You need a recent Rust toolchain (1.85 or newer).

### Desktop app

```sh
cargo run --release -p thermal-studio              # opens with a sample note
cargo run --release -p thermal-studio -- photo.jpg # or open an image (drag and drop also works)
cargo run --release -p thermal-studio -- https://example.com # or make a QR code for a link
```

Write Markdown, load a photo or type a link for a QR code, adjust it with a live preview, then press **Print**.
In Markdown, a fenced code block marked `qr` prints as a QR code.

The **Templates** tab has ready-made notes with a few blanks to fill in: a Wi-Fi card with a QR code that guests
can scan, a checklist, a jar label with the date it was made and a use-by date, and a contact card whose QR code
saves your details. **Add…** lists more from the official collection in [`templates/`](templates), or adds one from an
`https://` link or a file. Added templates are saved, so
they work offline. To write your own, see [`crates/thermal-templates`](crates/thermal-templates/README.md).

### Command line

```sh
# Save a preview as PNG
cargo run --release -p thermal-cli -- markdown examples/sample.md -o note.png
cargo run --release -p thermal-cli -- image photo.jpg -o preview.png

# Print over Bluetooth
cargo run --release -p thermal-cli -- markdown note.md --print
cargo run --release -p thermal-cli -- image photo.jpg --print

# Fill in a template: a ready-made note with a few blanks (list them with `templates`)
cargo run --release -p thermal-cli -- template wifi-card --set ssid=Home --set password=secret -o wifi.png
cargo run --release -p thermal-cli -- template checklist --set items=Milk --set items=Eggs --print

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
| [`crates/thermal-raster`](crates/thermal-raster) | Image preparation, dithering and QR codes |
| [`crates/thermal-markdown`](crates/thermal-markdown) | Markdown and QR-label rendering, with bundled English and Tamil fonts |
| [`crates/thermal-templates`](crates/thermal-templates) | Fill-in-the-blanks templates that produce Markdown (format described in its README) |
| [`templates/`](templates) | The official collection of templates the apps offer to add, and its `index.json` |
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

### QR codes

QR codes aren't dithered. They're drawn dot for dot, so every module is a whole number of printer dots (never fewer
than 3, about 0.4 mm) with the standard white border around it. A link you type like `example.com` is encoded as
`https://example.com` so phone cameras open it, while the text printed under the code stays as you typed it. Error
correction is raised automatically whenever that doesn't make the code bigger.

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
