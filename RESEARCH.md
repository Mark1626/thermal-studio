# Seznik MiniX: hardware and protocol notes

Notes from reverse-engineering the **Seznik MiniX** pocket thermal printer, which this project's tools are built on.
Everything here was observed on one unit from a Mac in November 2026, using `blueutil`, `system_profiler`, the
`bluetoothd` logs and the small probe programs in [`tools/probe/`](tools/probe/).

Printer addresses and serial numbers are redacted. Where a command needs your printer's address, it's written as
`<address>`.

## Summary

- **It's a rebadged LuckJingle S1.** The hardware comes from Xiamen Lujiang Technology ("LuckJingle") and speaks
  their "Lujiang" protocol: proprietary `10 FF …` commands plus the ESC/POS `GS v 0` raster command. It is *not* a
  standard ESC/POS printer.
- **Use Bluetooth LE.** The printer has a BLE interface (service `FF00`) that needs no pairing. That's what the
  official phone app uses, and what these tools use.
- **It also has Classic Bluetooth (a serial port).** This works too, but it needs pairing, and macOS makes that
  look broken (§3).
- **Printing is verified.** A full test print over BLE worked end to end (§5.3).
- **There's prior art.** [minixprint](https://github.com/SwapnanilDhol/minixprint) targets the same printer. The unit
  tested here runs older firmware that behaves differently in a few places (§5.4).

---

## 1. Device identity

| Field | Value | How it was read |
|---|---|---|
| Classic Bluetooth name | `Seznik MiniX_XXXX` (`XXXX` = last two bytes of the address, swapped) | Bluetooth inquiry |
| BLE name | `Seznik MiniX_XXXX_LE` | BLE scan |
| Address prefix (OUI) | `90:22:33`, not registered with the IEEE | maclookup.app |
| Firmware model | `S1_EY628` | `10 FF 20 F0` |
| Firmware version | `V1.6.23` | `10 FF 20 F1` |
| Serial number | `P#####S1B#####` | `10 FF 20 F2` |
| Bluetooth chip | Yichip (Hangzhou), Bluetooth 4.2 dual mode (company ID `0x050E`) | `bluetoothd` log |
| Print width | **384 dots** per line (verified), 203 dpi, 58 mm paper | Test print and product listing |

---

## 2. Bluetooth layout

### 2.1 Classic Bluetooth

The printer advertises Class of Device `0x042610`: an *Imaging* device with the *Display* bit set, rather than the
*Printer* bit. It's a harmless quirk.

It offers two SDP records:

| Service | Details |
|---|---|
| Serial Port (`0x1101`), named `"SPP slave "` | **RFCOMM channel 1** |
| Device ID (`0x1200`) | Vendor ID `0x05AC` (Apple), product ID `0x0239`, version `0x0644` |

Serial Port is the only useful Classic channel. The Device ID record falsely identifies the printer as an Apple
keyboard (§3.3).

### 2.2 Bluetooth LE

The printer advertises as `Seznik MiniX_XXXX_LE`, with **no service UUIDs or manufacturer data**. Scanning with a
service filter won't find it, so match on the name instead.

No pairing or bonding is needed to connect.

| Characteristic (service `FF00`) | Properties | Used for |
|---|---|---|
| `FF01` | notify | Replies to commands (model, battery, status, `OK`) |
| `FF02` | write, write without response | **Commands and print data** |
| `FF03` | notify | Flow-control credits (§5.2) |

The full service UUID is `0000FF00-0000-1000-8000-00805F9B34FB`. On macOS the maximum write is 237 bytes without
response (ATT MTU 240) or 512 bytes with response.

---

## 3. Why pairing looks broken on macOS

Pairing the Classic side from System Settings seems to fail. There are three separate reasons, and none of them is a
missing driver.

| What you see | Why | What to do |
|---|---|---|
| Pairing hangs, then fails | Nobody confirmed the pairing request, so it timed out after 32 s | Accept the macOS pairing dialog |
| "Connected" flips back to "Not Connected" | The printer only offers a serial port, so macOS has nothing to keep open and drops the link after ~15 s | Nothing; this is normal. Opening the serial port reconnects it. |
| Wrong icon or odd behaviour in Settings | The printer claims to be an Apple keyboard | Ignore it |
| The BLE side never shows up as pairable | BLE doesn't need pairing | Connect directly over BLE |

