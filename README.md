
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

## What Compression Looks Like

### Example 1 — `cargo build` (compilation failure)

<details>
<summary><strong>Raw output sent to LLM without AURA</strong> &nbsp;·&nbsp; ~7,800 tokens</summary>

```
   Compiling proc-macro2 v1.0.79
   Compiling unicode-ident v1.0.12
   Compiling quote v1.0.35
   Compiling syn v2.0.52
   Compiling serde_derive v1.0.197
   Compiling serde v1.0.197
   Compiling autocfg v1.2.0
   Compiling libc v0.2.153
   Compiling cfg-if v1.0.0
   Compiling once_cell v1.19.0
   Compiling smallvec v1.13.1
   ... (180 more crate compile lines) ...
   Compiling aura v0.1.0 (/home/user/aura)
error[E0308]: mismatched types
  --> src/compress.rs:47:18
   |
47 |         Ok(target)
   |            ^^^^^^ expected `ServiceTarget`, found `&ServiceTarget`
   |
note: expected struct `ServiceTarget`
      found reference `&ServiceTarget`

error[E0502]: cannot borrow `state` as immutable because it is also borrowed as mutable
  --> src/pty.rs:134:22
   |
131|         let mut s = state.lock().unwrap();
   |                     ----- mutable borrow occurs here
134|         debug!("{:?}", state);
   |                        ^^^^^ immutable borrow occurs here

For more information about these errors, try `rustc --explain E0308`.
error: could not compile `aura` (bin "aura") due to 2 previous errors
```
</details>

**AURA output** &nbsp;·&nbsp; ~120 tokens

```
cargo build FAILED (2 errors):
• src/compress.rs:47 — E0308: mismatched types; Ok(target) expects ServiceTarget, got &ServiceTarget
• src/pty.rs:134 — E0502: cannot borrow `state` as immutable; mutable borrow active from line 131
```

---

### Example 2 — `pytest` (test suite with failures)

<details>
<summary><strong>Raw output sent to LLM without AURA</strong> &nbsp;·&nbsp; ~11,200 tokens</summary>

```
============================= test session starts ==============================
platform linux -- Python 3.11.8, pytest-8.1.1, pluggy-1.4.0
rootdir: /home/user/myapp
collected 214 items

tests/test_auth.py ....................................                   [ 16%]
tests/test_billing.py .................F..........                       [ 29%]
tests/test_db.py .......................................                  [ 44%]
tests/test_api.py ...........F...................................F.......  [ 87%]
tests/test_utils.py ...........................                          [100%]

=================================== FAILURES ===================================
_________________________ test_stripe_webhook_signature ________________________

    def test_stripe_webhook_signature():
        payload = b'{"type": "payment_intent.succeeded"}'
>       assert verify_signature(payload, "whsec_test123") == True
E       AssertionError: assert False == True
E       +  where False = verify_signature(b'{"type": ...}', 'whsec_test123')

tests/test_billing.py:203: AssertionError

_________________________ test_user_rate_limit_redis ___________________________

    def test_user_rate_limit_redis():
>       client = redis.Redis(host='localhost', port=6379)
>       client.ping()
E       redis.exceptions.ConnectionError: Error 111 connecting to localhost:6379. Connection refused.

tests/test_api.py:87: ConnectionError
... (490 more lines of tracebacks, warnings, captured stdout) ...
```
</details>

**AURA output** &nbsp;·&nbsp; ~180 tokens

```
pytest: 211 passed, 3 failed (214 total)

FAILURES:
• tests/test_billing.py:203 — test_stripe_webhook_signature: verify_signature() returned False; expected True (whsec_test123)
• tests/test_api.py:87 — test_user_rate_limit_redis: Redis ConnectionError on localhost:6379 (connection refused — Redis not running?)
• tests/test_api.py:341 — test_admin_export_csv: AssertionError response.status_code == 403, got 500
```

---

