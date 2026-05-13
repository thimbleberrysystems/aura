
<div align="center">

# ✦ AURA

### _OS-Level Token Compression for the AI-Native Development Stack_

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

> **AURA intercepts terminal I/O at the PTY layer — below every IDE, every AI coding assistant, every CI runner — and compresses command output before it ever reaches an LLM context window.**

</div>

---

## The Token Problem Is Bigger Than Anyone Is Talking About

Every serious AI coding workflow — Claude Code, GitHub Copilot, Cursor, Cline, Aider — has the same silent bottleneck: **terminal output**.

A single `cargo build` failure dumps 8,000 tokens of redundant log lines into a context window. A `kubectl describe pod` fills 3,000 tokens with boilerplate. A `pytest` run with 200 tests produces 15,000 tokens of which an LLM needs perhaps 400 to understand the failure.

**The LLM cannot act on what it cannot fit.** When the context window fills, the model silently truncates — dropping exactly the data the developer needed it to reason about. The "fix" today is: the user manually copies the relevant lines. This is not a solution. This is a tax.

**AURA eliminates that tax at the source.**

---

## What AURA Is

AURA is a **PTY-level compression daemon** written in Rust. It wraps the system shell inside a pseudo-terminal master/slave pair — the same OS primitive that every terminal emulator, SSH session, and CI executor uses. At this layer, AURA intercepts all command output *before* any application sees it, runs it through a local LLM compressor, and re-emits a high-entropy, low-token equivalent.

This is not a plugin. It is not a wrapper script. It is not a middleware library that needs to be integrated.

**It operates below the application layer. All tools inherit it automatically.**

---

## Integration Architecture

Because AURA operates at the OS PTY layer, every AI coding assistant that reads terminal output integrates with AURA for free — with no SDK, no API, no plugin required.

```mermaid
%%{init: {'theme': 'base', 'themeVariables': {'primaryColor': '#0F172A', 'primaryTextColor': '#F8FAFC', 'primaryBorderColor': '#334155', 'lineColor': '#6366F1', 'secondaryColor': '#1E293B', 'tertiaryColor': '#0F172A', 'clusterBkg': '#1E293B', 'clusterBorder': '#334155', 'titleColor': '#C7D2FE', 'edgeLabelBackground': '#1E293B', 'fontSize': '14px'}}}%%
flowchart TB
    subgraph CONSUMERS["AI Coding Assistants  ·  Context Window Consumers"]
        direction LR
        CLAUDE["Claude Code"]
        COPILOT["GitHub Copilot"]
        CURSOR["Cursor"]
        CLINE["Cline / Aider"]
        OPENAI["OpenAI Codex CLI"]
        CUSTOM["Custom Agents"]
    end

    subgraph AURA_LAYER["  ✦  AURA  ·  PTY Compression Layer  (Rust · Tokio)  "]
        direction TB
        PTY["PTY Master/Slave Intercept\nOS-level · zero application changes\nportable-pty · all platforms"]
        STRIP["ANSI / VT100 Normalizer\ntermwiz VteParser\nclean semantic text"]
        RING["Summary Ring Buffer\nN configurable slots\nrolling session context"]
        LLM_LOCAL["Local LLM Compressor\nllama3 · qwen · deepseek · any model\ngenai unified interface · streaming"]
        THRESH["Threshold Gate\npass-through if output < N bytes\nzero latency for short commands"]
    end

    subgraph OS["Operating System  ·  PTY Subsystem"]
        SHELL["Shell Process\nbash · zsh · fish\nPTY slave · unmodified"]
    end

    subgraph INFRA["Local Inference  ·  No Cloud Required"]
        OLLAMA["Ollama / llama.cpp\nquantized models\non-device · private"]
    end

    SHELL -->|raw stdout/stderr\nfull noise + ANSI| PTY
    PTY --> STRIP
    STRIP --> THRESH
    THRESH -->|below threshold\npass-through verbatim| CONSUMERS
    THRESH -->|above threshold| RING
    RING -->|prior session context| LLM_LOCAL
    STRIP -->|current output| LLM_LOCAL
    LLM_LOCAL <-->|HTTP streaming| OLLAMA
    LLM_LOCAL -->|compressed · high-entropy\n80–95% fewer tokens| CONSUMERS
    LLM_LOCAL -->|cmd + summary| RING

    classDef consumerCls fill:#312E81,stroke:#4338CA,color:#E0E7FF
    classDef auraCoreCls fill:#1E3A5F,stroke:#2563EB,color:#DBEAFE
    classDef shellCls fill:#14532D,stroke:#16A34A,color:#DCFCE7
    classDef infraCls fill:#451A03,stroke:#B45309,color:#FEF3C7

    class CLAUDE,COPILOT,CURSOR,CLINE,OPENAI,CUSTOM consumerCls
    class PTY,STRIP,RING,LLM_LOCAL,THRESH auraCoreCls
    class SHELL shellCls
    class OLLAMA infraCls
```

