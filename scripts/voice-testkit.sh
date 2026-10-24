#!/usr/bin/env bash
# Локальный голосовой тесткит для ручной проверки oto: Voice WS, UDP,
# RTP и отправка Opus. DAVE выключен — это не тест MLS.
#
# Использование:
#   scripts/voice-testkit.sh            # поднять тесткит, напечатать env
#   scripts/voice-testkit.sh --with-app # поднять тесткит и клиент сразу
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LOG="${VOICE_TESTKIT_LOG:-/tmp/opencode/voice-testkit.log}"
CERT="${VOICE_TESTKIT_CERT:-/tmp/opencode/oto-testkit-cert.pem}"
BUNDLE="${VOICE_TESTKIT_CA_BUNDLE:-/tmp/opencode/oto-testkit-ca-bundle.pem}"
mkdir -p "$(dirname "$LOG")" "$(dirname "$CERT")"

APP="$ROOT/src-tauri/target/debug/discord-client"
if [[ ! -x "$APP" ]]; then
  echo "Нет debug-бинарника: $APP" >&2
  echo "Собери: (cd src-tauri && cargo build --locked)" >&2
  exit 1
fi

echo "Собираю voice-testkit..."
cargo build --quiet --manifest-path tools/voice-testkit/Cargo.toml

TESTKIT_CERT_PATH="$CERT" tools/voice-testkit/target/debug/voice-testkit >"$LOG" 2>&1 &
TESTKIT_PID=$!
APP_PID=""
cleanup() {
  if [[ -n "$APP_PID" ]]; then kill "$APP_PID" 2>/dev/null || true; fi
  kill "$TESTKIT_PID" 2>/dev/null || true
  wait "$TESTKIT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in $(seq 1 100); do
  if grep -q '^ENDPOINT=' "$LOG" 2>/dev/null; then break; fi
  if ! kill -0 "$TESTKIT_PID" 2>/dev/null; then
    echo "Тесткит не запустился:" >&2
    cat "$LOG" >&2
    exit 1
  fi
  sleep 0.1
done

ENDPOINT="$(grep '^ENDPOINT=' "$LOG" | head -1 | cut -d= -f2-)"
if [[ -z "${ENDPOINT:-}" ]]; then
  echo "Не дождался ENDPOINT от тесткита" >&2
  cat "$LOG" >&2
  exit 1
fi

# SSL_CERT_FILE заменяет системное хранилище, поэтому склеиваем тестовый
# сертификат с системным бандлом — иначе отвалится HTTPS к Discord.
cat "$CERT" /etc/ssl/certs/ca-certificates.crt > "$BUNDLE"

cat <<EOF

Тесткит: wss://$ENDPOINT (DAVE выключен — проверка транспорта/микрофона)
Сертификат: $CERT
Лог: $LOG

Запусти клиент с override (работает только в debug-сборке):
  DISCORD_VOICE_ENDPOINT_OVERRIDE=$ENDPOINT \\
  SSL_CERT_FILE=$BUNDLE \\
  $APP

Или сразу: $0 --with-app
Затем зайди в голосовой канал — микрофон пойдёт в тесткит.
Ctrl+C — остановить.

EOF

if [[ "${1:-}" == "--with-app" ]]; then
  if pgrep -x discord-client >/dev/null 2>&1; then
    echo "Клиент уже запущен; закрой его или запусти override вручную." >&2
    exit 1
  fi
  DISCORD_VOICE_ENDPOINT_OVERRIDE="$ENDPOINT" SSL_CERT_FILE="$BUNDLE" "$APP" &
  APP_PID=$!
  wait "$APP_PID" || true
else
  wait "$TESTKIT_PID" || true
fi
