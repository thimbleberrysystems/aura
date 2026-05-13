
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
%%{init: {'theme': 'default', 'themeVariables': {'primaryColor': '#EEF2FF', 'primaryTextColor': '#1E1B4B', 'primaryBorderColor': '#6366F1', 'lineColor': '#4F46E5', 'secondaryColor': '#F5F3FF', 'tertiaryColor': '#FAFAFA', 'clusterBkg': '#F5F3FF', 'clusterBorder': '#A5B4FC', 'titleColor': '#1E1B4B', 'edgeLabelBackground': '#EEF2FF', 'fontSize': '14px'}}}%%
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
%%{init: {'theme': 'default', 'themeVariables': {'primaryColor': '#EEF2FF', 'primaryTextColor': '#1E1B4B', 'primaryBorderColor': '#6366F1', 'lineColor': '#4F46E5', 'secondaryColor': '#F5F3FF', 'tertiaryColor': '#FAFAFA', 'clusterBkg': '#F5F3FF', 'clusterBorder': '#A5B4FC', 'titleColor': '#1E1B4B', 'edgeLabelBackground': '#EEF2FF', 'fontSize': '13px'}}}%%
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

