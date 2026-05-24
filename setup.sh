#!/usr/bin/env bash
# setup.sh — one-time dev environment setup for Cameo on macOS (Apple Silicon + Intel).
#
# Installs nothing globally; it checks the toolchain, adds the Rust targets we
# ship for, and pulls JS + Cargo deps. Re-runnable any time.
#
# Usage:
#   ./setup.sh
#
# Windows: use setup.ps1 instead.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

info() { printf '→ %s\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '  \033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || die "this script is for macOS — on Windows run setup.ps1"

# ── toolchain ────────────────────────────────────────────────────────────────
info "checking toolchain"
command -v node  >/dev/null || die "Node.js not found — install Node 20+ (e.g. \`brew install node\`)"
command -v pnpm  >/dev/null || die "pnpm not found — \`npm i -g pnpm\` or \`brew install pnpm\`"
command -v rustc >/dev/null || die "Rust not found — install via https://rustup.rs"
command -v cargo >/dev/null || die "cargo not found — install Rust via https://rustup.rs"
ok "node $(node -v)  ·  pnpm $(pnpm -v)  ·  $(rustc --version)"

# Xcode Command Line Tools — required to compile the Rust backend / link the .app.
if xcode-select -p >/dev/null 2>&1; then
  ok "Xcode Command Line Tools present"
else
  warn "Xcode Command Line Tools missing — run: xcode-select --install"
fi

# ── Rust targets (we ship a universal binary: arm64 + x86_64) ─────────────────
info "ensuring Rust targets for arm64 + Intel"
if command -v rustup >/dev/null; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    if rustup target list --installed 2>/dev/null | grep -qx "$t"; then
      ok "$t"
    else
      info "adding $t"; rustup target add "$t"
    fi
  done
else
  warn "rustup not found — can't verify cross targets; \`build_release.sh\` needs both aarch64- and x86_64-apple-darwin"
fi

# ── dependencies ──────────────────────────────────────────────────────────────
info "pnpm install"
pnpm install
ok "JS dependencies installed"

info "warming the Cargo cache (cargo fetch)"
( cd src-tauri && cargo fetch >/dev/null ) && ok "Cargo dependencies fetched"

# ── Codex CLI (not bundled — Cameo drives the user's own, authenticated copy) ──
if command -v codex >/dev/null 2>&1; then
  ok "codex found: $(command -v codex)"
else
  warn "codex CLI not on PATH. Cameo needs it at runtime (it is NOT bundled):"
  warn "    npm i -g @openai/codex   # or your installer of choice"
  warn "    codex login              # ChatGPT subscription auth (no API key)"
fi

echo ""
ok "setup complete"
echo ""
echo "  next:"
echo "    pnpm tauri dev        # live dev with hot reload"
echo "    ./build_dev.sh        # build an unsigned debug .app to click"
echo "    ./build_release.sh    # build the signed, universal release .dmg"
echo ""
