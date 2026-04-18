#!/usr/bin/env bash
# =============================================================================
# QOMN v3.2 -- Installation Script
# Tested on: Ubuntu 24.04 LTS (AMD64, AVX2 required)
# Source:    https://github.com/condesi/qomn
# Author:    Percy Rojas Masgo <percy.rojas@condesi.pe>
# License:   Apache-2.0
# =============================================================================
set -e

INSTALL_DIR="/opt/qomn"
BINARY="/usr/local/bin/qomn"
SERVICE="qomn-nfpa"
PORT="9001"
DOMAIN=""          # optional: your domain for nginx SSL (leave empty to skip)
NGINX_PATH=""      # optional: URL path prefix, e.g. /qomn  (leave empty for /)

# ─── colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'; NC='\033[0m'
ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
info() { echo -e "${CYAN}[--]${NC} $*"; }
fail() { echo -e "${RED}[!!]${NC} $*"; exit 1; }

echo "================================================================="
echo " QOMN Installer"
echo " https://github.com/condesi/qomn"
echo "================================================================="

# ─── 1. system check ──────────────────────────────────────────────────────────
info "Checking system requirements..."

[[ "$(uname -s)" == "Linux" ]] || fail "Linux required"
[[ "$(uname -m)" == "x86_64" ]] || fail "x86_64 required for AVX2"

# check AVX2 support
if grep -q avx2 /proc/cpuinfo; then
    ok "AVX2 supported"
else
    fail "AVX2 not found in /proc/cpuinfo. QOMN requires AVX2 (Intel Haswell+ / AMD Ryzen+)"
fi

# minimum RAM: 2GB
RAM_KB=$(grep MemTotal /proc/meminfo | awk '{print $2}')
if [[ $RAM_KB -lt 2000000 ]]; then
    fail "Minimum 2GB RAM required (found: $(( RAM_KB / 1024 ))MB)"
fi
ok "RAM: $(( RAM_KB / 1024 / 1024 ))GB"

# ─── 2. system dependencies ───────────────────────────────────────────────────
info "Installing system packages..."
apt-get update -qq
apt-get install -y -qq \
    curl git build-essential pkg-config libssl-dev \
    nginx certbot python3-certbot-nginx 2>/dev/null || \
apt-get install -y -qq curl git build-essential pkg-config libssl-dev
ok "System packages installed"

# ─── 3. rust ──────────────────────────────────────────────────────────────────
if command -v cargo &>/dev/null || [[ -f "$HOME/.cargo/bin/cargo" ]]; then
    CARGO="${HOME}/.cargo/bin/cargo"
    RUSTC="${HOME}/.cargo/bin/rustc"
    [[ -f "$HOME/.cargo/bin/cargo" ]] || CARGO="$(command -v cargo)"
    [[ -f "$HOME/.cargo/bin/rustc" ]] || RUSTC="$(command -v rustc)"
    ok "Rust already installed: $($RUSTC --version)"
else
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    source "$HOME/.cargo/env"
    CARGO="$HOME/.cargo/bin/cargo"
    RUSTC="$HOME/.cargo/bin/rustc"
    ok "Rust installed: $($RUSTC --version)"
fi

export PATH="$HOME/.cargo/bin:$PATH"

# ─── 4. clone or update repo ──────────────────────────────────────────────────
if [[ -d "$INSTALL_DIR/.git" ]]; then
    info "Updating existing repo at $INSTALL_DIR..."
    git -C "$INSTALL_DIR" pull --ff-only
    ok "Repo updated"
else
    info "Cloning https://github.com/condesi/qomn → $INSTALL_DIR ..."
    git clone https://github.com/condesi/qomn "$INSTALL_DIR"
    ok "Repo cloned"
fi

# ─── 5. build ─────────────────────────────────────────────────────────────────
info "Building QOMN (release mode, AVX2 target)..."
cd "$INSTALL_DIR"
RUSTFLAGS="-C target-cpu=native" $CARGO build --release 2>&1 | tail -5
ok "Build complete"

# ─── 6. install binary ────────────────────────────────────────────────────────
cp -f "$INSTALL_DIR/target/release/qomn" "$BINARY"
chmod +x "$BINARY"
ok "Binary installed: $BINARY"
$BINARY --version 2>/dev/null || info "(--version not implemented, binary is present)"

# ─── 7. stdlib check ──────────────────────────────────────────────────────────
STDLIB="$INSTALL_DIR/stdlib/all_domains.qomn"
if [[ ! -f "$STDLIB" ]]; then
    fail "stdlib not found at $STDLIB -- check repo structure"
fi
ok "Stdlib found: $STDLIB"

# ─── 8. systemd service ───────────────────────────────────────────────────────
info "Creating systemd service: $SERVICE ..."
cat > "/etc/systemd/system/${SERVICE}.service" <<EOF
[Unit]
Description=QOMN Plan Engine (Port $PORT)
After=network.target

