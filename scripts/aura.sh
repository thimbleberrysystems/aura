#!/usr/bin/env bash
set -euo pipefail

# Cross-platform-ish launcher for Ollama (Docker) + aura
# - Attempts to install Docker if missing (Linux/macOS partial support)
# - Runs Ollama in Docker (container name: aura-ollama)
# - Exports PATH to include project `target/debug` so `aura-cli` is available
# - Runs `aura-cli` in foreground and stops the container on exit if started by this script

OLLAMA_IMAGE="${OLLAMA_IMAGE:-ollama/ollama:latest}"
OLLAMA_CONTAINER="${OLLAMA_CONTAINER:-aura-ollama}"
MODEL_VOLUME="${MODEL_VOLUME:-$HOME/.ollama}"
OLLAMA_PORT="${OLLAMA_PORT:-11434}"

started_by_script=false
INSTALL_MODE=false

# Simple arg parsing: only support --install for now
while [ "$#" -gt 0 ]; do
  case "$1" in
    --install)
      INSTALL_MODE=true
      shift
      ;;
    *)
      shift
      ;;
  esac
done

command_exists() { command -v "$1" >/dev/null 2>&1; }

install_docker_linux() {
  echo "Installing Docker via official convenience script (get.docker.com)..."
  # Use Docker's convenience installer which handles many distros.
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL https://get.docker.com -o get-docker.sh
    sudo sh get-docker.sh
    rm -f get-docker.sh
  else
    echo "curl required to run Docker convenience installer. Install curl or install Docker manually." >&2
    return 1
  fi
}

install_docker_macos() {
  echo "Attempting Docker install via brew (macOS)"
  if command -v brew >/dev/null 2>&1; then
    brew install --cask docker || brew install docker
    echo "If Docker Desktop was installed, please start it manually.";
  else
    echo "Homebrew not found. Install Homebrew or Docker Desktop manually." >&2
    return 1
  fi
}

if ! command_exists docker; then
  echo "docker not found — attempting install (this will try and may require sudo)..."
  if [ "$(uname -s)" = "Darwin" ]; then
    install_docker_macos || true
  else
    install_docker_linux || true
  fi
fi

if ! command_exists docker; then
  echo "docker still not available. Please install Docker and re-run." >&2
  exit 1
fi

# Ensure docker daemon responsive
if ! docker info >/dev/null 2>&1; then
  echo "Docker daemon not responding. Attempting to start (systemd)..."
  if command -v systemctl >/dev/null 2>&1; then
    sudo systemctl start docker || true
  fi
  if ! docker info >/dev/null 2>&1; then
    echo "Docker daemon not available. Start Docker and re-run." >&2
    exit 1
  fi
fi

echo "Preparing Ollama container (${OLLAMA_CONTAINER}) using image ${OLLAMA_IMAGE}..."
if docker ps --format '{{.Names}}' | grep -q "^${OLLAMA_CONTAINER}$"; then
  echo "Using existing running container ${OLLAMA_CONTAINER}."
else
  if docker ps -a --format '{{.Names}}' | grep -q "^${OLLAMA_CONTAINER}$"; then
    echo "Starting existing container ${OLLAMA_CONTAINER}..."
    docker start "${OLLAMA_CONTAINER}"
  else
    echo "Pulling image ${OLLAMA_IMAGE}..."
    docker pull "${OLLAMA_IMAGE}" || true
    echo "Creating container ${OLLAMA_CONTAINER}..."
    docker run -d --name "${OLLAMA_CONTAINER}" -p "${OLLAMA_PORT}:11434" -v "${MODEL_VOLUME}:/root/.ollama" "${OLLAMA_IMAGE}" || {
      echo "Failed to start Ollama container." >&2
      exit 1
    }
    started_by_script=true
  fi
fi

