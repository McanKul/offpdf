//! Unit tests for local AI hardware/resource compatibility (issue #82).
//!
//! The evaluator is pure, so boundary values and unavailable facts are tested
//! without depending on the machine running the tests. One probe smoke test
//! exercises the real host reader without asserting volatile values.

use crate::ai::compatibility::{
    evaluate, probe, CompatibilityReport, CompatibilityStatus, ModelRequirements,
    RuntimeCapabilities, SystemFacts,
};

const GIB: u64 = 1024 * 1024 * 1024;

fn requirements() -> ModelRequirements {
    ModelRequirements {
        min_memory_bytes: 4 * GIB,
        min_disk_bytes: 8 * GIB,
        architectures: Vec::new(),
        runtime: None,
    }
}

fn supported_facts() -> SystemFacts {
    SystemFacts {
        os: Some("windows".to_string()),
        arch: Some("x86_64".to_string()),
        runtime: Some(RuntimeCapabilities {
            id: "llama.cpp".to_string(),
            available: true,
        }),
        total_memory_bytes: Some(16 * GIB),
        available_memory_bytes: Some(8 * GIB),
        available_disk_bytes: Some(64 * GIB),
    }
}

fn assert_actionable(report: &CompatibilityReport, label: &str) {
    assert!(
        !report.reason.trim().is_empty(),
        "{label}: reason must be non-empty"
    );
    assert!(
        !report.recommendation.trim().is_empty(),
        "{label}: recommendation must be non-empty"
    );
}

#[test]
fn compat_supported() {
    let report = evaluate(&requirements(), &supported_facts());
    assert_eq!(
        report.status,
        CompatibilityStatus::Supported,
        "compat-supported: generous memory and disk must be Supported"
    );
    assert_actionable(&report, "compat-supported");
    assert_eq!(report.operating_system.as_deref(), Some("windows"));
    assert_eq!(report.architecture.as_deref(), Some("x86_64"));
    assert_eq!(report.runtime_id.as_deref(), Some("llama.cpp"));
    assert!(report.runtime_available);
    assert_eq!(report.total_memory_bytes, Some(16 * GIB));
}

#[test]
fn compat_exact_minimums_are_limited_not_unsupported() {
    let mut facts = supported_facts();
    facts.available_memory_bytes = Some(4 * GIB);
    facts.available_disk_bytes = Some(8 * GIB);

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::SupportedWithLimits,
        "compat-boundary: exactly meeting minimums must be limited, not unsupported"
    );
    assert!(
        report.reason.contains("memory") && report.reason.contains("disk"),
        "compat-boundary: reason must name both tight resources, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-boundary");
}

#[test]
fn compat_headroom_boundary_is_supported() {
    let mut facts = supported_facts();
    facts.available_memory_bytes = Some(5 * GIB);
    facts.available_disk_bytes = Some(10 * GIB);

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Supported,
        "compat-headroom: exactly 25% headroom must be Supported"
    );
}

#[test]
fn compat_one_byte_below_headroom_is_limited() {
    let mut facts = supported_facts();
    facts.available_memory_bytes = Some(5 * GIB - 1);
    facts.available_disk_bytes = Some(10 * GIB);

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::SupportedWithLimits,
        "compat-headroom: one byte below 25% headroom must be limited"
    );
    assert!(
        report.reason.contains("memory"),
        "compat-headroom: reason must mention memory, got {:?}",
        report.reason
    );
}

#[test]
fn compat_one_byte_below_memory_minimum_is_unsupported() {
    let mut facts = supported_facts();
    facts.available_memory_bytes = Some(4 * GIB - 1);

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-memory-boundary: one byte below the minimum must be Unsupported"
    );
    assert!(
        report.reason.contains("memory"),
        "compat-memory-boundary: reason must mention memory, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-memory-boundary");
}

#[test]
fn compat_one_byte_below_disk_minimum_is_unsupported() {
    let mut facts = supported_facts();
    facts.available_disk_bytes = Some(8 * GIB - 1);

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-disk-boundary: one byte below the minimum must be Unsupported"
    );
    assert!(
        report.reason.contains("disk"),
        "compat-disk-boundary: reason must mention disk, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-disk-boundary");
}

#[test]
fn compat_unavailable_memory_is_limited() {
    let mut facts = supported_facts();
    facts.available_memory_bytes = None;

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::SupportedWithLimits,
        "compat-memory-unavailable: unknown available memory must be limited"
    );
    assert!(
        report.reason.contains("memory"),
        "compat-memory-unavailable: reason must mention memory, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-memory-unavailable");
}

#[test]
fn compat_unavailable_disk_is_limited() {
    let mut facts = supported_facts();
    facts.available_disk_bytes = None;

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::SupportedWithLimits,
        "compat-disk-unavailable: unknown free disk space must be limited"
    );
    assert!(
        report.reason.contains("disk"),
        "compat-disk-unavailable: reason must mention disk, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-disk-unavailable");
}

#[test]
fn compat_unavailable_os_is_limited() {
    let mut facts = supported_facts();
    facts.os = None;

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::SupportedWithLimits,
        "compat-os-unavailable: an unknown OS must be limited"
    );
    assert!(
        report.reason.contains("operating system"),
        "compat-os-unavailable: reason must mention the OS, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-os-unavailable");
}

