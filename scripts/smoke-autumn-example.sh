#!/usr/bin/env bash
# Boots the real `autumn_app` example binary and checks it over HTTP, in the
# dev and prod profiles. Then it stops the app with SIGTERM. CI runs this
# script. Run it from any directory:
#
#   bash scripts/smoke-autumn-example.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
app_dir="$root/crates/autumn-plugin-san-mateo/examples/autumn_app"
work=$(mktemp -d)
pid=""

cleanup() {
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    kill -KILL "$pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  if [ -f "$work/app.log" ]; then
    echo "--- app log ---" >&2
    cat "$work/app.log" >&2
  fi
  exit 1
}

cargo build --locked --quiet -p autumn-plugin-san-mateo --example autumn_app
bin="${CARGO_TARGET_DIR:-$root/target}/debug/examples/autumn_app"

# Random secrets for this run. The checks make sure that the log does not
# show them.
key=$(openssl rand -base64 32)
secret="sk_example_$(openssl rand -hex 16)"
# The prod profile of autumn-web also needs a signing secret and trusted
# hosts.
signing=$(openssl rand -hex 32)

free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

# start PROFILE [VAR=VALUE...]: starts the app with a clean env.
start() {
  local profile=$1
  shift
  port=$(free_port)
  base="http://127.0.0.1:$port"
  env -i PATH="$PATH" HOME="$HOME" \
    AUTUMN_PROFILE="$profile" \
    AUTUMN_MANIFEST_DIR="$app_dir" \
    AUTUMN_SERVER__PORT="$port" \
    AUTUMN_SERVER__PRESTOP_GRACE_SECS=0 \
    AUTUMN_SAN_MATEO__SERVER_URL="$base" \
    SAN_MATEO_DEV_SECRET_KEY="$secret" \
    "$@" \
    "$bin" >"$work/app.log" 2>&1 &
  pid=$!
}

# wait_healthy: waits up to 60 s for the San Mateo health route.
wait_healthy() {
  for _ in $(seq 1 600); do
    kill -0 "$pid" 2>/dev/null || fail "the app stopped at boot"
    if curl -fsS --noproxy '*' "$base/san-mateo/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  fail "no health reply in 60 s"
}

# expect STATUS CURL_ARGS...: sends one request and checks the status.
expect() {
  local want=$1
  shift
  local got
  got=$(curl -s -o "$work/body" -w '%{http_code}' --noproxy '*' "$@")
  [ "$got" = "$want" ] || fail "want $want, got $got for $* : $(cat "$work/body")"
}

# stop_cleanly: sends SIGTERM and wants exit code 0 within 15 s.
stop_cleanly() {
  kill -TERM "$pid"
  for _ in $(seq 1 150); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -0 "$pid" 2>/dev/null && fail "no stop within 15 s of SIGTERM"
  local code=0
  wait "$pid" || code=$?
  pid=""
  [ "$code" -eq 0 ] || fail "exit code $code after SIGTERM"
}

no_secrets_in_log() {
  if grep -qF -e "$key" -e "$secret" -e "$signing" "$work/app.log"; then
    fail "the log shows a secret"
  fi
}

for profile in dev prod; do
  echo "autumn_app: profile $profile"
  start "$profile" SAN_MATEO_ENCRYPTION_KEY="$key" \
    AUTUMN_SECURITY__SIGNING_SECRET="$signing" \
    AUTUMN_SECURITY__TRUSTED_HOSTS__HOSTS=127.0.0.1
  wait_healthy
  auth="authorization: Bearer $secret"
  expect 200 "$base/"
  grep -q "autumn_app" "$work/body" || fail "GET / does not name the app"
  expect 401 -H 'accept: application/json' "$base/san-mateo/providers/acme-crm"
  expect 200 -H "$auth" "$base/san-mateo/providers/acme-crm"
  expect 201 -H "$auth" -H 'content-type: application/json' \
    -d '{"end_user":{"id":"u1"}}' "$base/san-mateo/connect/sessions"
  grep -qF "\"connect_link\":\"$base/san-mateo/connect?" "$work/body" \
    || fail "the connect link does not use the public URL"
  expect 404 -H 'accept: application/json' "$base/contacts/no-such-connection"
  if [ "$profile" = dev ]; then
    expect 200 "$base/actuator/health"
    grep -q san_mateo "$work/body" || fail "the health report has no san_mateo"
  fi
  stop_cleanly
  no_secrets_in_log
done

echo "autumn_app: a missing key stops the boot"
start dev
for _ in $(seq 1 300); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.1
done
kill -0 "$pid" 2>/dev/null && fail "the app runs with no encryption key"
code=0
wait "$pid" || code=$?
pid=""
[ "$code" -ne 0 ] || fail "exit code 0 with no encryption key"
grep -q SAN_MATEO_ENCRYPTION_KEY "$work/app.log" || fail "the error does not name the variable"
no_secrets_in_log

echo "autumn_app: OK"