[Service]
Environment=QOMNI_PATCH_ENABLED=0
Type=simple
User=root
WorkingDirectory=$INSTALL_DIR
ExecStart=$BINARY serve $STDLIB $PORT
Restart=always
RestartSec=3
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "$SERVICE"
systemctl restart "$SERVICE"
sleep 2

if systemctl is-active --quiet "$SERVICE"; then
    ok "Service $SERVICE is running"
else
    fail "Service failed to start. Check: journalctl -u $SERVICE -n 50"
fi

# ─── 9. nginx config (optional) ───────────────────────────────────────────────
if [[ -n "$DOMAIN" ]]; then
    info "Configuring nginx for $DOMAIN ..."
    PREFIX="${NGINX_PATH:-/qomn}"
    # strip trailing slash
    PREFIX="${PREFIX%/}"

    NGINX_CONF="/etc/nginx/sites-available/qomn"
    cat > "$NGINX_CONF" <<EOF
server {
    listen 80;
    server_name $DOMAIN;

    location ${PREFIX}/demo/ {
        alias $INSTALL_DIR/demo/;
        index index.html;
        add_header Cache-Control "no-cache";
    }

    location ${PREFIX}/api/ {
        proxy_pass http://127.0.0.1:${PORT}/;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_read_timeout 120s;
        proxy_send_timeout 120s;
        add_header Access-Control-Allow-Origin *;
    }
}
EOF

    ln -sf "$NGINX_CONF" /etc/nginx/sites-enabled/qomn
    nginx -t && systemctl reload nginx
    ok "Nginx configured for $DOMAIN${PREFIX}/api/"

    info "Run to enable SSL: certbot --nginx -d $DOMAIN"
else
    info "Nginx skipped (DOMAIN not set). API available at: http://localhost:$PORT"
fi

# ─── 10. firewall: open port if ufw active ────────────────────────────────────
if command -v ufw &>/dev/null && ufw status | grep -q "Status: active"; then
    ufw allow "$PORT/tcp" comment "QOMN API" &>/dev/null
    ok "UFW: port $PORT opened"
fi

# ─── 11. verify ───────────────────────────────────────────────────────────────
echo ""
echo "================================================================="
echo " VERIFICATION"
echo "================================================================="

info "Testing /status endpoint..."
STATUS=$(curl -sf "http://127.0.0.1:$PORT/status" 2>/dev/null || curl -sf "http://127.0.0.1:$PORT/simulation/status" 2>/dev/null || echo "")
if echo "$STATUS" | grep -q '"ok":true'; then
    ok "API responding: $(echo $STATUS | python3 -c 'import json,sys; d=json.load(sys.stdin); print(f"per_s={d.get(\"per_s\",\"?\")}, valid_frac={d.get(\"valid_frac\",\"?\")}")' 2>/dev/null || echo "ok")"
else
    info "Status endpoint path may differ. Try manually:"
    echo "    curl http://127.0.0.1:$PORT/simulation/status"
fi

info "Testing plan execution (NFPA 20 fire pump)..."
RESULT=$(curl -sf -X POST "http://127.0.0.1:$PORT/plan/execute" \
    -H "Content-Type: application/json" \
    -d '{"plan":"plan_pump_sizing","params":{"Q_gpm":500,"P_psi":100,"eff":0.75}}' 2>/dev/null || echo "")
if echo "$RESULT" | grep -q "16.835"; then
    ok "plan_pump_sizing: 500 GPM / 100 PSI / 0.75 eff = 16.835017 HP (correct)"
elif echo "$RESULT" | grep -q '"ok":true'; then
    ok "plan_pump_sizing responded (check value manually)"
else
    info "Plan test inconclusive. Try manually:"
    echo "    curl -X POST http://127.0.0.1:$PORT/plan/execute \\"
    echo "      -H 'Content-Type: application/json' \\"
    echo "      -d '{\"plan\":\"plan_pump_sizing\",\"params\":{\"Q_gpm\":500,\"P_psi\":100,\"eff\":0.75}}'"
fi

echo ""
echo "================================================================="
echo " QOMN INSTALLED SUCCESSFULLY"
echo "================================================================="
echo ""
echo "  API (local):   http://127.0.0.1:$PORT"
[[ -n "$DOMAIN" ]] && echo "  API (public):  https://$DOMAIN${NGINX_PATH:-/qomn}/api/"
echo "  Service:       systemctl status $SERVICE"
echo "  Logs:          journalctl -u $SERVICE -f"
echo "  Stdlib:        $STDLIB"
echo "  Source:        https://github.com/condesi/qomn"
echo ""
echo "  Quick verification:"
echo "    curl http://127.0.0.1:$PORT/simulation/status"
echo "    curl http://127.0.0.1:$PORT/plans"
echo ""
