#!/usr/bin/env bash
set -euo pipefail

# ── Set your model here ────────────────────────────────────────────────────────
AURA_MODEL="${AURA_MODEL:-qwen2.5-coder:3b}"
# ──────────────────────────────────────────────────────────────────────────────

export AURA_MODEL

# Build if binary is missing
if [ ! -x ./target/debug/aura ]; then
  echo "Building aura..."
  cargo build --bins
fi

# Pull model if not already present. Use an ephemeral container that runs a local
# ollama server inside the container and pulls the model into the shared volume.
# Pull model if not already present (use a transient Ollama container)
if command -v docker >/dev/null 2>&1; then
  temp_name="aura-temp-ollama-$$"
  echo "Ensuring model '${AURA_MODEL}' is available (using temporary container ${temp_name})..."

  if ! docker run -d --name "$temp_name" -v "$HOME/.ollama:/root/.ollama" ollama/ollama serve >/dev/null 2>&1; then
    echo "Failed to start temporary Ollama container for model pull." >&2
  else
    # wait for ollama CLI inside container to be responsive
    for i in 1 2 3 4 5 6 7 8 9 10; do
      if docker exec "$temp_name" ollama list >/dev/null 2>&1; then
        break
      fi
      sleep 1
    done

    if docker exec "$temp_name" ollama list 2>/dev/null | grep -q "^${AURA_MODEL}"; then
      echo "Model '${AURA_MODEL}' already present."
    else
      echo "Pulling model '${AURA_MODEL}' into local Ollama store..."
      if ! docker exec "$temp_name" ollama pull "${AURA_MODEL}"; then
        echo "Model pull failed for '${AURA_MODEL}'" >&2
        docker rm -f "$temp_name" >/dev/null 2>&1 || true
        exit 1
      fi
    fi

    docker rm -f "$temp_name" >/dev/null 2>&1 || true
  fi
else
  echo "docker not found; skipping model pull. If you expect to use a local Ollama, install docker or ensure the model is available." >&2
fi

export PATH="$(pwd)/target/debug:$PATH"
echo "Launching aura (model: ${AURA_MODEL})"
./target/debug/aura

