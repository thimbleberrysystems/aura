<#
PowerShell launcher for Ollama (Docker) + aura
- Attempts to install Docker if missing (winget on Windows)
- Runs Ollama in Docker (container name: aura-ollama)
- Adds project `target\debug` to PATH so `aura-cli` is available
- Runs `aura-cli` in foreground and stops the container on exit if started by this script
#>

param(
  [switch]$ForceInstall
)

$OLLAMA_IMAGE = $env:OLLAMA_IMAGE -or 'ollama/ollama:latest'
$OLLAMA_CONTAINER = $env:OLLAMA_CONTAINER -or 'aura-ollama'
$MODEL_VOLUME = $env:MODEL_VOLUME -or "$env:USERPROFILE\.ollama"
$OLLAMA_PORT = $env:OLLAMA_PORT -or 11434

$startedByScript = $false

function CmdExists($name) {
  return (Get-Command $name -ErrorAction SilentlyContinue) -ne $null
}

if (-not (CmdExists docker)) {
  Write-Host "Docker not found. Attempting install via winget..."
  if ($ForceInstall) {
    if (CmdExists winget) {
      winget install --id Docker.DockerDesktop -e --accept-package-agreements --accept-source-agreements
      Write-Host "Please start Docker Desktop manually if required and re-run this script."
      exit 0
    } else {
      Write-Error "winget not available. Install Docker Desktop manually."
      exit 1
    }
  } else {
    Write-Error "Docker not found. Rerun with -ForceInstall to attempt installation or install Docker manually."
    exit 1
  }
}

# Wait for Docker to be ready
try {
  docker info | Out-Null
} catch {
  Write-Error "Docker daemon not responding. Start Docker Desktop and retry."
  exit 1
}

Write-Host "Preparing Ollama container $OLLAMA_CONTAINER using image $OLLAMA_IMAGE..."
$existing = docker ps --format '{{.Names}}' | Select-String -Pattern "^$OLLAMA_CONTAINER$"
if ($existing) {
  Write-Host "Using existing running container $OLLAMA_CONTAINER"
} else {
  $existsStopped = docker ps -a --format '{{.Names}}' | Select-String -Pattern "^$OLLAMA_CONTAINER$"
  if ($existsStopped) {
    docker start $OLLAMA_CONTAINER | Out-Null
  } else {
    docker pull $OLLAMA_IMAGE
    docker run -d --name $OLLAMA_CONTAINER -p "$OLLAMA_PORT`:11434" -v "$MODEL_VOLUME:/root/.ollama" $OLLAMA_IMAGE | Out-Null
    $startedByScript = $true
  }
}

Write-Host "Waiting for Ollama to become healthy on http://localhost:$OLLAMA_PORT ..."
for ($i=0; $i -lt 30; $i++) {
  try {
    Invoke-RestMethod -Uri "http://localhost:$OLLAMA_PORT/api/tags" -UseBasicParsing -ErrorAction Stop | Out-Null
    Write-Host "Ollama healthy."
    break
  } catch {
    Start-Sleep -Seconds 1
  }
  if ($i -eq 29) {
    Write-Error "Ollama did not become healthy in time."
    if ($startedByScript) { docker stop $OLLAMA_CONTAINER | Out-Null }
    exit 1
  }
}

# Add target\debug to PATH for this session
$projBin = Join-Path (Get-Location) 'target\debug'
$env:PATH = "$projBin;$env:PATH"

Write-Host "Launching aura-cli (foreground). Press Ctrl-C to stop."
if (CmdExists aura-cli) {
  & aura-cli
} elseif (Test-Path -Path "target\debug\aura.exe" -PathType Leaf) {
  & "target\debug\aura.exe"
} elseif (Test-Path -Path "target\debug\aura" -PathType Leaf) {
  & "target\debug\aura"
} else {
  Write-Error "Could not find aura binary. Build with 'cargo build --bins' and retry."
  if ($startedByScript) { docker stop $OLLAMA_CONTAINER | Out-Null }
  exit 1
}

if ($startedByScript) {
  Write-Host "Stopping Ollama container $OLLAMA_CONTAINER..."
  docker stop $OLLAMA_CONTAINER | Out-Null
}

exit 0
