import '../src/rust/api/url_ext.dart' as frb;

/// URL rewrite helpers matching the original `UrlExt` extension API.
extension UrlExt on String {
  /// Rewrites a remote URL to the local proxy URL.
  ///
  /// Uses the active proxy port when initialized; otherwise default port 20250.
  String toLocalUrl() => frb.toLocalUrlStr(url: this);

  Uri toLocalUri() => Uri.parse(toLocalUrl());

  String toOriginUrl() => frb.toOriginUrlStr(url: this);

  Uri toOriginUri() => Uri.parse(toOriginUrl());

  String get generateMd5 => frb.generateMd5Str(input: this);

  String toSafeUrl() => frb.toSafeUrlStr(url: this);

  Uri toSafeUri() => Uri.parse(toSafeUrl());
}