The log excerpts below are from `bluetoothd`. Times are relative to the first line, and the printer's address is
replaced with `<printer>`.

### 3.1 Pairing needs someone to confirm it

The printer uses Secure Simple Pairing, which macOS handles as a "numeric comparison" request. When `blueutil --pair`
started pairing, the request went to blueutil instead of the system dialog. Nothing confirmed it, so it timed out:

```
 +0.000s Server.Security  Secure-Simple-Pairing has started on device …
 +0.937s Server.App       CBClassicSession::pairingRequested: Authentication type for device …: Numeric Comparison (SSP)
 +0.937s Server.Security  Sending 'numeric comparison request' pairing event for device <printer>
+32.930s Server.Security  Reached pairing timeout for pending attempt with device <printer>
+32.930s Server.Security  pairing timedout, rejecting pairing request
+32.934s Server.Security  Pairing has completed with result 705 on device <printer>
```

When pairing was started in a way that let macOS show its own dialog, accepting it worked first time:

```
 +0.000s Server.BTUIClient Sending pairSSPWithNumericComparison for address: <private>
 +3.386s Server.BTUIClient Device Address: <private> with accept 1, ignore button 0
 +3.431s Server.Security   Pairing has completed with result 0 on device <printer>
 +3.589s Server.Security   New link key created for device …
```

The printer has no screen, so there's no code to compare. Just accept the dialog.

### 3.2 The link drops after about 15 seconds

After pairing, connecting brings the link up. macOS then finds no profile it wants to keep open (`sevices: 0`) and
disconnects:

```
 +0.000s CBStackDeviceMonitor ACL connected: <printer>, result 0
 +0.394s Server.Core   Overriding Connection Result for device <private> for serial port profile
 +0.394s Server.App    UN: notifyDeviceConnected: ACL? 1, sevices: 0
+15.457s CBStackDeviceMonitor ACL disconnected: <printer>, result 0, reason 10722
```

This is normal for a serial-only device. Pairing also creates a serial port, `/dev/cu.SeznikMiniX_XXXX`. Opening
it brings the link back up on demand.

### 3.3 The fake Apple identity