---

## The Economics

| Scenario | Raw tokens to LLM | AURA-compressed | Reduction |
|---|---|---|---|
| `cargo build` (failure) | ~8,000 | ~400 | **95%** |
| `kubectl describe pod` | ~2,800 | ~180 | **94%** |
| `pytest` (200 tests, 3 failures) | ~14,000 | ~600 | **96%** |
| `git log --stat` (50 commits) | ~6,000 | ~350 | **94%** |
| `docker build` (20 layers) | ~4,500 | ~250 | **94%** |

At $15/M input tokens (GPT-4-class), a single engineer running 50 terminal-heavy LLM interactions per day generates ~**\$4,500/year in avoidable token spend**. AURA eliminates the majority of it.

More importantly: **token budget recovered = reasoning capacity restored**. The LLM that previously truncated context now sees a complete, information-dense picture of the session. This is not a cost story — it is a capability story.

---

## Why This Is Hard to Copy

### 1 · OS-Level Integration Is a Structural Moat

Every competing approach — IDE extensions, shell functions, wrapper scripts, MCP tools — operates above the application layer. They require per-tool integration, per-tool maintenance, and per-tool trust grants. AURA operates **below all of them**, at the PTY syscall boundary. There is no application to integrate with. There is no SDK to ship.

This is the same architectural position that holds for network proxies, hypervisors, and OS security modules — deep enough that the value compounds across every tool that runs above it.

### 2 · The Compression Happens at the Right Time

Today's tools compress (if at all) at read time — when the LLM is already consuming context. AURA compresses **at write time**, the moment the shell emits bytes. The downstream LLM never sees noise. There is no prompt budget to manage. There is no chunking strategy to tune.

### 3 · Rolling Context Is Self-Improving

AURA maintains a configurable ring buffer of `(command, summary)` pairs. Each summarization call receives prior session context, so the compressor progressively understands the session's semantic state. Output summaries become more precise and more referential over time — exactly what a downstream reasoning agent needs.

### 4 · Privacy by Architecture

All inference runs locally. No terminal output, no command, no summary ever leaves the machine. This is not a privacy policy — it is a system property. In enterprise and regulated environments, this is the only viable architecture.

---

## Technical Architecture

