# aura

Minimal PTY example in Rust that spawns your shell on a slave PTY and forwards I/O.

Quickstart
 - Build and run in debug:

```bash
cargo run --bin aura
```

- Build release:

```bash
cargo build --release
./target/release/aura
```

Build and install

```bash
cargo build
# or
cargo install --path .
```

Running interactively

- Run directly from the repo (debug build):

```bash
./target/debug/aura
```

- Run inside an `xterm` (useful for testing as a separate window):

```bash
xterm -hold -T "Aura" -e ./target/debug/aura
```

Implementation notes

- gRPC-related status helper is in [src/grpc.rs](src/grpc.rs).
- Status/diagnostics helpers are in [src/tools/status.rs](src/tools/status.rs).

Testing

```bash
cargo test
```

VS Code integrated terminal

Add a profile and set it as the default in your `settings.json`:

```json
{
	"terminal.integrated.profiles.linux": {
		"aura": {
			"path": "./target/debug/aura",
			"args": []
		}
	},
	"terminal.integrated.defaultProfile.linux": "aura"
}
```

Contributing

Feel free to file issues or pull requests. For MCP changes, update the proto, run `cargo test`, and include integration tests for UDS/TCP where appropriate.

License

MIT/Apache-2.0 (see repository root for exact licensing terms)
