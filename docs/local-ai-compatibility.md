# Local AI compatibility checks

OffPDF has a local backend compatibility check for optional models. Settings
calls it when a model file is previewed, before the user imports it. The check
itself does not make a request, collect telemetry, inspect a PDF, or read the
model file.

The Rust/Tauri backend exposes:

```text
ai_check_compatibility(requirements) -> CompatibilityReport
```

The preview command also returns the same report for the selected file. For an
imported file the minimum disk requirement is the file size, the memory
requirement is left undeclared, and the architecture and runtime are treated as
unknown. The UI says that memory was not declared instead of inventing a value.

## Requirements from the caller

Model requirements are passed in, so the same check works for any future model
catalog or user-imported manifest:

```json
{
  "minMemoryBytes": 4294967296,
  "minDiskBytes": 8589934592,
  "architectures": ["x86_64", "aarch64"],
  "runtime": "any"
}
```

- `minMemoryBytes` is the minimum **available** memory, not total RAM.
- `minDiskBytes` is the free space required on the volume that holds the model
  store.
- An empty `architectures` list, or `["any"]`, means the model is
  architecture-independent.
- An empty, absent, or `"any"` runtime accepts any available runtime. A named
  runtime must match the runtime reported by the backend.

## Where Settings uses it

The local-model section is visible in Settings even when no model is installed.
After the user chooses a file, the facts panel shows its size, license status,
target store location, and compatibility verdict before the explicit Import
action. Import never starts automatically and the UI does not download a file.

The preview report uses:

- `minDiskBytes = file size` because the store needs room for one verified copy.
- `minMemoryBytes = 0` because imported files do not declare a memory minimum.
- An empty architecture list and an empty runtime, meaning "not declared".

## Results

Every result includes a short `reason`, an actionable `recommendation`, and the
facts that were used (OS, architecture, runtime, memory, and free disk space).

| Status | Meaning |
| --- | --- |
| `supported` | The OS, architecture, runtime, available memory, and free disk space meet the requirements, with headroom. |
| `supportedWithLimits` | The minimums are met, but memory or disk space has less than 25% headroom, or a system value could not be read. |
| `unsupported` | The OS, architecture, or runtime is unsupported or mismatched, or available memory/disk space is below the minimum. |

Unavailable system information is never treated as a successful check. It is
reported as `supportedWithLimits` with an explanation so the user can verify it
before installing. A probe failure never blocks or changes the existing PDF
tools.

## Current runtime status

The local-AI branch currently ships only the deterministic test backend
(`fake`), not a real inference runtime. Until a real runtime lands, the live
report is `unsupported` for every model and explains that the test backend is
not a real model runtime. Settings labels the backend as a test backend and
does not present it as a ready production model. Unit tests inject a capable
runtime so all three statuses and their boundaries are covered without
downloading weights.