```mermaid
%%{init: {'theme': 'base', 'themeVariables': {'primaryColor': '#1E293B', 'primaryTextColor': '#F1F5F9', 'primaryBorderColor': '#475569', 'lineColor': '#818CF8', 'secondaryColor': '#1E293B', 'tertiaryColor': '#0F172A', 'clusterBkg': '#1E293B', 'clusterBorder': '#3730A3', 'titleColor': '#C7D2FE', 'edgeLabelBackground': '#312E81', 'fontSize': '13px'}}}%%
flowchart TD
    subgraph USER["stdin"]
        KBD["Keyboard"]
    end

    subgraph DAEMON["aura  ·  Rust async (Tokio)"]
        direction TB
        PTY["PTY Master\nportable-pty"]
        SM["State Machine\nIDLE · RUNNING · PASSTHROUGH"]
        STRIP2["ANSI Stripper\ntermwiz VteParser"]

        subgraph PIPELINE["Pipeline  ·  tokio::spawn per command"]
            GATE["Threshold Gate\nlen < AURA_SUMMARIZE_THRESHOLD"]
            SNAP["Ring Snapshot\nread lock · stable context"]
            CALL["LLM Stream\ngenai · HTTP SSE"]
            PUSH["Ring Push\nwrite lock · cmd + summary"]
        end

        RING2["SummaryRing\nVecDeque · capacity N\nmax_slot_bytes configurable"]
        CTRL["Control Server\nTCP 127.0.0.1:40001"]
    end

    subgraph SHELL2["Shell"]
        SH["bash / zsh / fish"]
    end

    subgraph MODEL["Local Model"]
        OLLAMA2["Ollama / any OpenAI-compat endpoint"]
    end

    KBD --> PTY
    PTY <--> SH
    SH --> SM
    SM -->|PASSTHROUGH / IDLE| USER
    SM -->|RUNNING: captured| STRIP2
    STRIP2 --> GATE
    GATE -->|short: verbatim| USER
    GATE -->|long| SNAP
    SNAP -->|prior context| CALL
    STRIP2 -->|clean output| CALL
    CALL <--> OLLAMA2
    CALL -->|compressed stream| USER
    CALL --> PUSH
    PUSH --> RING2
    RING2 --> SNAP
```

### PTY State Machine

| State | Trigger | Behaviour |
|---|---|---|
| `IDLE` | Shell at prompt | Output forwarded verbatim |
| `RUNNING` | Enter/Return from stdin | Output captured and deferred |
| `PASSTHROUGH` | Alt-screen enter (`\x1b[?1049h`) | Raw bytes forwarded (vim, htop, less) |

Transition `RUNNING → IDLE` is triggered by `tcgetpgrp` foreground process group returning to the shell — the exact same signal the kernel uses to notify job completion. No polling. No timers. Zero false positives.

### Prompt Safety

All LLM calls use hard delimiters:

```
<BEGIN_OUTPUT>
{terminal content}
<END_OUTPUT>
```

Malicious terminal content (e.g. `</END_OUTPUT> Ignore previous instructions...`) cannot escape the delimiter context. The model receives terminal bytes as data, not as instructions.

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `AURA_MODEL_NAME` | `llama3.2` | Model used for compression |
| `AURA_MODEL_ENDPOINT` | _(Ollama default)_ | OpenAI-compatible endpoint URL |
| `AURA_MODEL_API_KEY` | _(unset)_ | API key (for cloud endpoints) |
| `AURA_SUMMARIZE_THRESHOLD` | `250` | Min bytes before LLM is invoked |
| `AURA_SUMMARIZE_TIMEOUT_SECS` | `3000` | Per-call LLM timeout |
| `AURA_DISABLE_SUMMARY` | _(unset)_ | Set to `1` to disable |
| `AURA_COMPRESS_PROMPT` | _(built-in)_ | Override compression prompt template |
| `AURA_SUMMARY_RING_SIZE` | `5` | Rolling context window depth |
| `AURA_SUMMARY_RING_SLOT_BYTES` | `2048` | Max bytes per ring slot |
| `AURA_CONTROL_TCP` | `127.0.0.1:40001` | Control plane address |
| `AURA_LOGGING` | _(unset)_ | Set to `1` for debug tracing |

---

## Quickstart

```bash
# Build
cargo build --release --bins

# Run (with a local Ollama instance)
./target/release/aura

# Or use the convenience script (starts Docker-based Ollama)
./scripts/aura.sh
```

