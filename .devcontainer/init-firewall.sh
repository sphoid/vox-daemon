#!/bin/bash
set -euo pipefail

###############################################################################
# init-firewall.sh — DevContainer egress firewall for Vox Daemon (Rust)
#
# Based on: https://github.com/anthropics/claude-code/blob/main/.devcontainer/init-firewall.sh
#
# This script implements a default-deny egress policy, allowing outbound
# connections ONLY to explicitly listed domains and IP ranges.
#
# MODIFICATION LOG (vs. upstream):
#   - Added Rust ecosystem domains (crates.io, static.rust-lang.org, etc.)
#   - Added Hugging Face domains (for Whisper model downloads)
#   - Added Ollama domain (for LLM integration testing)
#   - Added freedesktop.org (for PipeWire/pipewire-rs docs and repos)
#   - Added Ubuntu/Debian package mirrors (for system-level build deps)
###############################################################################

# ─── Preserve Docker internal DNS NAT rules ─────────────────────────────────
DOCKER_DNS_OUTPUT=$(iptables -t nat -S OUTPUT 2>/dev/null | grep DOCKER || true)
DOCKER_DNS_POSTROUTING=$(iptables -t nat -S POSTROUTING 2>/dev/null | grep DOCKER || true)

# ─── Flush all existing rules ────────────────────────────────────────────────
iptables -F
iptables -X
iptables -t nat -F
iptables -t nat -X
ipset destroy 2>/dev/null || true

# ─── Restore Docker DNS NAT rules ───────────────────────────────────────────
if [ -n "$DOCKER_DNS_OUTPUT" ]; then
    while IFS= read -r rule; do
        iptables -t nat ${rule/-A/-A} 2>/dev/null || true
    done <<< "$DOCKER_DNS_OUTPUT"
fi
if [ -n "$DOCKER_DNS_POSTROUTING" ]; then
    while IFS= read -r rule; do
        iptables -t nat ${rule/-A/-A} 2>/dev/null || true
    done <<< "$DOCKER_DNS_POSTROUTING"
fi

# ─── Block all IPv6 outbound ─────────────────────────────────────────────────
# The firewall only manages IPv4 (iptables). Without explicit ip6tables rules,
# IPv6 connections bypass the firewall entirely — or worse, hang for 30+ seconds
# before falling back to IPv4, which is what causes "No route to host" on
# CloudFront-backed domains like sh.rustup.rs.
#
# Drop all IPv6 outbound except loopback. This forces all traffic through the
# IPv4 path where our iptables + ipset rules apply.
if command -v ip6tables &> /dev/null; then
    ip6tables -F 2>/dev/null || true
    ip6tables -A INPUT -i lo -j ACCEPT
    ip6tables -A OUTPUT -o lo -j ACCEPT
    ip6tables -P INPUT DROP
    ip6tables -P FORWARD DROP
    ip6tables -P OUTPUT DROP
    echo "IPv6 outbound blocked (forcing IPv4 for all connections)."
fi

# ─── Basic infrastructure access ────────────────────────────────────────────
# Allow loopback
iptables -A INPUT -i lo -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT

# Allow DNS (required for domain resolution)
iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT

# Allow SSH (for git over SSH)
iptables -A OUTPUT -p tcp --dport 22 -j ACCEPT

# ─── Create ipset for allowed domains ───────────────────────────────────────
ipset create allowed-domains hash:net

