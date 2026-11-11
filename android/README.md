# Vibe Local — Android app

Native Android client for **Vibe Local**, a local AI inference manager.
Kotlin 2.0 + Jetpack Compose (Material 3, Obsidian Dark), AndroidX-only
dependencies, `org.json` + `HttpURLConnection` for HTTP. The inference engine
is the Rust workspace at the repository root, linked through JNI
(`libvibe_local_jni.so`).

## Requirements

| Tool | Version |
|---|---|
| Android Studio | Ladybug (2024.2) or newer |
| Android SDK | compileSdk/targetSdk **35**, minSdk **26** |
| JDK | 17 |
| Gradle | 8.9 (wrapper) |
| Android NDK | **r27** (for the Rust native lib) |
| Rust | stable, with `cargo-ndk` (`cargo install cargo-ndk`) |
| Rust targets | `aarch64-linux-android`, `armv7-linux-androideabi` |

### Gradle wrapper jar

This repository intentionally ships **no binary** `gradle/wrapper/gradle-wrapper.jar`.
The `gradlew` / `gradlew.bat` scripts and `gradle-wrapper.properties` (Gradle 8.9)
are present. Regenerate the jar once, either by:

- opening `android/` in Android Studio (it syncs and restores the wrapper), or
- running `gradle wrapper --gradle-version 8.9` in `android/` with a local Gradle installation.

After that, `./gradlew assembleDebug` works normally.

## Building the native library

From the **repository root** (the Cargo workspace), with NDK r27 installed and
`ANDROID_NDK_HOME` (or `NDK_HOME`) pointing at it:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi
cargo install cargo-ndk