#[test]
fn compat_unavailable_architecture_is_limited() {
    let mut facts = supported_facts();
    facts.arch = None;

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::SupportedWithLimits,
        "compat-arch-unavailable: unknown architecture must be limited"
    );
    assert!(
        report.reason.contains("architecture"),
        "compat-arch-unavailable: reason must mention architecture, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-arch-unavailable");
}

#[test]
fn compat_unsupported_architecture() {
    let mut facts = supported_facts();
    facts.arch = Some("x86".to_string());

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-arch-unsupported: 32-bit x86 must be Unsupported"
    );
    assert!(
        report.reason.contains("x86"),
        "compat-arch-unsupported: reason must mention x86, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-arch-unsupported");
}

#[test]
fn compat_model_architecture_mismatch_is_unsupported() {
    let mut requirements = requirements();
    requirements.architectures = vec!["aarch64".to_string()];
    let facts = supported_facts();

    let report = evaluate(&requirements, &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-model-arch: an aarch64-only model on x86_64 must be Unsupported"
    );
    assert!(
        report.reason.contains("aarch64"),
        "compat-model-arch: reason must mention the model architecture, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-model-arch");
}

#[test]
fn compat_architecture_any_is_not_a_restriction() {
    let mut requirements = requirements();
    requirements.architectures = vec!["any".to_string(), "aarch64".to_string()];

    let report = evaluate(&requirements, &supported_facts());
    assert_eq!(
        report.status,
        CompatibilityStatus::Supported,
        "compat-arch-any: an 'any' architecture must override other list entries"
    );
}

#[test]
fn compat_missing_runtime_is_unsupported() {
    let mut facts = supported_facts();
    facts.runtime = None;

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-runtime-missing: no runtime must be Unsupported"
    );
    assert_actionable(&report, "compat-runtime-missing");
}

#[test]
fn compat_fake_runtime_is_unsupported() {
    let mut facts = supported_facts();
    facts.runtime = Some(RuntimeCapabilities {
        id: "fake".to_string(),
        available: false,
    });

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-runtime-fake: the Fake backend must not count as a real runtime"
    );
    assert!(
        report.reason.contains("test inference backend"),
        "compat-runtime-fake: reason must identify the Fake backend, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-runtime-fake");
}

#[test]
fn compat_runtime_mismatch_is_unsupported() {
    let mut requirements = requirements();
    requirements.runtime = Some("onnx".to_string());

    let report = evaluate(&requirements, &supported_facts());
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-runtime-mismatch: a model for another runtime must be Unsupported"
    );
    assert!(
        report.reason.contains("onnx"),
        "compat-runtime-mismatch: reason must mention the requested runtime, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-runtime-mismatch");
}

#[test]
fn compat_runtime_any_matches_the_available_runtime() {
    let mut requirements = requirements();
    requirements.runtime = Some("any".to_string());

    let report = evaluate(&requirements, &supported_facts());
    assert_eq!(
        report.status,
        CompatibilityStatus::Supported,
        "compat-runtime-any: 'any' must accept the available runtime"
    );
}

#[test]
fn compat_unsupported_os() {
    let mut facts = supported_facts();
    facts.os = Some("android".to_string());

    let report = evaluate(&requirements(), &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Unsupported,
        "compat-os: an unsupported OS must be Unsupported"
    );
    assert!(
        report.reason.contains("android"),
        "compat-os: reason must mention the OS, got {:?}",
        report.reason
    );
    assert_actionable(&report, "compat-os");
}

#[test]
fn compat_zero_requirements_ignore_unavailable_resources() {
    let requirements = ModelRequirements {
        min_memory_bytes: 0,
        min_disk_bytes: 0,
        architectures: Vec::new(),
        runtime: None,
    };
    let mut facts = supported_facts();
    facts.available_memory_bytes = None;
    facts.available_disk_bytes = None;

    let report = evaluate(&requirements, &facts);
    assert_eq!(
        report.status,
        CompatibilityStatus::Supported,
        "compat-zero: no memory or disk requirement must not be limited by unreadable values"
    );
}

#[test]
fn compat_report_serializes_camel_case() {
    let mut facts = supported_facts();
    facts.available_memory_bytes = Some(4 * GIB);
    facts.available_disk_bytes = Some(8 * GIB);

    let value = serde_json::to_value(evaluate(&requirements(), &facts))
        .expect("compat-json: report must serialize");
    assert_eq!(value["status"], "supportedWithLimits");
    assert_eq!(value["totalMemoryBytes"], 16 * GIB);
    assert_eq!(value["availableMemoryBytes"], 4 * GIB);
    assert_eq!(value["requiredDiskBytes"], 8 * GIB);
    assert!(value["recommendation"].is_string());
}

#[test]
fn compat_probe_reads_the_local_host_without_panicking() {
    let facts = probe(Some(&std::env::temp_dir()));
    assert!(facts.os.is_some(), "compat-probe: OS must be detected");
    assert!(
        facts.arch.is_some(),
        "compat-probe: architecture must be detected"
    );
    assert!(
        facts.runtime.is_some(),
        "compat-probe: runtime facts must be present"
    );
    assert!(
        facts.available_disk_bytes.is_some(),
        "compat-probe: the temp directory must have a readable volume"
    );
}
