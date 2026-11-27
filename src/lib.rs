//! SigmaOS rebuild — system rebuild orchestrator.
//!
//! Plans and executes ordered system rebuild steps (initramfs, kernel modules,
//! DKMS modules, bootloader, initramfs tools) with dry-run support and
//! per-step status tracking.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Errors produced by this component.
#[derive(Debug, thiserror::Error)]
pub enum RebuildError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("step failed: {0}")]
    StepFailed(String),
    #[error("validation error: {0}")]
    Validation(String),
}

/// A rebuild step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildStep {
    pub name: String,
    /// Command to run (executed via `sh -c` equivalent — caller supplies argv).
    pub command: Vec<String>,
    /// Whether failure aborts the whole rebuild.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Human description.
    #[serde(default)]
    pub description: String,
}

fn default_true() -> bool {
    true
}

/// Result of a single step execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

/// Execution record for a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub step: String,
    pub status: StepStatus,
    pub exit_code: Option<i32>,
    pub output_tail: String,
}

/// Full rebuild plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildPlan {
    pub kernel_release: String,
    pub steps: Vec<RebuildStep>,
}

impl RebuildPlan {
    /// Standard SigmaOS rebuild plan.
    pub fn standard(kernel_release: &str) -> Self {
        Self {
            kernel_release: kernel_release.to_string(),
            steps: vec![
                RebuildStep {
                    name: "dkms".into(),
                    command: vec!["dkms".into(), "autoinstall".into()],
                    required: true,
                    description: "Rebuild DKMS modules for kernel".into(),
                },
                RebuildStep {
                    name: "initramfs".into(),
                    command: vec!["sigma-initramfs".into(), "--all".into()],
                    required: true,
                    description: "Regenerate initramfs images".into(),
                },
                RebuildStep {
                    name: "bootloader".into(),
                    command: vec![
                        "grub-mkconfig".into(),
                        "-o".into(),
                        "/boot/grub/grub.cfg".into(),
                    ],
                    required: false,
                    description: "Update bootloader config".into(),
                },
            ],
        }
    }

    /// Validate plan.
    pub fn validate(&self) -> Result<(), RebuildError> {
        if self.steps.is_empty() {
            return Err(RebuildError::Validation("empty rebuild plan".into()));
        }
        for s in &self.steps {
            if s.name.is_empty() || s.command.is_empty() {
                return Err(RebuildError::Validation(
                    "step requires name and command".into(),
                ));
            }
        }
        Ok(())
    }

    /// Print plan as human-readable text.
    pub fn describe(&self) -> String {
        let mut out = format!("Rebuild plan for kernel {}:\n", self.kernel_release);
        for (i, s) in self.steps.iter().enumerate() {
            let req = if s.required { "required" } else { "optional" };
            out.push_str(&format!(
                "  {}. {} [{}] — {}\n",
                i + 1,
                s.name,
                req,
                if s.description.is_empty() {
                    s.command.join(" ")
                } else {
                    s.description.clone()
                }
            ));
        }
        out
    }

    /// Execute the plan. `dry_run` prints commands without executing.
    /// `runner` executes a command, returning (exit_code, output_tail).
    pub fn execute<F>(&self, dry_run: bool, mut runner: F) -> Result<Vec<StepRecord>, RebuildError>
    where
        F: FnMut(&[String]) -> Result<(i32, String), RebuildError>,
    {
        self.validate()?;
        let mut records = Vec::new();
        for step in &self.steps {
            let record = if dry_run {
                StepRecord {
                    step: step.name.clone(),
                    status: StepStatus::Skipped,
                    exit_code: None,
                    output_tail: format!("would run: {}", step.command.join(" ")),
                }
            } else {
                match runner(&step.command) {
                    Ok((code, tail)) if code == 0 => StepRecord {
                        step: step.name.clone(),
                        status: StepStatus::Success,
                        exit_code: Some(code),
                        output_tail: tail,
                    },
                    Ok((code, tail)) => {
                        let rec = StepRecord {
                            step: step.name.clone(),
                            status: StepStatus::Failed,
                            exit_code: Some(code),
                            output_tail: tail,
                        };
                        if step.required {
                            records.push(rec);
                            return Err(RebuildError::StepFailed(format!(
                                "{} exited {}",
                                step.name, code
                            )));
                        }
                        records.push(rec);
                        continue;
                    }
                    Err(e) => {
                        if step.required {
                            return Err(e);
                        }
                        records.push(StepRecord {
                            step: step.name.clone(),
                            status: StepStatus::Failed,
                            exit_code: None,
                            output_tail: e.to_string(),
                        });
                        continue;
                    }
                }
            };
            records.push(record);
        }
        Ok(records)
    }
}