cargo ndk -t arm64-v8a -t armeabi-v7a -o android/app/src/main/jniLibs build --release -p vibe-local-jni
```

This drops `libvibe_local_jni.so` into
`android/app/src/main/jniLibs/{arm64-v8a,armeabi-v7a}/`, which the Gradle build
packages automatically.

### Graceful fallback (no native lib)

If `System.loadLibrary("vibe_local_jni")` fails (missing ABI, linker error,
library not built yet), the app **does not crash**:

- `NativeLib` records the failure; every JNI call goes through `SafeBridge`,
  which refuses to call into a missing library and returns `BridgeResult.Err`.
- A persistent banner explains the situation.
- The repository switches to **HTTP fallback mode**: all reads/writes go to a
  user-entered Vibe-Link host (Chat & Link tab → "Vibe-Link host"), e.g. a
  desktop running `vibe-local-cli serve --lan`. An optional
  `X-Vibe-Local-Token` can be configured for LAN-authenticated hosts.
- LAN worker mode and leaderboard submission require the native engine and
  are disabled with an explanatory message in fallback mode.

## Building the APK

```bash
cd android
./gradlew assembleDebug        # app/build/outputs/apk/debug/app-debug.apk
./gradlew assembleRelease      # minified + shrunk (see proguard-rules.pro)
```

No Android SDK is required to *read* this source tree; the code is complete
but has not been compiled on the authoring machine.

## Architecture

```
com.vibelocal.app
├── VibeLocalApp / AppContainer     manual DI: repository + guards + downloads
├── MainActivity                    edge-to-edge Compose host
├── core
│   ├── NativeLib                   one-time System.loadLibrary + availability flag
│   ├── VibeLocalBridge             object with the 13 external JNI functions
│   ├── SafeBridge / BridgeResult   try/catch wrapper (UnsatisfiedLinkError, JSON)
│   ├── Models                      typed payload mirrors + defensive org.json parsers
│   ├── LmkdMemoryGuard             guard #1
│   ├── ThermalGovernor             guard #2
│   ├── VibeLinkForegroundService   guard #3
│   ├── AutoUnloadIdleTimer         guard #4
│   └── LanWorkerController         token/IP/lifecycle coordination for the service
├── data
│   ├── VibeRepository              transport switch: JNI ⇄ HTTP fallback
│   ├── VibeHttpClient              HttpURLConnection + org.json, token header
│   ├── DataDirManager              internal filesDir/vibe_data ⇄ SAF USB OTG root
│   ├── DownloadController          DownloadManager → staging → models dir import
│   └── AppPrefs                    SharedPreferences settings
├── ui
│   ├── MainScreen                  4-tab Scaffold + global safety banners
│   ├── theme                       Obsidian Dark (#090A0F / #131620 / #7C5CFF / #22D3EE)
│   ├── scan                        Scan & Match tab (StateFlow ViewModel)
│   ├── leaderboard                 10-pillar leaderboard + Canvas Pareto chart
│   ├── vibemark                    benchmark tab
│   └── chat                        chat + LAN worker + settings tab
└── util                            JSON helpers, LAN IP discovery, formatting
```

Each tab has a `ViewModel` exposing a single `StateFlow<UiState>`; the
repository is the only place that talks to the engine. All hardware numbers
shown in the UI come from `scanHardware()` / `/api/hardware` — nothing is
hardcoded.

### JNI contract

`object VibeLocalBridge` (instance methods on the Kotlin `object` singleton)
exposes exactly: `scanHardware`, `recommend`, `inspectGguf`, `runVibeMark`,
`leaderboard`, `submitLeaderboard`, `startServer`, `stopServer`,
`serverStatus`, `chat`, `unloadModel`, `setThermalReading`, `setIdleTimeout`.
Every function returns a JSON string `{"ok":true,"data":...}` or
`{"ok":false,"error":"..."}`. The Rust crate should bind via
`RegisterNatives` or `Java_com_vibelocal_app_core_VibeLocalBridge_<name>`
symbols. ProGuard keeps these names (see `app/proguard-rules.pro`).

### HTTP API

Native server listens on `127.0.0.1:20129` (also used for LAN worker mode):

- `GET /api/hardware`, `/api/recommendations`, `/api/leaderboard?profile=<id>`, `/api/status`
- `POST /api/load {path,backend,ngl,ctx}`, `POST /api/unload`,
  `POST /api/vibemark {path,budget_secs}`
- `POST /v1/chat/completions` (OpenAI shape + extra `"strip_think":true`), `GET /v1/models`

LAN mode requires `X-Vibe-Local-Token: <token>` on `/api/*` and `/v1/*`;
mutating POSTs send `Content-Type: application/json`. The client adds the
header automatically whenever a token is configured.

## The four safety guards

1. **`LmkdMemoryGuard`** — polls `ActivityManager.MemoryInfo`
   (`availMem`, `threshold`, `lowMemory`) every 2 s into a `StateFlow`.
   Model loads are blocked unless `availMem ≥ modelSizeBytes + 512 MB`.
   Registers `ComponentCallbacks2`; at `TRIM_MEMORY_RUNNING_CRITICAL`
   (or `onLowMemory`) it unloads the model immediately. The UI shows a
   low-memory banner and a precise "blocked" message on load attempts.

2. **`ThermalGovernor`** — `PowerManager.addThermalStatusListener` (API 29+)
   maps `THERMAL_STATUS_*` to actions; additionally reads SoC thermal zones
   from `/sys/class/thermal` (when readable), forwards the reading into the
   native governor via `setThermalReading()`, and reads the native view back
   through `serverStatus()`. Native thresholds: **42 °C throttle, 45 °C pause**.
   At `SEVERE+` (or ≥ 45 °C) inference is paused: the model is unloaded, chat
   sends are blocked, and a warning banner is shown.

3. **`VibeLinkForegroundService`** — notification channel `vibe_link`,
   ongoing notification with a Stop action, `START_STICKY`, partial WakeLock
   (+ optional high-perf WifiLock), `foregroundServiceType="dataSync"`.
   Calls `startServer(port, lan=true, token, dataDir)` on start and
   `stopServer()` on destroy. Only runs while LAN worker mode is enabled;
   if the server fails to start, the service stops itself.

4. **`AutoUnloadIdleTimer`** — coroutine countdown, default **300 s**,
   configurable **60–1800 s** via the slider in Chat & Link. Reset on every
   chat/load interaction; on expiry it syncs `setIdleTimeout()` into the
   engine and calls `unloadModel()`.

## LAN pairing with Vibe Router

1. Open **Chat & Link**, toggle **Serve this device on the LAN**.
2. The app generates a **32-hex token** (`SecureRandom`, 16 bytes), starts
   the native server on port **20129** with `lan=true`, and launches the
   foreground service.
3. The card shows the token and the device LAN IPv4 address
   (`NetworkInterface` enumeration, preferring `wlan*`/`eth*`).
4. In **Vibe Router**, add a worker at `http://<device-ip>:20129` with that
   token. Every `/api/*` and `/v1/*` request must carry
   `X-Vibe-Local-Token: <token>`; mutating POSTs must be JSON.
5. **Stop worker** (button or notification action) calls `stopServer()` and
   stops the service; the token is discarded.

## Data directory & downloads

- Default data dir: `context.filesDir/vibe_data`; models in `<dataDir>/models`.
- Optional **USB OTG root** via SAF (`ACTION_OPEN_DOCUMENT_TREE`, persisted
  URI permission). Java-side I/O uses `DocumentFile`; the native engine gets
  the derived `/storage/<volume>` path.
- Downloads use `DownloadManager` into app-external staging
  (`getExternalFilesDir("vibe_downloads")`), then are moved into the models
  dir (stream-copy fallback for SAF). **Only `https://huggingface.co/` URLs
  are accepted.** Progress is polled and shown per candidate.

## Known limitations

- The Gradle wrapper jar must be regenerated (see above); no binary is committed.
- The native lib must be built with `cargo-ndk` before JNI mode works;
  otherwise the app runs in HTTP fallback mode.
- SAF/OTG: direct filesystem access to removable volumes is subject to
  scoped-storage/FUSE policy. If the native engine cannot open a derived
  `/storage/<volume>` path on a given OEM build, use internal storage.
- The JNI bridge has no `loadModel()`; loading goes through `POST /api/load`.
  In native mode the app starts the loopback server (`lan=false`) on demand.
- `chat()` JNI takes a single prompt string, so the transcript is flattened
  for native mode; HTTP mode sends the full OpenAI messages array.
- Leaderboard submission is native-only (no HTTP submit endpoint is defined
  in the API contract).
- Android 15 limits `dataSync` foreground services to ~6 h per day; the
  WakeLock uses a matching safety timeout.
- Gated Hugging Face repos need auth headers that DownloadManager cannot
  carry anonymously; such downloads will fail with an HTTP error.