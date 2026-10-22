//! On-device hardware and resource compatibility for optional AI models.
//!
//! [`evaluate`] is pure so tests can inject boundary values and unavailable
//! system information. [`probe`] is the only host reader. Requirements are
//! supplied by the caller; no model is hard-coded here.
//!
//! A probe failure is reported as "supported with limits" or "unsupported";
//! it never becomes an error that could affect the PDF tools.

use std::path::Path;

use serde::{Deserialize, Serialize};

const SUPPORTED_OS: &[&str] = &["windows", "macos", "darwin", "linux"];
const SUPPORTED_ARCH: &[&str] = &["x86_64", "aarch64"];

/// A model is "supported" only when the minimum requirement plus this much
/// headroom is available. Exactly meeting the minimum is "supported with
/// limits".
const HEADROOM_DIVISOR: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompatibilityStatus {
    Supported,
    SupportedWithLimits,
    Unsupported,
}

/// Requirements passed in by the caller for one model. They are deliberately
/// independent of any model catalog or bundled manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequirements {
    pub min_memory_bytes: u64,
    pub min_disk_bytes: u64,
    /// Empty or `["any"]` means the model is architecture-independent.
    #[serde(default)]
    pub architectures: Vec<String>,
    /// Empty, `None`, or `"any"` means the model accepts any available runtime.
    #[serde(default)]
    pub runtime: Option<String>,
}

/// Capabilities of the inference layer in this build. A real runtime will
/// replace the Fake values in [`probe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilities {
    pub id: String,
    pub available: bool,
}

/// Host facts. Every field is optional so an unavailable value can be tested
/// and reported without failing the whole check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFacts {
    pub os: Option<String>,
    pub arch: Option<String>,
    pub runtime: Option<RuntimeCapabilities>,
    pub total_memory_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
    pub available_disk_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReport {
    pub status: CompatibilityStatus,
    pub reason: String,
    pub recommendation: String,
    pub operating_system: Option<String>,
    pub architecture: Option<String>,
    pub runtime_id: Option<String>,
    pub runtime_available: bool,
    pub required_memory_bytes: u64,
    pub total_memory_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
    pub required_disk_bytes: u64,
    pub available_disk_bytes: Option<u64>,
}

/// Read the local host. No network, telemetry, child process, or model file
/// access. `store_root` is only used to find the volume that will hold the
/// model; if it is unavailable, the disk fact is `None`.
pub fn probe(store_root: Option<&Path>) -> SystemFacts {
    let (total_memory, available_memory) = memory_bytes();
    SystemFacts {
        os: non_empty(std::env::consts::OS),
        arch: non_empty(std::env::consts::ARCH),
        runtime: Some(RuntimeCapabilities {
            id: "fake".to_string(),
            available: false,
        }),
        total_memory_bytes: nonzero(total_memory),
        available_memory_bytes: nonzero(available_memory),
        available_disk_bytes: store_root.and_then(available_disk),
    }
}

