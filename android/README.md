# Thermal Studio for Android

Jetpack Compose app on top of the same Rust crates as the desktop tools, reached through
[UniFFI](https://mozilla.github.io/uniffi-rs/) (`crates/thermal-ffi`). Bluetooth is done in Kotlin with
Android's `BluetoothGatt`; the print flow itself is the shared, hardware-verified `PrintSession` from
`thermal-proto`.

```
crates/thermal-raster, thermal-markdown, thermal-proto   shared Rust core
crates/thermal-ffi                                       UniFFI facade → libthermal_ffi.so + Kotlin bindings
android/app                                              Compose UI, Kotlin BLE transport
```

Features: Markdown editor with live preview (English and Tamil), photo dithering (all algorithms including
Sketch, presets, tone controls, rotation), QR codes for links with an optional caption, templates (a Wi-Fi card,
checklist, jar label and contact card, plus more from the official collection, a link or a file), "Share → Thermal Studio" for images,
links and text, and BLE printing with progress, battery and status. Choose your printer once from a list of nearby
devices; the app remembers it.

The app only uses the internet when you open **Add a template** (to list the official collection) or add or
refresh a template.

## Requirements

| Tool | Version | Notes |
|---|---|---|
| JDK | 17+ | `brew install openjdk@21` |
| Android SDK | platform `android-37.2` (compile; runtime target is API 36), build-tools 36.x, platform-tools | `~/Library/Android/sdk` |
| Android NDK | `30.0.16248370` | pinned in `app/build.gradle.kts` |
| Rust | workspace toolchain + `aarch64-linux-android` | `rustup target add aarch64-linux-android` |
| cargo-ndk | 4.x | `cargo install cargo-ndk` |

Android Studio is optional. If you install it, point it at the same SDK (`~/Library/Android/sdk`).

## Build

```sh
cd android
export JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home   # or Android Studio's JBR
echo "sdk.dir=$HOME/Library/Android/sdk" > local.properties                     # first time only
./gradlew assembleDebug        # → app/build/outputs/apk/debug/app-debug.apk
```

`preBuild` runs two Gradle tasks first:

1. `cargoBuildRust`: `cargo ndk -t arm64-v8a … build --release -p thermal-ffi` →
   `app/src/main/jniLibs/arm64-v8a/libthermal_ffi.so`
2. `generateUniffiBindings`: `uniffi-bindgen generate --library …` →
   `app/src/main/java/uniffi/thermal_ffi/thermal_ffi.kt`

Both outputs are git-ignored and regenerated when the Rust sources change.

## Install on a phone (wireless debugging, no cable)

On the phone (Android 11+), go to **Settings → About phone** and tap **Build number** seven times. Then
**Developer options → Wireless debugging → Pair device with pairing code**.

```sh
ADB=~/Library/Android/sdk/platform-tools/adb
$ADB pair <ip>:<pairing-port>      # enter the 6-digit code shown on the phone
$ADB connect <ip>:<port>           # the port shown on the Wireless debugging screen
$ADB devices
./gradlew installDebug             # or: $ADB install -r app/build/outputs/apk/debug/app-debug.apk
```

## Using it

1. Switch the printer on and close the vendor app (the printer accepts one BLE connection).
2. Press **Print**. The first time, Android asks for the *Nearby devices* permission, then shows **Choose
   printer** with the printers nearby. Tap yours. It's remembered, and the print continues.
3. Each print connects to the remembered printer's address, prints, reads battery/status and disconnects, as the
   vendor app does. If the address fails, the app finds the printer again by name and updates the saved address.
4. The printer chip opens settings: the chosen printer with its last-known model, battery and status, **Check
   status / Change / Forget**, plus density and feed after printing.
5. Debug logs use the tag `ThermalStudio`: `adb logcat -s ThermalStudio`.
6. Prints longer than 200 mm ask for confirmation.

## Notes

- **minSdk 33** (Android 13) keeps the BLE code on the modern, non-deprecated GATT APIs.
- Only **arm64-v8a** is built. Add `"armeabi-v7a"`/`"x86_64"` to `abiFilters` and the `cargo ndk -t` list if
  needed (the emulator has no BLE anyway).
- **Photos** are decoded by Android (`ImageDecoder`: HEIC, AVIF, EXIF rotation), flattened onto white, capped
  at 2048 px and passed to Rust as RGBA.
- **Text:** English and Tamil, using fonts embedded in the Rust library (Atkinson Hyperlegible, Fira Mono, Noto
  Sans Tamil). Output looks the same on the phone and the desktop.
- **Markdown images** are resolved relative to the app's private `files/images` directory.
- **Sharing a link** (for example from Chrome) opens it on the **QR code** tab, using the page title as the
  caption. Shared text that's more than a single link goes to the Markdown editor as before.
