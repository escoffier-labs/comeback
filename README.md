# comeback

Record a bug on your screen instead of describing it to your coding agent.

Press Alt+Shift+S and talk through the problem while you click around and draw on the screen. Press it again to stop. comeback turns the recording into a small bundle an agent can read: a contact sheet of the key moments, full-size keyframes, a transcript tied to those moments, and a JSON manifest. The text summary lands on your clipboard, ready to paste into Claude Code, Codex, OpenCode or any other agent.

In a kitchen, a comeback is a dish the guest sends back because something is wrong.

## Status

Early. Nothing records yet. The design lives in [docs/specs](docs/specs) and the build plans in [docs/plans](docs/plans). Linux comes first, with X11 and Wayland both supported in v1. Windows follows.

## Credit

The idea comes from [Scrawl](https://github.com/kitlangton/scrawl) by Kit Langton, a macOS app. comeback is a separate Rust implementation for Linux and Windows and contains no Scrawl code.

## License

MIT