### Example 3 — `kubectl describe pod` (crash loop)

<details>
<summary><strong>Raw output sent to LLM without AURA</strong> &nbsp;·&nbsp; ~2,600 tokens</summary>

```
Name:             api-server-7d9f8b-xk2p4
Namespace:        production
Priority:         0
Node:             gke-cluster-1-default-pool-3a8f2b1c/10.128.0.5
Start Time:       Tue, 13 May 2026 09:14:22 +0000
Labels:           app=api-server
                  pod-template-hash=7d9f8b
Annotations:      <none>
Status:           Running
IP:               10.100.4.17
IPs:
  IP:           10.100.4.17
Controlled By:    ReplicaSet/api-server-7d9f8b
Containers:
  api-server:
    Container ID:   containerd://a8f3c2...
    Image:          gcr.io/myproject/api-server:v2.4.1
    Image ID:       gcr.io/myproject/api-server@sha256:3f8a...
    Port:           8080/TCP
    Host Port:      0/TCP
    State:          Waiting
      Reason:       CrashLoopBackOff
    Last State:     Terminated
      Reason:       Error
      Exit Code:    1
      Started:      Tue, 13 May 2026 09:22:11 +0000
      Finished:     Tue, 13 May 2026 09:22:14 +0000
    Ready:          False
    Restart Count:  7
    Limits:
      cpu:     500m
      memory:  512Mi
    Requests:
      cpu:        250m
      memory:     256Mi
    Environment:
      DATABASE_URL:    <set to the key 'url' in secret 'db-secret'>    Optional: false
      REDIS_URL:       <set to the key 'url' in secret 'redis-secret'> Optional: false
      LOG_LEVEL:       info
    Mounts:
      /var/run/secrets/kubernetes.io/serviceaccount from kube-api-access-xyz (ro)
... (60 more lines of events, volume mounts, node info) ...
Events:
  Warning  BackOff    2m    kubelet  Back-off restarting failed container api-server
```
</details>

**AURA output** &nbsp;·&nbsp; ~90 tokens

```
Pod api-server-7d9f8b-xk2p4 (production): CrashLoopBackOff
• Image: gcr.io/myproject/api-server:v2.4.1
• Exit code 1 · 7 restarts · last crash duration: 3s (09:22:11–09:22:14)
• Node: 10.128.0.5 · Pod IP: 10.100.4.17
• Event: Back-off restarting failed container api-server (2m ago)
```

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

    classDef inputCls  fill:#FEF9C3,stroke:#CA8A04,color:#422006
    classDef ptyCls    fill:#DBEAFE,stroke:#2563EB,color:#1E3A8A
    classDef smCls     fill:#E0E7FF,stroke:#4338CA,color:#1E1B4B
    classDef stripCls  fill:#EDE9FE,stroke:#7C3AED,color:#2E1065
    classDef gateCls   fill:#D1FAE5,stroke:#059669,color:#064E3B
    classDef snapCls   fill:#CFFAFE,stroke:#0891B2,color:#164E63
    classDef callCls   fill:#FCE7F3,stroke:#DB2777,color:#500724
    classDef pushCls   fill:#FEE2E2,stroke:#DC2626,color:#450A0A
    classDef ringCls   fill:#FEF3C7,stroke:#D97706,color:#451A03
    classDef ctrlCls   fill:#F1F5F9,stroke:#64748B,color:#0F172A
    classDef shellCls  fill:#D1FAE5,stroke:#059669,color:#064E3B
    classDef modelCls  fill:#FEE2E2,stroke:#DC2626,color:#450A0A

    class KBD inputCls
    class PTY ptyCls
    class SM smCls
    class STRIP2 stripCls
    class GATE gateCls
    class SNAP snapCls
    class CALL callCls
    class PUSH pushCls
    class RING2 ringCls
    class CTRL ctrlCls
    class SH shellCls
    class OLLAMA2 modelCls
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