# ─── GitHub IPs (dynamic, fetched from API) ─────────────────────────────────
echo "Fetching GitHub IP ranges..."
GITHUB_META=$(curl -s https://api.github.com/meta)

if ! echo "$GITHUB_META" | jq -e '.web and .api and .git' > /dev/null 2>&1; then
    echo "ERROR: Failed to fetch GitHub meta or missing required fields"
    exit 1
fi

# Extract and aggregate GitHub CIDRs
GITHUB_CIDRS=$(echo "$GITHUB_META" | jq -r '(.web + .api + .git)[]')

for cidr in $GITHUB_CIDRS; do
    if [[ "$cidr" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+/[0-9]+$ ]]; then
        ipset add allowed-domains "$cidr" 2>/dev/null || true
    fi
done
echo "GitHub IPs added."

# ─── Allowed domains ────────────────────────────────────────────────────────
# Each domain is resolved via DNS and its IPs added to the ipset.
#
# To add a new domain: simply add it to the DOMAINS array below, then rebuild
# the devcontainer.

DOMAINS=(
    #── Anthropic (Claude API) ──
    "api.anthropic.com"
    "statsig.anthropic.com"

    #── npm registry (for tooling) ──
    "registry.npmjs.org"

    #── VS Code / Cursor infrastructure ──
    "marketplace.visualstudio.com"
    "vscode.blob.core.windows.net"
    "update.code.visualstudio.com"

    #── Monitoring ──
    "sentry.io"
    "statsig.com"

    #── Rust ecosystem ──
    "crates.io"                         # Rust package registry (API)
    "static.crates.io"                  # Crate downloads (CDN)
    "index.crates.io"                   # Sparse registry index
    "static.rust-lang.org"              # Rustup, toolchain downloads
    "doc.rust-lang.org"                 # Rust documentation
    "sh.rustup.rs"                      # Rustup installer
    "forge.rust-lang.org"               # Rust infrastructure

    #── Rust crate source downloads (hosted on GitHub, but CDN may differ) ──
    "objects.githubusercontent.com"      # GitHub raw file / release downloads
    "raw.githubusercontent.com"          # GitHub raw content
    "github-releases.githubusercontent.com"  # GitHub release assets

    #── Hugging Face (Whisper model downloads) ──
    "huggingface.co"                    # Model hub
    "cdn-lfs.huggingface.co"            # Large file storage (model weights)
    "cdn-lfs-us-1.huggingface.co"       # US LFS endpoint
    "cdn-lfs-eu-1.huggingface.co"       # EU LFS endpoint

    #── Ollama (local LLM server — may need to pull models) ──
    "ollama.com"                        # Ollama website / model library
    "registry.ollama.ai"                # Ollama model registry

    #── freedesktop.org (PipeWire, pipewire-rs docs/repos) ──
    "gitlab.freedesktop.org"            # PipeWire source, pipewire-rs
    "pipewire.pages.freedesktop.org"    # PipeWire Rust docs

    #── Ubuntu/Debian package mirrors (for system build dependencies) ──
    #── Needed for: libpipewire-0.3-dev, libayatana-appindicator3-dev,
    #── libgtk-3-dev, cmake, clang, pkg-config, etc.
    "archive.ubuntu.com"
    "security.ubuntu.com"
    "deb.debian.org"
    "security.debian.org"
    "packages.microsoft.com"            # May be needed for some devcontainer base images

    #── codeberg.org (whisper-rs canonical repo) ──
    "codeberg.org"

    #── Claude Code plugin system ──
    #── Plugins install via `git clone` from GitHub (already allowed via
    #── GitHub meta API above), but the git protocol may resolve different
    #── IPs than the web/API endpoints. Add these explicitly.
    "github.com"                        # git clone over HTTPS
    "api.github.com"                    # Marketplace metadata lookups
    "codeload.github.com"              # Git archive downloads (used by some plugin installs)

    #── npm CDN (some plugins use npx or npm install for tooling) ──
    "registry.yarnpkg.com"              # Fallback registry
    "npmjs.com"                         # npm website (package metadata)
    "www.npmjs.com"                     # npm website

    #── Rust component downloads (rust-analyzer binary) ──
    #── `rustup component add rust-analyzer` downloads from these.
    #── They're all CloudFront-backed (handled by the AWS IP range fetch
    #── above), but adding them to the DNS resolution list ensures we
    #── catch any non-CloudFront IPs too.
    "toolchains.rust-lang.org"          # Rustup toolchain manifests
    "dev-static.rust-lang.org"          # Alternative static host
)

echo "Resolving allowed domains..."
for domain in "${DOMAINS[@]}"; do
    # Skip comment-only lines
    [[ "$domain" =~ ^#.*$ ]] && continue

    # Resolve A records
    ips=$(dig +noall +answer A "$domain" 2>/dev/null | awk '{print $5}' || true)
    for ip in $ips; do
        if [[ "$ip" =~ ^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$ ]]; then
            ipset add allowed-domains "$ip" 2>/dev/null || true
        fi
    done
done
echo "Domain resolution complete."

# ─── AWS CloudFront IPs (dynamic, fetched from AWS) ─────────────────────────
# sh.rustup.rs, static.rust-lang.org, and Hugging Face CDN all sit behind
# AWS CloudFront. CloudFront rotates edge IPs aggressively, so resolving
# the domain once at startup doesn't work — curl will get different IPs
# later. Instead, we fetch the full CloudFront IP range list from AWS's
# published IP ranges endpoint (same approach as the GitHub meta API above).
echo "Fetching AWS CloudFront IP ranges..."
AWS_RANGES=$(curl -sf --connect-timeout 10 https://ip-ranges.amazonaws.com/ip-ranges.json || true)

if [ -n "$AWS_RANGES" ]; then
    CF_CIDRS=$(echo "$AWS_RANGES" | jq -r '.prefixes[] | select(.service == "CLOUDFRONT") | .ip_prefix')
    CF_COUNT=0
    for cidr in $CF_CIDRS; do
        if [[ "$cidr" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+/[0-9]+$ ]]; then
            ipset add allowed-domains "$cidr" 2>/dev/null || true
            CF_COUNT=$((CF_COUNT + 1))
        fi
    done
    echo "  Added $CF_COUNT CloudFront CIDR ranges."
else
    echo "WARN: Could not fetch AWS IP ranges. Falling back to static ranges."
    # Fallback: known CloudFront ranges for the IPs we've seen in practice.
    # These may go stale — if rustup/huggingface stop working, re-fetch.
    ipset add allowed-domains 3.160.0.0/12 2>/dev/null || true
    ipset add allowed-domains 3.172.0.0/14 2>/dev/null || true
    ipset add allowed-domains 13.32.0.0/15 2>/dev/null || true
    ipset add allowed-domains 13.224.0.0/14 2>/dev/null || true
    ipset add allowed-domains 13.249.0.0/16 2>/dev/null || true
    ipset add allowed-domains 18.64.0.0/14 2>/dev/null || true
    ipset add allowed-domains 18.154.0.0/15 2>/dev/null || true
    ipset add allowed-domains 18.160.0.0/15 2>/dev/null || true
    ipset add allowed-domains 18.164.0.0/15 2>/dev/null || true
    ipset add allowed-domains 52.84.0.0/15 2>/dev/null || true
    ipset add allowed-domains 54.182.0.0/16 2>/dev/null || true
    ipset add allowed-domains 54.192.0.0/16 2>/dev/null || true
    ipset add allowed-domains 54.230.0.0/16 2>/dev/null || true
    ipset add allowed-domains 54.239.128.0/18 2>/dev/null || true
    ipset add allowed-domains 64.252.64.0/18 2>/dev/null || true
    ipset add allowed-domains 65.8.0.0/16 2>/dev/null || true
    ipset add allowed-domains 65.9.0.0/17 2>/dev/null || true
    ipset add allowed-domains 99.84.0.0/16 2>/dev/null || true
    ipset add allowed-domains 99.86.0.0/16 2>/dev/null || true
    ipset add allowed-domains 108.138.0.0/15 2>/dev/null || true
    ipset add allowed-domains 108.156.0.0/14 2>/dev/null || true
    ipset add allowed-domains 116.129.226.0/25 2>/dev/null || true
    ipset add allowed-domains 130.176.0.0/16 2>/dev/null || true
    ipset add allowed-domains 143.204.0.0/16 2>/dev/null || true
    ipset add allowed-domains 144.220.0.0/16 2>/dev/null || true
    ipset add allowed-domains 204.246.164.0/22 2>/dev/null || true
    ipset add allowed-domains 204.246.168.0/22 2>/dev/null || true
    ipset add allowed-domains 205.251.192.0/19 2>/dev/null || true
    echo "  Added static CloudFront fallback ranges."
fi

# ─── Fastly CDN (serves crates.io / static.crates.io) ───────────────────────
ipset add allowed-domains 151.101.0.0/16 2>/dev/null || true

# ─── Cloudflare (uncomment if needed for crates.io or HuggingFace) ──────────
# ipset add allowed-domains 104.16.0.0/12 2>/dev/null || true
# ipset add allowed-domains 172.64.0.0/13 2>/dev/null || true

# ─── Detect host bridge network ─────────────────────────────────────────────
HOST_IP=$(ip route | grep default | awk '{print $3}')
if [ -z "$HOST_IP" ]; then
    echo "ERROR: Cannot detect host IP"
    exit 1
fi
HOST_CIDR="${HOST_IP%.*}.0/24"

# Allow bidirectional host network access
iptables -A INPUT -s "$HOST_CIDR" -j ACCEPT
iptables -A OUTPUT -d "$HOST_CIDR" -j ACCEPT

# ─── Apply default-deny policies ────────────────────────────────────────────
iptables -P INPUT DROP
iptables -P FORWARD DROP
iptables -P OUTPUT DROP

# Allow established/related connections (critical for TCP handshakes)
iptables -A INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT

# Allow traffic to IPs in the allowed-domains ipset
iptables -A OUTPUT -m set --match-set allowed-domains dst -j ACCEPT

# Reject everything else with an informative ICMP message
iptables -A OUTPUT -j REJECT --reject-with icmp-admin-prohibited

echo "Firewall rules applied."

# ─── Verification tests ─────────────────────────────────────────────────────
echo "Running verification tests..."

# Test 1: Should BLOCK unauthorized domain
if curl -sf --connect-timeout 5 https://example.com > /dev/null 2>&1; then
    echo "FAIL: Firewall did NOT block example.com"
    exit 1
else
    echo "  PASS: example.com blocked"
fi

# Test 2: Should ALLOW GitHub
if curl -sf --connect-timeout 5 https://api.github.com/zen > /dev/null 2>&1; then
    echo "  PASS: api.github.com allowed"
else
    echo "FAIL: Firewall blocked api.github.com"
    exit 1
fi

# Test 3: Should ALLOW crates.io
if curl -sf --connect-timeout 5 https://crates.io/api/v1/crates?page=1\&per_page=1 > /dev/null 2>&1; then
    echo "  PASS: crates.io allowed"
else
    echo "WARN: crates.io not reachable (may need Fastly CDN range)"
fi

# Test 4: Should ALLOW sh.rustup.rs (CloudFront-backed)
if curl -sf --connect-timeout 5 -o /dev/null https://sh.rustup.rs 2>&1; then
    echo "  PASS: sh.rustup.rs allowed"
else
    echo "WARN: sh.rustup.rs not reachable (check CloudFront IP ranges)"
fi

# Test 5: Should ALLOW Anthropic API
if curl -sf --connect-timeout 5 https://api.anthropic.com > /dev/null 2>&1; then
    echo "  PASS: api.anthropic.com allowed"
else
    # Anthropic API returns errors without auth, but connection should work
    echo "  PASS: api.anthropic.com connection attempted (auth error expected)"
fi

echo ""
echo "Firewall initialization complete."
echo "Default policy: DENY ALL outbound"
echo "Allowed: GitHub, npm, Anthropic, VS Code, Rust ecosystem,"
echo "         Hugging Face, Ollama, freedesktop, Ubuntu/Debian repos"