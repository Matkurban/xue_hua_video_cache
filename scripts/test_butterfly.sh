#!/usr/bin/env bash
# Fast smoke test: curl reachability + Rust unit tests (no network E2E by default).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
URL="https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4"

echo "==> butterfly.mp4 reachable (HEAD)"
curl -sfI "$URL" | grep -i 'accept-ranges: bytes' >/dev/null

echo "==> Rust unit tests (ignored network E2E excluded)"
(cd "$ROOT/rust" && cargo test -q -- --skip butterfly_mp4_precache)

/// Opt-in Dart network E2E (requires device + network):
///   cd example && flutter test integration_test/butterfly_mp4_e2e_test.dart -d macos --dart-define=RUN_E2E=true
echo "==> OK: smoke passed in under 1 minute"
echo "    Opt-in Rust E2E: cd rust && cargo test --ignored butterfly"
echo "    Opt-in Dart E2E: cd example && flutter test integration_test/butterfly_mp4_e2e_test.dart -d macos --dart-define=RUN_E2E=true"
