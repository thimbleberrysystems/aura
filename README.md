
# aura

Minimal PTY example in Rust that spawns your shell on a slave PTY and forwards I/O. This repository provides:

- `aura` — the PTY daemon that runs your shell and exposes a small control API.
- `aura-cli` — a lightweight client that talks to the control channel to request commands such as `status`.

Quickstart

- Build both binaries in debug:

```bash
cargo build --bins
```

- Run the daemon:

```bash
./target/debug/aura
```

- In another shell (or inside the `aura` session) invoke the client:

```bash
./target/debug/aura-cli status
```

Release build

```bash
cargo build --release --bins
./target/release/aura
./target/release/aura-cli status
```

Installation

Copy `aura-cli` to a directory on your `PATH` (example for user install):

```bash
mkdir -p "$HOME/.local/bin"
cp target/release/aura-cli "$HOME/.local/bin/"
```


Control channel

By default `aura` exposes a control channel using a Unix-domain socket on Unix/macOS and a TCP loopback listener as a portable fallback.

- UDS path: `$AURA_CONTROL_SOCKET` or `${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/aura.sock`
- TCP: `$AURA_CONTROL_TCP` or `127.0.0.1:40001`

Environment variables

The following environment variables control `aura` behavior:

- `AURA_CONTROL_SOCKET`: path to Unix-domain socket for control (overrides default).
- `AURA_CONTROL_TCP`: TCP address for control fallback (default `127.0.0.1:40001`).
- `AURA_LOGGING`: set to `1`, `true`, or `yes` to enable logging; otherwise logging is suppressed by default. When enabled, `aura` will respect `RUST_LOG` for fine-grained filtering.
- `XDG_RUNTIME_DIR`: used to locate the default UDS path when present.

Examples

- Request status via the client:

```bash
./target/debug/aura-cli status
# or, if installed to PATH
aur a-cli status
```

- Quick debug using `socat` (UDS):

```bash
echo -n "status\n" | socat - UNIX-CONNECT:"${AURA_CONTROL_SOCKET:-/run/user/$(id -u)/aura.sock}"
```

Implementation notes

- Control server: `src/cli_server.rs` (UDS + TCP listener, simple one-line command protocol).
- Client: `src/cli_client.rs` (built as `aura-cli`).
- Status/diagnostics helpers: `src/tools/status.rs`.

Testing

```bash
cargo test
```

Contributing

Feel free to file issues or pull requests. Run `cargo test` and `cargo build --bins` before submitting changes.

License

MIT/Apache-2.0 (see repository root for exact licensing terms)

Ingestion (VT100 → embeddings → SQLite vector store)
---------------------------------------------------

- The daemon sanitizes PTY output (VT100 parsing via `termwiz`) and produces
	`SanitizedChunk` items containing `session_id`, `ts`, and `text`.
- Ingestion is enabled by default and configurable via environment variables
	and `config/aura.toml` entries documented in `src/cfg.rs`.
- Embeddings are produced using the `rig` providers (default: Ollama
	`nomic-embed-text`, 768 dims). The embedding provider and dims can be
	overridden via `AURA_EMBEDDING_MODEL` and `AURA_EMBEDDING_DIMS`.
- Storage uses `rig-sqlite` backed by the `sqlite-vec` extension. The SQLite
	file path is controlled by `AURA_SQLITE_PATH` (default `./aura.db`).

Files of interest:

- `src/ingest/mod.rs` — ingestion worker: batching, embedding, and storage.
- `src/cfg.rs` — configuration helpers and environment variable defaults.
- `Cargo.toml` — added `rig-sqlite`, `sqlite-vec`, and `tokio-rusqlite`.

Notes:

- A running Ollama server is required to use the default embedding provider.
- The code initializes `sqlite-vec` before opening the database so that the
	vector extension is available to the `rig-sqlite` store.

If you'd like, I can add a small integration test or a startup README snippet
that demonstrates ingest end-to-end (requires Ollama running).
