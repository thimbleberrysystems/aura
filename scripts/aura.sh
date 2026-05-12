#!/usr/bin/env bash
set -euo pipefail

# ── Set your model here ────────────────────────────────────────────────────────
AURA_MODEL="${AURA_MODEL:-qwen2.5-coder:3b}"
# ──────────────────────────────────────────────────────────────────────────────

export AURA_MODEL

# ── Model server address (host[:port]) ─────────────────────────────────────────
# Default to localhost:11434 (typical Ollama port). Can be e.g. 127.0.0.1:11434
AURA_MODEL_ADDR="${AURA_MODEL_ADDR:-127.0.0.1:11434}"
export AURA_MODEL_ADDR

# parse host and port
if [[ "${AURA_MODEL_ADDR}" == *:* ]]; then
  AURA_MODEL_HOST="${AURA_MODEL_ADDR%%:*}"
  AURA_MODEL_PORT="${AURA_MODEL_ADDR##*:}"
else
  AURA_MODEL_HOST="${AURA_MODEL_ADDR}"
  AURA_MODEL_PORT="11434"
fi
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
  echo "Ensuring model '${AURA_MODEL}' is available and model server running at ${AURA_MODEL_HOST}:${AURA_MODEL_PORT} (container: ${container_name})..."

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

  if docker exec "${container_name}" ollama list 2>/dev/null | grep -q "^${AURA_MODEL}"; then
    echo "Model '${AURA_MODEL}' already present."
  else
    echo "Pulling model '${AURA_MODEL}' into local Ollama store..."
    if ! docker exec "${container_name}" ollama pull "${AURA_MODEL}"; then
      echo "Model pull failed for '${AURA_MODEL}'" >&2
      exit 1
    fi
  fi
else
  echo "docker not found; skipping model pull. If you expect to use a local Ollama, install docker or ensure the model is available." >&2
fi

export PATH="$(pwd)/target/debug:$PATH"
echo "Launching aura (model: ${AURA_MODEL})"
./target/debug/aura

