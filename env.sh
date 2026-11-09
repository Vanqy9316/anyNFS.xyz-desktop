# Source this to add the vendored Node.js toolchain to PATH for this workspace.
# Example: source env.sh
export PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/vendor/node-v26.13.0-linux-x64/bin:$PATH"