/// Current kernel release.
pub fn current_kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Save records as JSON.
pub fn save_records(records: &[StepRecord], path: &Path) -> Result<(), RebuildError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(records)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_runner(_: &[String]) -> Result<(i32, String), RebuildError> {
        Ok((0, "done".into()))
    }

    #[test]
    fn standard_plan_validates_and_describes() {
        let p = RebuildPlan::standard("6.1.0-sigma");
        p.validate().unwrap();
        let d = p.describe();
        assert!(d.contains("6.1.0-sigma"));
        assert!(d.contains("dkms"));
        assert!(d.contains("required"));
        assert!(d.contains("optional")); // bootloader step
    }

    #[test]
    fn empty_and_blank_steps_rejected() {
        let mut p = RebuildPlan::standard("x");
        p.steps.clear();
        assert!(p.validate().is_err());
        p.steps.push(RebuildStep {
            name: String::new(),
            command: vec!["true".into()],
            required: true,
            description: String::new(),
        });
        assert!(p.validate().is_err());
        p.steps[0].name = "ok".into();
        p.steps[0].command.clear();
        assert!(p.validate().is_err());
    }

    #[test]
    fn dry_run_skips_all_steps() {
        let p = RebuildPlan::standard("x");
        let recs = p.execute(true, ok_runner).unwrap();
        assert_eq!(recs.len(), 3);
        assert!(recs.iter().all(|r| r.status == StepStatus::Skipped));
        assert!(recs[0].output_tail.contains("would run:"));
    }

    #[test]
    fn required_failure_aborts_plan() {
        let p = RebuildPlan::standard("x");
        let mut calls = 0;
        let r = p.execute(false, |cmd| {
            calls += 1;
            if cmd[0] == "dkms" {
                Ok((1, "boom".into()))
            } else {
                Ok((0, "ok".into()))
            }
        });
        assert!(matches!(r, Err(RebuildError::StepFailed(_))));
        assert_eq!(calls, 1); // aborted before initramfs step
    }

    #[test]
    fn optional_failure_continues() {
        let p = RebuildPlan::standard("x");
        let recs = p
            .execute(false, |cmd| {
                if cmd[0] == "grub-mkconfig" {
                    Ok((1, "no grub".into()))
                } else {
                    Ok((0, "ok".into()))
                }
            })
            .unwrap();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[2].status, StepStatus::Failed);
        assert_eq!(recs[0].status, StepStatus::Success);
    }

    #[test]
    fn runner_error_on_optional_continues() {
        let p = RebuildPlan::standard("x");
        let recs = p
            .execute(false, |cmd| {
                if cmd[0] == "grub-mkconfig" {
                    Err(RebuildError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "missing",
                    )))
                } else {
                    Ok((0, "ok".into()))
                }
            })
            .unwrap();
        assert_eq!(recs[2].status, StepStatus::Failed);
        assert_eq!(recs[2].exit_code, None);
    }

    #[test]
    fn save_records_roundtrip() {
        let recs = vec![StepRecord {
            step: "s".into(),
            status: StepStatus::Success,
            exit_code: Some(0),
            output_tail: "t".into(),
        }];
        let dir = std::env::temp_dir().join(format!("rebuild-{}", std::process::id()));
        let path = dir.join("nested/records.json");
        save_records(&recs, &path).unwrap();
        let back: Vec<StepRecord> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].step, "s");
        assert_eq!(back[0].status, StepStatus::Success);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serde_step_defaults_required_true() {
        // `required` defaults to true when absent from JSON.
        let j = r#"{"name":"x","command":["true"],"description":""}"#;
        let s: RebuildStep = serde_json::from_str(j).unwrap();
        assert!(s.required);
    }
}
