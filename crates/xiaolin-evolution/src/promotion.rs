//! Skill promotion pipeline — ties together trajectory analysis, gap detection,
//! skill extraction, and lifecycle management (candidate → active → retired).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::skill_extractor::{SkillExtractor, SkillStatus};
use crate::skill_gap::{detect_gaps, GapReport};
use crate::skill_store::{MaintenanceReport, SkillStore};
use crate::trajectory::TrajectoryStore;

/// Configuration for the promotion pipeline.
#[derive(Debug, Clone)]
pub struct PromotionConfig {
    /// Min usage before candidate→active promotion.
    pub promote_min_usage: u32,
    /// Min success rate for promotion (0.0..1.0).
    pub promote_min_success_rate: f64,
    /// Min usage before considering retirement.
    pub retire_min_usage: u32,
    /// Max success rate for retirement (active→retired).
    pub retire_max_success_rate: f64,
    /// Min trajectories to trigger skill extraction.
    pub extraction_min_trajectories: u32,
    /// Similarity threshold for clustering.
    pub extraction_similarity_threshold: f64,
}

impl Default for PromotionConfig {
    fn default() -> Self {
        Self {
            promote_min_usage: 3,
            promote_min_success_rate: 0.7,
            retire_min_usage: 10,
            retire_max_success_rate: 0.3,
            extraction_min_trajectories: 3,
            extraction_similarity_threshold: 0.6,
        }
    }
}

/// Result of a full promotion pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// Maintenance report (promotions + retirements).
    pub maintenance: MaintenanceReport,
    /// New skills extracted from recent trajectories.
    pub new_skills_extracted: usize,
    /// Gap analysis report.
    pub gap_report: GapReport,
    /// Total trajectories analyzed.
    pub trajectories_analyzed: usize,
}

/// Run the full skill promotion pipeline.
///
/// Steps:
/// 1. Load recent trajectories from the trajectory store.
/// 2. Extract new candidate skills from successful trajectory clusters.
/// 3. Save new candidates to the skill store.
/// 4. Run maintenance (promote + retire).
/// 5. Detect skill gaps from failure patterns.
pub async fn run_pipeline(
    agent_id: &str,
    trajectory_store: &TrajectoryStore,
    skill_store: &SkillStore,
    config: &PromotionConfig,
) -> Result<PipelineResult> {
    // Step 1: Load recent trajectories
    let trajectories = trajectory_store
        .list_by_agent(agent_id, 200)
        .await?;
    let trajectories_analyzed = trajectories.len();

    // Step 2: Extract new candidate skills
    let extractor = SkillExtractor {
        min_occurrences: config.extraction_min_trajectories,
        similarity_threshold: config.extraction_similarity_threshold,
    };
    let new_skills = extractor.extract_skills(&trajectories);
    let new_skills_extracted = new_skills.len();

    // Step 3: Save new candidates
    for mut skill in new_skills {
        skill.status = SkillStatus::Candidate;
        if let Err(e) = skill_store.save_skill(&skill).await {
            tracing::warn!(
                skill_id = %skill.id,
                error = %e,
                "failed to save extracted skill"
            );
        }
    }

    // Step 4: Maintenance — promote + retire
    let maintenance = skill_store.maintenance().await?;
    if maintenance.promoted > 0 || maintenance.retired_active > 0 {
        tracing::info!(
            promoted = maintenance.promoted,
            retired = maintenance.retired_active,
            "skill maintenance completed"
        );
    }

    // Step 5: Gap detection
    let gap_report = detect_gaps(agent_id, &trajectories);
    if !gap_report.gaps.is_empty() {
        tracing::info!(
            gaps = gap_report.gaps.len(),
            failures = gap_report.failure_count,
            "skill gaps detected"
        );
    }

    Ok(PipelineResult {
        maintenance,
        new_skills_extracted,
        gap_report,
        trajectories_analyzed,
    })
}

/// Lightweight version — just runs maintenance (promote/retire)
/// without trajectory analysis. Suitable for periodic background tasks.
pub async fn run_maintenance_only(skill_store: &SkillStore) -> Result<MaintenanceReport> {
    skill_store.maintenance().await
}
