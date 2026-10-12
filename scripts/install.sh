#!/usr/bin/env sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 TPT Solutions
#
# tpt-homestack systemd installer (Linux, x86_64 or ARM64).
#   sudo ./scripts/install.sh              install from ./target/release
#   sudo INSTALL_BIN=/path/tpt-home ./scripts/install.sh
#   sudo ./scripts/install.sh --uninstall
set -eu

SERVICE=tpt-home
USER_NAME=tpt-home
GROUP_NAME=tpt-home
INSTALL_BIN="${INSTALL_BIN:-target/release/tpt-home}"
DATA_DIR=/var/lib/tpt-home
BIN_DIR=/usr/local/bin
REMOTE_ADDR="${REMOTE_ADDR:-127.0.0.1}"

if [ "$(id -u)" -ne 0 ]; then
  echo "run as root (sudo $0)"; exit 1
fi

if [ "${1:-}" = "--uninstall" ]; then
  systemctl disable --now "$SERVICE" 2>/dev/null || true
  rm -f "$BIN_DIR/tpt-home" "/etc/systemd/system/$SERVICE.service"
  systemctl daemon-reload
  echo "uninstalled (data left in $DATA_DIR)"
  exit 0
fi

[ -x "$INSTALL_BIN" ] || { echo "binary not found at $INSTALL_BIN (cargo build --release first)"; exit 1; }

id -u "$USER_NAME" >/dev/null 2>&1 || useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin "$USER_NAME"
mkdir -p "$DATA_DIR"
chown -R "$USER_NAME:$GROUP_NAME" "$DATA_DIR" 2>/dev/null || chown -R "$USER_NAME:" "$DATA_DIR"
install -m 0755 "$INSTALL_BIN" "$BIN_DIR/tpt-home"

cat > "/etc/systemd/system/$SERVICE.service" <<EOF
[Unit]
Description=tpt-homestack (single-binary home server platform)
After=network-online.target
Wants=network-online.target

[Service]
User=$USER_NAME
Group=$GROUP_NAME
WorkingDirectory=$DATA_DIR
Environment=TPT_HOME_DATA_DIR=$DATA_DIR
Environment=TPT_HOME_HOST=$REMOTE_ADDR
ExecStart=$BIN_DIR/tpt-home run
Restart=on-failure
RestartSec=3
# Lean is the point: keep the service small.
MemoryMax=256M
Nice=5

# Basic hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$DATA_DIR
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now "$SERVICE"

echo
echo "installed. next:"
echo "  sudo -u $USER_NAME $BIN_DIR/tpt-home --data-dir $DATA_DIR init"
echo "  systemctl status $SERVICE"
