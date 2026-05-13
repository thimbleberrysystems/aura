#!/usr/bin/env bash
set -euo pipefail

# ── Model ──────────────────────────────────────────────────────────────────────
AURA_MODEL_NAME="${AURA_MODEL_NAME:-qwen2.5-coder:1.5b}"
export AURA_MODEL_NAME

# ── Model server address (host:port, used to bind Docker/Ollama) ───────────────
AURA_MODEL_ADDR="${AURA_MODEL_ADDR:-127.0.0.1:11434}"
# Full URL for the Rust genai client (must include /v1/ for Ollama OpenAI-compat path)
AURA_MODEL_ENDPOINT="${AURA_MODEL_ENDPOINT:-http://${AURA_MODEL_ADDR}/v1/}"
export AURA_MODEL_ADDR AURA_MODEL_ENDPOINT

# Optional API key (leave unset for local Ollama)
# export AURA_MODEL_API_KEY=

# parse host and port for Docker binding
if [[ "${AURA_MODEL_ADDR}" == *:* ]]; then
  AURA_MODEL_HOST="${AURA_MODEL_ADDR%%:*}"
  AURA_MODEL_PORT="${AURA_MODEL_ADDR##*:}"
else
  AURA_MODEL_HOST="${AURA_MODEL_ADDR}"
  AURA_MODEL_PORT="11434"
fi

# ── Summarization (only export if explicitly overriding) ──────────────────────
# Uncomment and set to override defaults (250 threshold, 3000s timeout, enabled)
# export AURA_DISABLE_SUMMARY=true
# export AURA_SUMMARIZE_THRESHOLD=500
# export AURA_SUMMARIZE_TIMEOUT_SECS=60

# ── Logging (only export if explicitly overriding) ─────────────────────────────
# export AURA_LOGGING=true

# ── Control TCP (only export if explicitly overriding) ────────────────────────
# export AURA_CONTROL_TCP=127.0.0.1:40001
# ──────────────────────────────────────────────────────────────────────────────

# Build if binary is missing
if [ ! -x ./target/debug/aura ]; then
  echo "Building aura..."
  cargo build --bins
fi

# Pull model if not already present. Use an ephemeral container that runs a local
# ollama server inside the container and pulls the model into the shared volume.
# Pull model if not already present (use a transient Ollama container)
if command -v docker >/dev/null 2>&1; then
  container_name="aura-ollama"
  echo "Ensuring model '${AURA_MODEL_NAME}' is available and model server running at ${AURA_MODEL_HOST}:${AURA_MODEL_PORT} (container: ${container_name})..."

  if docker ps --format '{{.Names}}' | grep -qx "${container_name}"; then
    echo "Container ${container_name} already running."
  elif docker ps -a --format '{{.Names}}' | grep -qx "${container_name}"; then
    echo "Starting existing container ${container_name}..."
    docker start "${container_name}" >/dev/null 2>&1 || true
  else
    echo "Starting new Ollama container ${container_name} (binding ${AURA_MODEL_HOST}:${AURA_MODEL_PORT}->11434)..."
    if ! docker run -d --name "${container_name}" -v "$HOME/.ollama:/root/.ollama" -p "${AURA_MODEL_HOST}:${AURA_MODEL_PORT}:11434" ollama/ollama serve >/dev/null 2>&1; then
      echo "Failed to start Ollama container ${container_name}." >&2
    fi
  fi

  # wait for ollama CLI inside container to be responsive
  for i in 1 2 3 4 5 6 7 8 9 10; do
    if docker exec "${container_name}" ollama list >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done

  if docker exec "${container_name}" ollama list 2>/dev/null | grep -q "^${AURA_MODEL_NAME}"; then
    echo "Model '${AURA_MODEL_NAME}' already present."
  else
    echo "Pulling model '${AURA_MODEL_NAME}' into local Ollama store..."
    if ! docker exec "${container_name}" ollama pull "${AURA_MODEL_NAME}"; then
      echo "Model pull failed for '${AURA_MODEL_NAME}'" >&2
      exit 1
    fi
  fi
else
  echo "docker not found; skipping model pull. If you expect to use a local Ollama, install docker or ensure the model is available." >&2
fi

export PATH="$(pwd)/target/debug:$PATH"
echo "Launching aura (model: ${AURA_MODEL_NAME}, endpoint: ${AURA_MODEL_ENDPOINT})"
./target/debug/aura