macOS reads the Device ID record (Apple's vendor ID with product ID `0x0239`) and treats the printer as an **Apple
Wireless Keyboard**. It then tries Apple-only features the printer doesn't support:

```
Device found: CBDevice …, PID 0x0239 (?), PrNm Apple Wireless Keyboard, VID 0x05AC, VS 2
Handle query complete … pid 0x239 isAppleBluetooth 1
HIDIdentification - Start identification for <private>
Enforce policy for <printer>, AVDTP
```

This is probably a default left in the Bluetooth module's firmware. It makes System Settings look broken, but it
doesn't stop the serial port or BLE from working.

---

## 4. Transports

| Transport | Status | Notes |
|---|---|---|
| **BLE** (`FF00`) | ✅ Works, no pairing | Subscribe to `FF01` and `FF03`, write to `FF02`. On macOS the app needs Bluetooth permission. |
| **Classic serial port** | ✅ Works once paired | On this firmware the serial port answers queries. On minixprint's newer firmware it never replies. |
| USB | Not tested | minixprint reports a USB serial device on their unit (VID `0x3021`, PID `0x6001`). |

---

## 5. Protocol

Commands go to `FF02` (or the serial port). Replies come back on `FF01`.

### 5.1 Queries

These are read-only and use no paper. The serial port gave the same answers for the commands tried over it.

| Command | Meaning | Reply |
|---|---|---|
| `10 FF 20 F0` | Model | `S1_EY628` |
| `10 FF 20 F1` | Firmware | `V1.6.23` |
| `10 FF 20 F2` | Serial number | `P#####S1B#####` |
| `10 FF 50 F1` | Battery | `00 <percent>`, for example `00 45` = 69 % (~800 ms to reply) |
| `10 FF 40` | Status bits | `00` = no errors |
| `10 FF 11` | Density | `01 00 00` (0 light, 1 normal, 2 dark) |
| `10 FF 13` | Auto power-off | `17` (23, probably minutes) |
| `10 FF 70` | All info | No reply on this firmware |
| `10 FF 20 A0` | Print speed | No reply |

Standard ESC/POS status and identity queries (`DLE EOT 1`, `GS I 1`) get no reply.

### 5.2 Flow control on `FF03`

The printer sends small frames on `FF03`:

```
on connect:          01 07       02 C8 00
after every write:   01 01
```

- `01 nn` grants *nn* send credits: 7 when you connect, then one more for each packet the printer consumes.
- `02 C8 00` reports the printer's MTU (200).

During the test print, 82 writes produced 83 `01 01` frames, while the status query still answered on `FF01`. So
these frames are credits, not status (minixprint reads them as status).

With write-with-response and one packet in flight, the write acknowledgements already pace the data and credits
never run out. A faster write-without-response sender should track credits: start at 7, subtract one per packet and
add *nn* for each `01 nn`.

### 5.3 Printing

A print job is this sequence:

```
00 ×12                       wake
10 FF F1 03                  enable "Lujiang mode" (minixprint: without it the printer accepts data but never prints)
10 FF 10 00 nn               set density 0–2 (optional)
1D 76 30 00 30 00 hL hH      GS v 0 raster header: 48 bytes per row (384 dots), h rows
<rows × 48 bytes>            1 bit per dot, most significant bit = leftmost dot, 1 = black
1B 4A nn                     feed nn dots
10 FF F1 45                  end of job
```

**Test print.** [`tools/probe/ble_print.swift`](tools/probe/ble_print.swift) printed a 384 × 200 image with a border,
some text, a ruler and four grey swatches. The 9,608 bytes went in 76 chunks of 128 bytes, using write with
response and a 20 ms gap between chunks:

```
  1.658s   <- FF03 [01 07]                         initial credits
  1.658s   <- FF03 [02 C8 00]                      MTU 200
  2.648s -> status 10 FF 40
  2.676s   <- FF01 [00]                            no errors
  2.688s -> wake 00 ×12
  2.708s -> enable Lujiang 10 FF F1 03
  2.774s -> GS v 0 header [1D 76 30 00 30 00 C8 00] + 9600 raster bytes in 76 × 128-byte chunks
  5.653s raster sent in 2.88 s                     ≈ 3.3 KB/s, no errors or retries
  5.976s   <- FF01 [4F 4B] "OK"                    raster done
  5.986s -> feed 80 dots 1B 4A 50
  6.007s -> end job 10 FF F1 45
  6.158s   <- FF01 [4F 4B] "OK"                    feed and end done
  9.029s -> status 10 FF 40
  9.053s   <- FF01 [00]                            status clean afterwards
```

**What the paper showed:**

| Check | Result | What it confirms |
|---|---|---|
| Border | ✅ All four edges printed | The full 384-dot width, with no cropping or offset |
| Ruler | ✅ Ticks every 48 dots, lined up | Byte boundaries are correct across the row |
| Text | ✅ Legible, not mirrored or inverted | Bit order and row order are right |
| Grey swatches | ⚠️ The 1-dot checkerboard came out muddy; the 25 % dot grid was very faint | Single isolated dots blur together on thermal paper |

**What this means for rendering:** avoid patterns made of single isolated dots. For photos, use error-diffusion
dithering (such as Atkinson) with a contrast and gamma boost, or clustered patterns at least 2 dots wide. Text and
line art print cleanly as plain black and white.

### 5.4 Differences from minixprint's unit

| | minixprint's unit | This unit |
|---|---|---|
| Model / firmware | `S1_LYiN48D_IND`, `V1.9.9` | `S1_EY628`, `V1.6.23` |
| Serial port replies | Never | Yes |
| `10 FF 70` (all info) | Supported | No reply |
| Status reply | `01 <byte>` on `FF03` | `00` on `FF01` |
| Credits on connect | `01 05` | `01 07` |
| Reply after feed and end of job | None | `OK` |

---

## 6. Where the hardware comes from

- **Seznik** (Seznik Lifestyle Pvt Ltd, India) sells the MiniX on Amazon.in and seznik.in. It's a rebrand.
- The printer is the **LuckJingle S1** from **Xiamen Lujiang Technology Co., Ltd.**, a maker of OEM portable
  thermal printers. The `S1_…` model string, the `10 FF …` commands and minixprint's findings all point to it.
- The official app is **Luck Jingle** (Android `com.dingdang.newprint`, iOS App Store id `1515245571`). It's built
  on the vendor's LuckPrinter SDK, which reportedly covers more than 150 models.
- The same BLE layout (`FF00` with `FF01`, `FF02` and `FF03`) is used by Phomemo, Marklife and other printers built
  on that SDK.
- The printer only prints images. Text mode and most ESC/POS commands don't work, so a "driver" is really an image
  renderer plus the framing above.

---

## 7. Prior art

| Project | Relevance |
|---|---|
| [SwapnanilDhol/minixprint](https://github.com/SwapnanilDhol/minixprint) | Python driver for the Seznik MiniX / LuckJingle S1 (MIT), with a `PROTOCOL.md` |
| [ChiaraCannolee/thermal-pocket-printer-basic](https://github.com/ChiaraCannolee/thermal-pocket-printer-basic) | Reverse engineering of the DP-L1S, another LuckPrinter SDK printer, from the Luck Jingle app |
| [Its issue #3](https://github.com/ChiaraCannolee/thermal-pocket-printer-basic/issues/3) | The Seznik LabelX (same `FF00` service) and the `FF03` credit behaviour |
| [0xMH/fichero-printer](https://github.com/0xMH/fichero-printer) | Notes on a related Lujiang protocol |
| [mzramna/phomemo-t02-linux](https://github.com/mzramna/phomemo-t02-linux) | Same BLE layout (Phomemo), different commands |

---

## 8. Open questions

- Does printing work over the Classic serial port?
- Does this firmware print without the `10 FF F1 03` enable command? (minixprint says V1.9.9 doesn't.)
- What unit is the auto power-off value in, and do the setters (`10 FF 12 hi lo`, `10 FF 10 00 n`) work?
- Is the USB-C port a serial device or charge-only?

---

## 9. Reproducing this

The probes are macOS-only. Build them with the Xcode command-line tools.

```sh
# Classic Bluetooth discovery and pairing state
blueutil --inquiry 15
blueutil --info <address>
system_profiler SPBluetoothDataType

# Probes
swiftc -O tools/probe/bt_info.swift   -o bt_info   && ./bt_info <address>   # Class of Device and SDP records
swiftc -O tools/probe/ble_scan.swift  -o ble_scan  && ./ble_scan minix 15   # BLE advertisement and GATT table
swiftc -O tools/probe/ble_query.swift -o ble_query && ./ble_query           # read-only queries over BLE
python3 -I tools/probe/spp_query.py /dev/cu.SeznikMiniX_XXXX                 # same queries over the serial port

# Bluetooth logs. In zsh, `log` is a shell builtin, so use the full path.
/usr/bin/log show --last 30m --style compact --predicate 'process == "bluetoothd"'
```

None of the probes above print or feed paper. The one that prints is separate and only prints when asked to:

```sh
swiftc -O tools/probe/ble_print.swift -o ble_print
./ble_print preview.png            # render only; check the preview first
./ble_print preview.png --print    # check status, then print once (~25 mm of paper)
```

Pairing over Classic Bluetooth leaves the printer paired and creates a serial port. To undo it, run
`blueutil --unpair <address>`.

---

## Sources

- [SwapnanilDhol/minixprint](https://github.com/SwapnanilDhol/minixprint) and its
  [PROTOCOL.md](https://github.com/SwapnanilDhol/minixprint/blob/main/PROTOCOL.md)
- [ChiaraCannolee/thermal-pocket-printer-basic](https://github.com/ChiaraCannolee/thermal-pocket-printer-basic) and
  [issue #3 (Seznik LabelX)](https://github.com/ChiaraCannolee/thermal-pocket-printer-basic/issues/3)
- [0xMH/fichero-printer PROTOCOL.md](https://github.com/0xMH/fichero-printer/blob/main/docs/PROTOCOL.md)
- [mzramna/phomemo-t02-linux](https://github.com/mzramna/phomemo-t02-linux)
- [print_master_ble (pub.dev)](https://pub.dev/packages/print_master_ble): `FF03` credit-based flow control
- [Luck Jingle (App Store)](https://apps.apple.com/us/app/luck-jingle/id1515245571),
  [Luck Jingle (AppBrain)](https://www.appbrain.com/app/luck-jingle/com.dingdang.newprint),
  [luckjingle.com](https://luckjingle.com/)
- [SEZNIK MiniX on Amazon.in](https://www.amazon.in/SEZNIK-Bluetooth-Thermal-Portable-Warranty/dp/B0D1GYL3MN),
  [seznik.in](https://seznik.in/)
- [Nordic bluetooth-numbers-database](https://github.com/NordicSemiconductor/bluetooth-numbers-database)
  (company ID `0x050E`)
- [maclookup.app](https://maclookup.app/search) (OUI `90:22:33`)
