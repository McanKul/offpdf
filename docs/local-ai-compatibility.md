# Local AI compatibility checks

OffPDF has a local backend compatibility check for optional models. The current
import/load UI does not call it yet; #82 remains open for that user-visible
preflight. The check itself does not make a request, collect telemetry, inspect
a PDF, or read the model file.

The Rust/Tauri backend exposes:

```text
ai_check_compatibility(requirements) -> CompatibilityReport
```

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
not a real model runtime. Unit tests inject a capable runtime so all three
statuses and their boundaries are covered without downloading weights.
