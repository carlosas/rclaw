#!/bin/bash
set -e

# Get the directory where the script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

# Build the shared base image
echo "Building rclaw-base image..."
docker build -t rclaw-base:latest -f Dockerfile.base .

# Build Gemini-specific image
echo "Building rclaw-agent-gemini image..."
docker build -t rclaw-agent-gemini:latest -f gemini/Dockerfile .

# Tag as the default agent for now (to avoid breaking current Rust code)
docker tag rclaw-agent-gemini:latest rclaw-agent:latest

echo "Images built successfully!"