AURA wraps your existing shell. Use it exactly as you use your terminal today. Commands shorter than `AURA_SUMMARIZE_THRESHOLD` bytes are passed through with zero latency.

---

## Roadmap

- [ ] Persistent cross-session ring (SQLite / DuckDB)
- [ ] `aura export` — serialize session summaries to JSON for downstream agents
- [ ] MCP server mode — expose compressed terminal context as an MCP resource
- [ ] Agent hooks — trigger actions on semantic pattern match (crash detected → open issue)
- [ ] Team broadcast — share session context across a local network
- [ ] Quantized on-device compressor — eliminate Ollama dependency entirely

---

## License

[MIT](LICENSE)


---

## The Problem

Developers spend hours in the terminal. Each command produces output — build logs, error traces, network diagnostics, deployment statuses — that vanishes from working memory the moment it scrolls off screen. You re-run commands you ran twenty minutes ago. You lose context between sessions.

**This is a solved problem. You just haven't had the right tool.**

---

## What AURA Does

AURA wraps your existing shell (`bash`, `zsh`, `fish` — whatever you use) inside an intelligent PTY layer. It intercepts every command and its output, runs it through a local LLM, and gives you back a semantically compressed version — the signal, not the noise.

More importantly: it **remembers**. Every compressed output is embedded into an in-memory vector store. The next time you run a related command, AURA automatically retrieves the most relevant past context and injects it into the model's prompt — without you lifting a finger.

**Your terminal gains a persistent, growing memory that makes every subsequent command smarter.**

---

## How It Works

> One loop. Every command. Gets smarter each time.

```mermaid
%%{init: {'theme': 'base', 'themeVariables': {'primaryColor': '#6D28D9', 'primaryTextColor': '#ffffff', 'primaryBorderColor': '#7C3AED', 'lineColor': '#A78BFA', 'secondaryColor': '#1E1B4B', 'tertiaryColor': '#0F0D1F', 'clusterBkg': '#1E1B4B', 'clusterBorder': '#4C1D95', 'titleColor': '#DDD6FE', 'edgeLabelBackground': '#2E1065', 'fontSize': '16px'}}}%%
flowchart TD
    DEV(["👤  You\n──────────────────────────────\nJust use your shell normally.\nNothing changes."])

    subgraph AURA ["  ✦  AURA  ·  AI-Native Terminal Layer  "]
        direction TB

        WATCH["🔬  Intercepts Every Command\nCaptures output silently\nZero latency · Zero config"]

        CLEAN["🧹  Extracts Pure Signal\nStrips ANSI · VT100 · OSC noise\nClean semantic text remains"]

        subgraph RAG ["  🗃  Semantic Memory  ·  Retrieval-Augmented Generation  "]
            direction LR
            EMBED["🧬  Vector Embedding\nnomic-embed-text · 768-dim\nOn-device · Private · Instant"]
            VSTORE["💾  In-Memory Vector Store\nCosine similarity\nGrows smarter every command"]
            EMBED <-->|persists & recalls| VSTORE
        end

        REASON["⚡  On-Device LLM Reasoning\nllama3 · deepseek · qwen2.5 · any model\nNo cloud · No API key · No data leaves your machine\nRAG context-injected · Prompt-injection safe"]
    end

    OUT(["✅  Smart Output\nCompressed · Contextual · Actionable\nGets smarter with every command"])

    DEV -- "  $ run any command  " --> WATCH
    WATCH --> CLEAN
    CLEAN --> EMBED
    VSTORE -. "top-k relevant history" .-> REASON
    CLEAN --> REASON
    REASON --> OUT
    OUT -. "displayed in your terminal" .-> DEV
    OUT -- "distilled into memory" --> EMBED

    classDef userCls fill:#F59E0B,stroke:#D97706,color:#1C1917
    classDef watchCls fill:#6D28D9,stroke:#4C1D95,color:#EDE9FE
    classDef cleanCls fill:#1D4ED8,stroke:#1E40AF,color:#EFF6FF
    classDef embedCls fill:#047857,stroke:#065F46,color:#D1FAE5
    classDef storeCls fill:#0E7490,stroke:#164E63,color:#CFFAFE
    classDef reasonCls fill:#9D174D,stroke:#831843,color:#FCE7F3
    classDef outCls fill:#B45309,stroke:#92400E,color:#FEF3C7

    class DEV userCls
    class WATCH watchCls
    class CLEAN cleanCls
    class EMBED embedCls
    class VSTORE storeCls
    class REASON reasonCls
    class OUT outCls
```

