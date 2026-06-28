import 'package:flutter_test/flutter_test.dart';

/// Official Flutter sample used by Rust/Dart network E2E tests.
const butterflyMp4TestUrl =
    'https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4';

void main() {
  test(
    'butterfly mp4 E2E is opt-in in the example app',
    () {},
    skip:
        'Run: cd example && flutter test integration_test/butterfly_mp4_e2e_test.dart -d macos --dart-define=RUN_E2E=true',
  );
}
