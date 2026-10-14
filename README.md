# Aqua

Aqua is a modern Wayland desktop for Linux, with a clean workspace, fluid window management and a glass-inspired interface.

## Make yourself at home

- A familiar menu bar, Dock and application launcher.
- Workspaces, window tiling and an overview of your open windows.
- Quick settings, notifications and clipboard history.
- Built-in file manager, system settings and application store.
- Light and dark appearance, configurable shortcuts and multilingual support.
- Screen capture, recording and sharing.

## Try Aqua

Aqua is under active development. A nested session lets you try it without replacing your current desktop.

```sh
cargo build --release
./target/release/aqua --winit
```

See [building and installation](docs/build.md) for dependencies, packages and native sessions.

## Text assistance

Glass suggestions appear beside supported text fields. Use ↑/↓ to choose, Tab to insert, and Escape to dismiss. Accepted text is typed quickly rather than pasted all at once.

Text assistance uses native Wayland input support; it is not screen recognition. Passwords stay in your system password store and are supplied only after you select an entry. See [compatibility and privacy](docs/text-assistance.md).

## Contributing

Bug reports and contributions are welcome. Please include your distribution, graphics hardware and steps to reproduce the problem.

## License

[MIT](LICENSE). Third-party dictionary notices are included alongside their data.
