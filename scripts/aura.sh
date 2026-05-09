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

command_exists() { command -v "$1" >/dev/null 2>&1; }

install_docker_linux() {
  echo "Installing Docker (attempt)..."
  if [ -f /etc/debian_version ]; then
    sudo apt-get update
    sudo apt-get install -y ca-certificates curl gnupg lsb-release
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
    sudo apt-get update
    sudo apt-get install -y docker-ce docker-ce-cli containerd.io
  elif [ -f /etc/fedora-release ] || command -v dnf >/dev/null 2>&1; then
    sudo dnf config-manager --add-repo=https://download.docker.com/linux/fedora/docker-ce.repo || true
    sudo dnf install -y docker-ce docker-ce-cli containerd.io
    sudo systemctl enable --now docker || true
  else
    echo "Automatic Docker install unsupported on this Linux distro. Install Docker manually." >&2
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

# Add target/debug to PATH so aura-cli is available
export PATH="$(pwd)/target/debug:$PATH"

echo "Launching aura-cli (foreground). Press Ctrl-C to stop."
if command_exists aura-cli; then
  aura-cli
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
