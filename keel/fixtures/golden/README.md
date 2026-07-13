# Golden fixtures — Go kalam.dylib vs Nothing A059

Captured `2026-07-12` from the real device via `kalam-probe --golden`
(KALAM_DUMP_DIR hook in KalamKit). 22 payloads, numbered in call order:
the exact bytes the **Go** kernel emits. `keel-ffi` must reproduce these
modulo the documented deviations below.

Device: Nothing A059, MTP extension `microsoft.com: 1.0; android.com: 1.0;`
(the `android.com` string is what keel uses for runtime quirk detection —
plan §3.2, the libmtp approach — instead of a static quirk table).

## Normalize before diffing (volatile fields)
`elapsedTime`, `speed`, `dateAdded`, any `*Time` — timestamps and rates vary
per run. The conformance oracle normalizes these; everything else is exact.

## Intentional divergences (keel is CORRECT, Go is buggy — plan §3.5)
- **`0015.json` emoji filename.** Uploaded `note-🛳️.txt`; the Go kernel walks
  it back as `note-️.txt` — the 🛳 (U+1F6F3, a UTF-16 surrogate pair) is
  dropped by Go's UCS-2 string codec, leaving only the U+FE0F variation
  selector. keel-proto's real UTF-16 codec round-trips the full name, so
  conformance WILL differ here. This is the port fix, not a regression.
  Any other fixture containing an astral-plane character is the same case.

## FFI-observable Go bugs that keel PRESERVES (must match byte-for-byte)
- `data: null` (never `[]`) for an empty Walk.
- `dateAdded` = local wall-clock + `.SSS` + a literal `Z` (not real UTC).
- Error envelope: `errorType`/`error` empty strings = success.
- Storages use raw Go PascalCase (`Sid`, `Info`, `MaxCapability`, …);
  FileExists uses lowercase `fullpath`; everything else camelCase.
