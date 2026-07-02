# ADR-0001: HLS cache key invariants

## Status

Accepted

## Context

HLS caching groups segment files under a playlist **directory key** (`task.hls_key` → `CacheKey.directory`). Precache, proxy playback, and removal must agree on this key or segments are written and read under different paths.

## Decision

1. **Single derivation**: `hls_key_for_url(url)` in `rust/src/ext/uri_ext.rs` — `uri_generate_md5(&to_safe_uri(url))`. Never use raw `generate_md5(url)` for playlist keys.
2. **Propagation**: When resolving master → media playlists, pass the caller's `hls_key` through nested `parse_playlist` calls; do not recompute from an intermediate URI.
3. **Proxy segments**: Segment proxy requests may override `hls_key` from the HLS registry (`find_segment_by_uri`); precache uses the playlist-level key for segment tasks via `segment_to_task`.
4. **Ranged MP4 entries**: `compute_entry` serializes query parameters in **sorted key order** before MD5 so cache keys are stable across process restarts.

## Consequences

- URLs with leading/trailing whitespace or encoding differences normalize once at the seam.
- `removeCacheByUrl` must receive the same optional `headers` used during precache when `custom_cache_id` host override is enabled.
- HLS precache remains a separate code path from `PrecacheOrchestrator` (see architecture review); merging is deferred.