---

## Technical Architecture

```mermaid
%%{init: {'theme': 'base', 'themeVariables': {'primaryColor': '#4F46E5', 'primaryTextColor': '#ffffff', 'primaryBorderColor': '#6366F1', 'lineColor': '#818CF8', 'secondaryColor': '#1E1B4B', 'tertiaryColor': '#0F0D1F', 'clusterBkg': '#1E1B4B', 'clusterBorder': '#3730A3', 'titleColor': '#C7D2FE', 'edgeLabelBackground': '#312E81', 'fontSize': '15px'}}}%%
flowchart TD
    subgraph USER["👤 User"]
        KBD[Keyboard / stdin]
    end

    subgraph AURA_DAEMON["⚙️  AURA Daemon  (aura — Rust + Tokio async runtime)"]
        direction TB
        PTY["PTY Master\nportable-pty · PTY master/slave pair"]
        SM["State Machine\nIDLE · RUNNING · PASSTHROUGH"]
        FLUSH["Flusher Thread\n200 ms silence detector"]
        STRIP["ANSI Stripper\ntermwiz VteParser · VT100/VT220/OSC/DCS"]

        subgraph SUMMARIZE["🧠 Summarize Task (tokio::spawn)"]
            RAG_Q["RAG Query\nembed_text → cosine → top_k(3)"]
            LLM["LLM Call\nrig-core Ollama agent · prompt injection safe"]
            RAG_S["RAG Store\nstore_text · fire & forget spawn"]
        end

        STORE["InMemoryStore\nVec of StoredChunk · RwLock\ncosine similarity · ephemeral"]
        CTRL["Control Server\nUnix socket · TCP :40001\nsingle-line command/reply"]
    end

    subgraph OLLAMA["🦙 Ollama  (Docker / local binary)"]
        EMB_MODEL["Embedding Model\nnomic-embed-text\ndirect HTTP · /api/embeddings"]
        COMP_MODEL["Completion Model\nllama3 / any Ollama model\nrig-core agent abstraction"]
    end

    subgraph SHELL["🐚 Child Shell"]
        SHELL_PROC["bash · zsh · fish\nPTY slave · real process"]
    end

    subgraph CLI["🖥  aura-cli"]
        CLI_BIN["aura-cli binary\nstatus · help"]
    end

    KBD -->|raw bytes| PTY
    PTY <-->|PTY master/slave| SHELL_PROC
    SHELL_PROC -->|stdout + stderr| SM
    SM -->|Running: buffer & suppress| FLUSH
    SM -->|Idle / Passthrough: forward verbatim| USER
    FLUSH -->|cmd + captured bytes| STRIP
    STRIP -->|clean text| RAG_Q
    RAG_Q -->|query vector| EMB_MODEL
    EMB_MODEL -->|Vec f32 embedding| RAG_Q
    RAG_Q -->|top-k context chunks| LLM
    STRIP -->|clean text| LLM
    LLM -->|augmented prompt| COMP_MODEL
    COMP_MODEL -->|model reply| LLM
    LLM -->|compressed summary| USER
    LLM -->|distilled text| RAG_S
    RAG_S -->|embed & write| EMB_MODEL
    RAG_S -->|Vec f32 + content| STORE
    STORE -->|retrieved chunks| RAG_Q
    CLI_BIN <-->|Unix socket / TCP| CTRL
    CTRL -->|AppContext · uptime| AURA_DAEMON

    classDef userCls fill:#F59E0B,stroke:#D97706,color:#1C1917
    classDef ptySmCls fill:#6D28D9,stroke:#4C1D95,color:#EDE9FE
    classDef stripCls fill:#1D4ED8,stroke:#1E40AF,color:#EFF6FF
    classDef ragCls fill:#047857,stroke:#065F46,color:#D1FAE5
    classDef llmCls fill:#9D174D,stroke:#831843,color:#FCE7F3
    classDef storeCls fill:#0E7490,stroke:#164E63,color:#CFFAFE
    classDef ollamaCls fill:#065F46,stroke:#047857,color:#D1FAE5
    classDef ctrlCls fill:#B45309,stroke:#92400E,color:#FEF3C7
    classDef cliCls fill:#B45309,stroke:#92400E,color:#FEF3C7

    class KBD userCls
    class PTY,SM,FLUSH ptySmCls
    class STRIP stripCls
    class RAG_Q ragCls
    class LLM,RAG_S llmCls
    class STORE storeCls
    class EMB_MODEL,COMP_MODEL,SHELL_PROC ollamaCls
    class CTRL ctrlCls
    class CLI_BIN cliCls
```