echo "Waiting for Ollama to become healthy on http://localhost:${OLLAMA_PORT} ..."
for i in $(seq 1 30); do
  if curl -sSf "http://localhost:${OLLAMA_PORT}/api/tags" >/dev/null 2>&1; then
    echo "Ollama is healthy."
    break
  fi
  sleep 1
  echo -n "."
  if [ "$i" -eq 30 ]; then
    echo
    echo "Ollama did not become healthy in time." >&2
    if [ "$started_by_script" = true ]; then
      echo "Stopping container ${OLLAMA_CONTAINER}..."
      docker stop "${OLLAMA_CONTAINER}" || true
    fi
    exit 1
  fi
done

# Optionally ensure the embedding model is available in Ollama (only in install mode)
if [ "$INSTALL_MODE" = true ]; then
  echo "Checking for nomic-embed-text model..."
  if ! curl -sSf "http://localhost:${OLLAMA_PORT}/api/models" 2>/dev/null | grep -q 'nomic-embed-text'; then
    echo "nomic-embed-text not found. Attempting to pull the model..."
    if command_exists ollama; then
      ollama pull nomic-embed-text || true
    elif docker ps --format '{{.Names}}' | grep -q "^${OLLAMA_CONTAINER}$"; then
      docker exec "${OLLAMA_CONTAINER}" ollama pull nomic-embed-text || true
    else
      echo "Could not pull model: no local ollama CLI and container not running." >&2
    fi
  else
    echo "nomic-embed-text model already present."
  fi
else
  echo "Install mode not enabled; skipping upfront model pull check. Use --install to pull models if needed."
fi

# Helper: ensure a named model is present in Ollama (checks env overrides)
pull_model() {
  local model="$1"
  if [ -z "$model" ]; then
    return 0
  fi
  echo "Checking for model '${model}'..."
  if curl -sSf "http://localhost:${OLLAMA_PORT}/api/models" 2>/dev/null | grep -q "${model}"; then
    echo "${model} already present."
    return 0
  fi
  echo "${model} not found. Attempting to pull..."
  if command_exists ollama; then
    ollama pull "${model}" || echo "ollama pull failed for ${model}" >&2
  elif docker ps --format '{{.Names}}' | grep -q "^${OLLAMA_CONTAINER}$"; then
    docker exec "${OLLAMA_CONTAINER}" ollama pull "${model}" || echo "container ollama pull failed for ${model}" >&2
  else
    echo "Could not pull model ${model}: no local ollama CLI and container not running." >&2
  fi
}

# Pull configured embedding and completion models if missing (env override supported)
EMBEDDING_MODEL="${AURA_EMBEDDING_MODEL:-nomic-embed-text}"
COMPLETION_MODEL="${AURA_COMPLETION_MODEL:-llama3}"
if [ "$INSTALL_MODE" = true ]; then
  pull_model "${EMBEDDING_MODEL}"
  pull_model "${COMPLETION_MODEL}"
  echo "Install mode complete: Docker and models are set up. Exiting as requested.";
  exit 0
else
  echo "Install mode not enabled; skipping model pulls. If models are missing the runtime may error or request you to pull them."
fi

# Add target/debug to PATH so aura-cli is available
export PATH="$(pwd)/target/debug:$PATH"

echo "Launching aura (foreground). Press Ctrl-C to stop."
if command_exists aura; then
  aura
elif [ -x ./target/debug/aura ]; then
  ./target/debug/aura
else
  echo "Could not find aura binary. Build with 'cargo build --bins' and retry." >&2
  if [ "$started_by_script" = true ]; then
    echo "Stopping container ${OLLAMA_CONTAINER}..."
    docker stop "${OLLAMA_CONTAINER}" || true
  fi
  exit 1
fi

# On exit, stop Ollama if we started it
if [ "$started_by_script" = true ]; then
  echo "Stopping Ollama container ${OLLAMA_CONTAINER}..."
  docker stop "${OLLAMA_CONTAINER}" || true
fi

exit 0
