# comeback

Record a bug on your screen instead of describing it to your coding agent.

Press Alt+Shift+S and talk through the problem while you click around and draw on the screen. Press it again to stop. comeback turns the recording into a small bundle an agent can read: a contact sheet of the key moments, full-size keyframes, a transcript tied to those moments, and a JSON manifest. The text summary lands on your clipboard, ready to paste into Claude Code, Codex, OpenCode or any other agent.

In a kitchen, a comeback is a dish the guest sends back because something is wrong.

## Status

Early. On X11, `comeback record` and the background daemon work. Wayland, the tray icon and most of the review commands do not exist yet. The design lives in [docs/specs](docs/specs) and the build plans in [docs/plans](docs/plans). Linux comes first, with X11 and Wayland both supported in v1. Windows follows.

| Works today (X11) | Not built yet |
|---|---|
| `comeback record [--window \| --area X,Y,W,H] [--duration S]`: screen, area or clicked window, microphone, clicks, drags, Shift-drag red ink | Wayland backend |
| Local transcription with `whisper-cli` and the ggml models you already have | Tray icon, Esc to cancel |
| Contact sheet, keyframes, transcript, `summary.md`, `comeback.json`, `recording.mp4`, `audio.wav` | `zoom`, `frames`, `export`, `rebuild`, `model fetch`, `pin` |
| Clipboard copy of the summary (`xclip`, `xsel` or `wl-copy`) | Dev-page SDK |
| `comeback daemon` with Alt+Shift+S, `toggle`, `cancel`, `status` | Daily cleanup of old captures, crash recovery of an interrupted recording |
| `comeback inspect`, `comeback demo`, `comeback skill [--install DIR]` | Stroke fade |

## Use

```bash
# record the monitor under the pointer until you press Enter or Ctrl-C
comeback record

# record a 30 s clip of one area, or click a window to record it
comeback record --area 0,0,1280,720 --duration 30
comeback record --window

# print what an agent should read
comeback inspect latest
```

Hold Shift and drag with the left button inside the recorded area to draw red ink. It appears in the video and in the keyframes, and leaves the screen 3 s after the last stroke (there is no fade yet).

While you draw, the program under the pointer gets no press, motion or release: comeback grabs Shift+left-button on the root window before any application sees it. Only exactly Shift counts. A Shift press outside the recorded area, Ctrl+Shift, other buttons and every plain click or drag still go to the application as usual. A Shift-click inside the area that never moves is recorded as a click and is also kept from the application, so use a plain click there. The ink never takes input either, so clicks on it reach the window underneath. The pointer is only borrowed: it goes back when you stop, when comeback fails and when it is killed. If another program already owns Shift+left-drag, `comeback record` says so when it starts and the bundle has a note.

Run the daemon from a terminal or a systemd user unit you write yourself. comeback does not install one.

```bash
comeback daemon          # grabs Alt+Shift+S, listens on $XDG_RUNTIME_DIR/comeback.sock
comeback toggle          # same as the hotkey: start, or stop and copy the summary
comeback cancel          # discard the recording in progress, keep every finished capture
comeback status
```

The control socket is mode 0600 and the daemon refuses a socket directory other users can enter. If another program already owns Alt+Shift+S the daemon says so and `comeback toggle` still works. Holding the hotkey counts as one press.

Stopping saves the bundle in the background, which can take a while with a long recording and a big Whisper model. Meanwhile `comeback status` says `compiling <id>`, `comeback cancel` says that the capture is already recorded and is kept (a finished capture is never discarded), and `comeback toggle` and the hotkey are refused as busy and not queued, so a press made during the wait cannot start a recording afterwards. Starting a recording is not interruptible: `status` waits for it, normally under a second and up to 14 s when the microphone is slow to answer.

Bundles go to `~/.local/share/comeback/captures/comeback-<unix_ms>.comeback/` (`COMEBACK_CAPTURES_DIR` overrides it). While a capture runs its raw media sits in `.recording-<id>/` next to the bundles. The bundle is built from hard links to that media and the directory is deleted only after the bundle exists, so a failed step, including an unreadable MP4, leaves the recording on disk and the error names the directory.

The video is H.264 High 4:2:0 and plays in browsers and QuickTime. The audio is 16 kHz mono 16-bit PCM in `audio.wav` and AAC in the MP4. If comeback is killed, the encoder gets the signal Ctrl-C would send, finishes its files and exits, so nothing keeps recording unseen.

## Requirements

Ubuntu 24.04 names. Nothing here needs a development header.

- GStreamer runtime with `gst-launch-1.0`, `ximagesrc`, `x264enc`, `mp4mux`, `avenc_aac`, `pulsesrc`: `gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-ugly gstreamer1.0-libav gstreamer1.0-x gstreamer1.0-pulseaudio`
- `ffmpeg` (with `ffprobe`) to read the recording back
- `whisper-cli` from whisper.cpp on `PATH` and a ggml model: `ggml-small.en.bin` in `~/.local/share/comeback/models` or `~/.local/share/whisper-models` (`COMEBACK_WHISPER_MODEL_NAME` picks another name, `COMEBACK_WHISPER_MODEL` a file, `COMEBACK_WHISPER_CLI` another binary)
- `xclip` or `xsel` for the clipboard
- An X11 session with XInput 2.2 or newer (older servers hide a click's release while any window holds the pointer, so recording refuses to start input capture on them and says why), SHAPE, RandR and, for the daemon hotkey, XKB

If there is no microphone the recording is video only and `summary.md` says so. If transcription fails or the audio is silent, the bundle still gets built with `transcript: null` and a note.

Set `COMEBACK_AUDIO_SOURCE` to `none`, or to a PulseAudio source name to pick a microphone. `test`, `silence` and `quiet-noise` are generated sources for machines and tests without one.

## Privacy

A capture holds your screen and your voice. Bundles stay on your disk and nothing is uploaded. A captures directory that comeback creates, every work directory and every bundle are mode 0700, and the recorded media files are 0600, whatever your umask. A directory you made yourself keeps its mode. Treat them like screenshots of everything you had open, and do not commit or attach them anywhere public.

## Tests

`cargo test --workspace` includes end to end tests that start a private Xvfb display, map client windows that receive real pointer events, and record it. They never touch your own `DISPLAY`. They skip with a message when Xvfb, GStreamer or ffmpeg are missing. Set `COMEBACK_REQUIRE_X11_TESTS=1` to make that a failure, which is what CI does. The Whisper test that needs a real model and a speech WAV runs with `COMEBACK_TEST_WAV=<16 kHz mono wav> cargo test -p comeback --features whisper-tests`.

## Differences from the spec

The X11 slice follows the spec's pipeline but swaps four tools for what an Ubuntu 24.04 desktop has installed. They sit behind small modules, so the spec's choices can replace them later. [docs/plans/2026-10-22-comeback-x11.md](docs/plans/2026-10-22-comeback-x11.md) has the reasons.

| Spec | Now |
|---|---|
| `gstreamer-rs` pipeline | the same pipeline run by `gst-launch-1.0` |
| 4 fps `appsink` candidates | candidate frames decoded from the finished MP4 with `ffmpeg` |
| `whisper-rs` | `whisper-cli` |
| `arboard` | `xclip`, `xsel` or `wl-copy` |
| `tiny-skia` ink in an ARGB window | a window shaped to the stroke pixels, which also works without a compositor |

## Credit

The idea comes from [Scrawl](https://github.com/kitlangton/scrawl) by Kit Langton, a macOS app. comeback is a separate Rust implementation for Linux and Windows and contains no Scrawl code.

## License

MIT