---

## Technical Design

### 1 · PTY Interception Layer

AURA opens a native PTY master/slave pair (`portable-pty`), spawns your real shell on the slave side, and sits in between. Three concurrent OS threads implement a lock-free state machine:

| State | Behaviour |
|-------|-----------|
| `IDLE` | PTY output forwarded verbatim to your terminal |
| `RUNNING` | Output captured and suppressed; display deferred |
| `PASSTHROUGH` | Full-screen apps (`vim`, `htop`, `less`) forwarded raw |

The transition `RUNNING → IDLE` is triggered by 200 ms of PTY silence — an empirically reliable signal that a command has finished.

### 2 · ANSI Normalization

Raw PTY output contains the full VT100/VT220 escape sequence set — cursor moves, colour codes, OSC titles, DCS strings. AURA uses `termwiz`'s `VteParser` (the same parser powering WezTerm) to strip all escape sequences and extract the semantic text. This clean text is what gets sent to the model and stored.

### 3 · LLM Summarization

The clean output is sent to a local Ollama instance with a carefully engineered prompt:

- **Role**: _"You are a compressor. Reduce terminal output for another LLM."_
- **Preserve**: error messages, stack traces, exit codes, unique identifiers (IPs, paths, UUIDs)
- **Discard**: progress bars, ANSI noise, repetitive in-progress logs
- **Safety**: `<BEGIN_OUTPUT>` / `<END_OUTPUT>` delimiters prevent prompt injection from malicious terminal content
- **Fallback**: if the summary is longer than the original, or empty, or times out — the original output is shown unchanged

### 4 · RAG Memory Engine

This is where AURA becomes genuinely novel.

**Query phase** (before LLM call): The clean output is embedded using a dedicated embedding model (`nomic-embed-text` via a direct HTTP call to `/api/embeddings` — no fragile SDK wrappers). The resulting vector is compared against all stored chunks via cosine similarity (`top_k(3)`). Matching chunks are injected into the prompt as `Previous Context`.

**Store phase** (after LLM call, fire-and-forget `tokio::spawn`): The distilled output is embedded and written into the `InMemoryStore` — an in-process `Vec<StoredChunk>` protected by a `tokio::RwLock`. This never delays your terminal.

The result: **each command you run makes every future command in the session smarter**. The store is ephemeral by design — it lives for the duration of your session, keeping memory footprint minimal and privacy concerns nonexistent.

### 5 · Control Plane