/// Compare caller-supplied requirements with host facts. This function cannot
/// fail: unavailable information becomes a cautious result instead.
pub fn evaluate(requirements: &ModelRequirements, facts: &SystemFacts) -> CompatibilityReport {
    let mut blockers: Vec<String> = Vec::new();
    let mut limits: Vec<String> = Vec::new();
    let mut recommendations: Vec<String> = Vec::new();
    let current_arch = facts.arch.as_deref().map(normalize_arch);
    let os = facts
        .os
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match os {
        None => limits.push("The operating system could not be detected.".to_string()),
        Some(os) if !is_supported_os(os) => {
            blockers.push(format!("Local AI is not supported on {os}."));
            recommendations
                .push("Use a supported 64-bit Windows, macOS, or Linux device.".to_string());
        }
        Some(_) => {}
    }

    match current_arch.as_deref() {
        None => limits.push("The processor architecture could not be detected.".to_string()),
        Some(arch)
            if !SUPPORTED_ARCH
                .iter()
                .any(|supported| supported.eq_ignore_ascii_case(arch)) =>
        {
            blockers.push(format!("Local AI is not supported on {arch} processors."));
            recommendations.push("Use a supported 64-bit x86_64 or aarch64 device.".to_string());
        }
        Some(arch) => {
            let requested = requested_architectures(&requirements.architectures);
            if !requested.is_empty()
                && !requested
                    .iter()
                    .any(|requested| requested.eq_ignore_ascii_case(arch))
            {
                blockers.push(format!(
                    "This model is built for {} and does not support {arch}.",
                    requested.join(", ")
                ));
                recommendations
                    .push("Choose a model built for this device's architecture.".to_string());
            }
        }
    }

    match facts.runtime.as_ref() {
        None => {
            blockers.push("No local inference runtime is available in this build.".to_string());
            recommendations.push(
                "Update OffPDF when local AI support is available, or keep using the PDF tools without a model."
                    .to_string(),
            );
        }
        Some(runtime) if !runtime.available => {
            blockers.push(if runtime.id.eq_ignore_ascii_case("fake") {
                "This build only includes the test inference backend, not a real local model runtime."
                    .to_string()
            } else {
                "The local inference runtime is not available.".to_string()
            });
            recommendations.push(
                "Update OffPDF when local AI support is available, or keep using the PDF tools without a model."
                    .to_string(),
            );
        }
        Some(runtime) if !runtime_matches(requirements.runtime.as_deref(), runtime) => {
            let requested = requirements
                .runtime
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("a different runtime");
            blockers.push(format!(
                "This model requires {requested}, but this build provides {}.",
                runtime.id
            ));
            recommendations.push("Choose a model that matches the available runtime.".to_string());
        }
        Some(_) => {}
    }

    if requirements.min_memory_bytes > 0 {
        match facts.available_memory_bytes {
            None => {
                limits.push("Available memory could not be read.".to_string());
                recommendations
                    .push("Check available memory before installing the model.".to_string());
            }
            Some(available) if available < requirements.min_memory_bytes => {
                blockers.push(format!(
                    "This model needs at least {} of available memory; this device does not have enough.",
                    human_bytes(requirements.min_memory_bytes)
                ));
                recommendations.push("Close other apps or choose a smaller model.".to_string());
            }
            Some(available) if !has_headroom(available, requirements.min_memory_bytes) => {
                limits.push(format!(
                    "Available memory is close to the {} minimum.",
                    human_bytes(requirements.min_memory_bytes)
                ));
                recommendations.push("Close other apps before loading the model.".to_string());
            }
            Some(_) => {}
        }
    }

    if requirements.min_disk_bytes > 0 {
        match facts.available_disk_bytes {
            None => {
                limits.push("Free disk space could not be read.".to_string());
                recommendations
                    .push("Confirm there is enough free disk space before installing.".to_string());
            }
            Some(available) if available < requirements.min_disk_bytes => {
                blockers.push(format!(
                    "This model needs at least {} of free disk space; this device does not have enough.",
                    human_bytes(requirements.min_disk_bytes)
                ));
                recommendations.push("Free up disk space or choose a smaller model.".to_string());
            }
            Some(available) if !has_headroom(available, requirements.min_disk_bytes) => {
                limits.push(format!(
                    "Free disk space is close to the {} minimum.",
                    human_bytes(requirements.min_disk_bytes)
                ));
                recommendations.push("Free up disk space before installing the model.".to_string());
            }
            Some(_) => {}
        }
    }

    recommendations.sort();
    recommendations.dedup();

    let status = if !blockers.is_empty() {
        CompatibilityStatus::Unsupported
    } else if !limits.is_empty() {
        CompatibilityStatus::SupportedWithLimits
    } else {
        CompatibilityStatus::Supported
    };

    let reason = if !blockers.is_empty() {
        blockers.join(" ")
    } else if !limits.is_empty() {
        limits.join(" ")
    } else {
        "This device meets the model's architecture, runtime, memory, and disk requirements."
            .to_string()
    };

    let recommendation = if !recommendations.is_empty() {
        recommendations.join(" ")
    } else {
        match status {
            CompatibilityStatus::Supported => "You can import or download this model.".to_string(),
            CompatibilityStatus::SupportedWithLimits => {
                "Close other apps and confirm enough free disk space before loading the model."
                    .to_string()
            }
            CompatibilityStatus::Unsupported => {
                "Use a supported device or choose a smaller model.".to_string()
            }
        }
    };

    CompatibilityReport {
        status,
        reason,
        recommendation,
        operating_system: facts.os.clone(),
        architecture: current_arch,
        runtime_id: facts.runtime.as_ref().map(|runtime| runtime.id.clone()),
        runtime_available: facts
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.available),
        required_memory_bytes: requirements.min_memory_bytes,
        total_memory_bytes: facts.total_memory_bytes,
        available_memory_bytes: facts.available_memory_bytes,
        required_disk_bytes: requirements.min_disk_bytes,
        available_disk_bytes: facts.available_disk_bytes,
    }
}

fn memory_bytes() -> (u64, u64) {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    (system.total_memory(), system.available_memory())
}

fn available_disk(path: &Path) -> Option<u64> {
    let mut probe = path;
    loop {
        if probe.exists() {
            return fs2::available_space(probe).ok();
        }
        probe = probe.parent()?;
    }
}

fn nonzero(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn is_supported_os(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    SUPPORTED_OS.iter().any(|supported| *supported == value)
}

fn normalize_arch(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x64" => "x86_64".to_string(),
        "arm64" => "aarch64".to_string(),
        other => other.to_string(),
    }
}

fn requested_architectures(values: &[String]) -> Vec<String> {
    if values
        .iter()
        .any(|value| value.trim().eq_ignore_ascii_case("any"))
    {
        return Vec::new();
    }

    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(normalize_arch)
        .collect()
}

fn runtime_matches(required: Option<&str>, runtime: &RuntimeCapabilities) -> bool {
    match required.map(str::trim).filter(|value| !value.is_empty()) {
        None => true,
        Some(value) if value.eq_ignore_ascii_case("any") => true,
        Some(value) => value.eq_ignore_ascii_case(&runtime.id),
    }
}

fn has_headroom(available: u64, required: u64) -> bool {
    if required == 0 {
        return true;
    }
    available.saturating_sub(required) >= required / HEADROOM_DIVISOR
}

fn human_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * MIB;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.0} MiB", value / MIB)
    } else {
        format!("{bytes} bytes")
    }
}
