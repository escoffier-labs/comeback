---
name: comeback
description: Use when the user pastes a "[comeback ...]" block or points at a *.comeback bundle. It is a screen recording of a bug with their voice, clicks and red ink, compiled into key frames. Read it before answering.
---

# comeback bundles

A comeback bundle is a bug report the user recorded instead of typing: a few seconds or minutes of screen, their voice, where they clicked and what they circled in red. comeback has already picked the key moments. Read those first and leave the video alone.

## What the paste looks like

```
[comeback 0:20 · firefox "Settings" · /path/to/comeback-1792.comeback]
Note: no microphone, recorded video only (...)
- #1 (0:03) [clicked "New tab"] "Clicking new tab does nothing."
- #2 (0:05) [scene changed] "Then this panel shows up."
- #3 (0:09) [marked] "This label is wrong."
Contact sheet: /path/to/comeback-1792.comeback/contact-sheet.png
```

- The first line gives the length, the app and window title when the user recorded one window, and the bundle directory.
- `Note:` lines say what is missing: no microphone, no speech, transcription failed, click and ink capture unavailable. Read them first, because they tell you how far to trust the rest.
- Each `- #N (m:ss)` line is a key frame. In brackets is what happened: `clicked`, `dragged`, `marked` (the user drew red ink) or `scene changed`. A click with no label reads `clicked (x 0.50, y 0.25)`: fractions of the recorded area from its top left. In quotes is what the user said around that moment.

## How to read it

1. Open `contact-sheet.png` first. It shows every key frame in order, numbered like the list.
2. Open the full-size `frames/frame-NN.png` for any frame you need in detail. Click points are ringed and ink is red.
3. Trust the screen over the transcript when they disagree. The transcript is automatic speech recognition and mishears names and code.
4. A `[marked]` frame is the user pointing at something. What they circled is the subject of the words next to it.
5. Only look at `recording.mp4` or `audio.wav` when the frames do not answer the question. Use `ffmpeg -ss <seconds> -i recording.mp4 -frames:v 1 out.png` for one frame at a time.

## Files in a bundle

| File | What it is |
|---|---|
| `summary.md` | The paste text plus the absolute path of every key frame |
| `contact-sheet.png` | All key frames in one image |
| `frames/frame-NN.png` | Full-size key frames |
| `comeback.json` | The manifest: timeline of clicks and drags (positions are fractions of the recorded area), ink marks with their strokes, transcript with word times, notes |
| `recording.mp4` | The whole recording, H.264 with AAC audio, ink included |
| `audio.wav` | The microphone, 16 kHz mono 16-bit PCM |

## Commands

- `comeback inspect <bundle|latest>` prints the paste text again. Add `--json` for the manifest.

A bundle holds the user's screen and voice. Do not upload it, commit it or paste its contents anywhere else without being asked.