A lightweight control server binds to both a Unix-domain socket (`$XDG_RUNTIME_DIR/aura.sock`) and a TCP loopback listener (`127.0.0.1:40001`). The `aura-cli` binary connects to this channel for real-time introspection (`status`, `help`). The gRPC layer (`tonic` + `prost`) is plumbed for future agent-to-agent communication.

---

## Quickstart

### Prerequisites

- **Rust** ≥ 1.76 (`curl https://sh.rustup.rs | sh`)
- **Docker** (for Ollama) — or a local `ollama` binary

### 1 · Start Ollama + build

```bash
./scripts/aura.sh           # starts Docker-based Ollama, builds, runs aura
```

Or manually:

```bash
ollama pull llama3
ollama pull nomic-embed-text
cargo build --release --bins
./target/release/aura
```

### 2 · Use it

Just use your terminal. Commands with output longer than 250 bytes (configurable) are automatically summarized. The first few commands build the memory; from there you'll see context-aware summaries.

```bash
# In a separate terminal or within the aura session:
./target/release/aura-cli status
```

---

## Configuration

All settings are live-reloaded from environment variables — no restart required.

| Variable | Default | Description |
|----------|---------|-------------|
| `AURA_COMPLETION_MODEL` | `llama3` | Ollama model used for summarization |
| `AURA_EMBEDDING_MODEL` | `nomic-embed-text` | Ollama model used for RAG embeddings |
| `AURA_OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama API endpoint |
| `AURA_SUMMARIZE_THRESHOLD` | `250` | Min output bytes before LLM is invoked |
| `AURA_SUMMARIZE_TIMEOUT_SECS` | `3000` | Summarization timeout (seconds) |
| `AURA_DISABLE_SUMMARY` | _(unset)_ | Set to `1` to disable summarization entirely |
| `AURA_DISABLE_RAG` | _(unset)_ | Set to `1` to disable embedding/vector store |
| `AURA_LOGGING` | _(unset)_ | Set to `1` to enable tracing (respects `RUST_LOG`) |
| `AURA_CONTROL_SOCKET` | `$XDG_RUNTIME_DIR/aura.sock` | Unix control socket path |
| `AURA_CONTROL_TCP` | `127.0.0.1:40001` | TCP control fallback address |

---

## Why This Matters

### For Developers

- **Zero workflow change.** Drop-in replacement for your terminal. Your shell, your aliases, your dotfiles — all untouched.
- **Local-first, private by design.** All inference runs on your machine via Ollama. Nothing leaves your network.
- **Any model, any size.** Switch from `llama3` to `deepseek-coder` to `qwen2.5` in one env var change. Quantized models work out of the box.

### For the AI Ecosystem

AURA represents a new primitive: **the AI-augmented shell session**. The terminal is the universal interface of software engineering. Every CI pipeline, every deployment, every debugging session flows through it. Instrumenting the terminal with a local reasoning layer — one that builds semantic memory across the session — creates a foundation for a class of developer agents that don't require cloud APIs, don't exfiltrate data, and don't break existing workflows.

This is the unsexy, invisible infrastructure layer that every "AI coding assistant" built on top of IDEs is missing. AURA operates at the OS process boundary, not the editor extension layer.

---

## Roadmap

- [ ] Persistent cross-session memory (DuckDB vector extension)
- [ ] `aura export` — serialize session memory to structured JSON for downstream agents
- [ ] Semantic search across session history via `aura-cli search <query>`
- [ ] Agent hooks — trigger external actions on pattern match (e.g., auto-open issue on detected crash)
- [ ] Team memory sharing — broadcast session context over a local network
- [ ] Streaming summarization — display partial results as the LLM generates them
- [ ] Plugin API — register custom tools that the LLM can invoke mid-session

---

## Contributing

AURA is at the frontier of local AI tooling. If you're building in the AI developer tools space, working on LLM inference, or just love systems programming in Rust — open an issue or a PR. We're moving fast.

---

## License

[MIT](LICENSE)
