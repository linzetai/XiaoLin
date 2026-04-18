use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::feedback::FeedbackStore;

/// Summary report of an agent's strategy performance.
///
/// `total_interactions` is the true count from the database.
/// `sample_size` reflects the capped window used for metric computation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyReport {
    pub agent_id: String,
    pub total_interactions: i64,
    pub sample_size: i64,
    pub thumbs_up: i64,
    pub thumbs_down: i64,
    pub avg_rating: Option<f64>,
    pub tool_success_rate: Option<f64>,
    pub retry_rate: f64,
    pub satisfaction_score: f64,
    pub recommendations: Vec<String>,
}

pub struct StrategyEvaluator<'a> {
    store: &'a FeedbackStore,
}

impl<'a> StrategyEvaluator<'a> {
    pub fn new(store: &'a FeedbackStore) -> Self {
        Self { store }
    }

    /// Generate a strategy report for the given agent.
    pub async fn evaluate(&self, agent_id: &str) -> Result<StrategyReport> {
        const SAMPLE_CAP: i64 = 1000;
        let total_interactions = self.store.count(agent_id).await?;
        let recent = self.store.recent(agent_id, SAMPLE_CAP).await?;
        let total = recent.len() as i64;

        let mut thumbs_up: i64 = 0;
        let mut thumbs_down: i64 = 0;
        let mut tool_success: i64 = 0;
        let mut tool_failure: i64 = 0;
        let mut user_retry: i64 = 0;
        let mut rating_sum: f64 = 0.0;
        let mut rating_count: i64 = 0;

        for f in &recent {
            match f.kind.as_str() {
                "thumbs_up" => thumbs_up += 1,
                "thumbs_down" => thumbs_down += 1,
                "tool_success" => tool_success += 1,
                "tool_failure" => tool_failure += 1,
                "user_retry" => user_retry += 1,
                "rating" => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&f.payload) {
                        if let Some(val) = v["value"].as_f64() {
                            rating_sum += val;
                            rating_count += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        let avg_rating = if rating_count > 0 {
            Some(rating_sum / rating_count as f64)
        } else {
            None
        };

        let tool_total = tool_success + tool_failure;
        let tool_success_rate = if tool_total > 0 {
            Some(tool_success as f64 / tool_total as f64)
        } else {
            None
        };

        let retry_rate = if total > 0 {
            user_retry as f64 / total as f64
        } else {
            0.0
        };

        let satisfaction_score = compute_satisfaction(
            thumbs_up,
            thumbs_down,
            avg_rating,
            tool_success_rate,
            retry_rate,
        );

        let recommendations = generate_recommendations(
            &satisfaction_score,
            tool_success_rate,
            retry_rate,
            thumbs_down,
            total,
        );

        Ok(StrategyReport {
            agent_id: agent_id.to_string(),
            total_interactions,
            sample_size: total,
            thumbs_up,
            thumbs_down,
            avg_rating,
            tool_success_rate,
            retry_rate,
            satisfaction_score,
            recommendations,
        })
    }
}

fn compute_satisfaction(
    up: i64,
    down: i64,
    avg_rating: Option<f64>,
    tool_sr: Option<f64>,
    retry_rate: f64,
) -> f64 {
    let mut score = 0.5;

    let vote_total = up + down;
    if vote_total > 0 {
        let vote_ratio = up as f64 / vote_total as f64;
        score = score * 0.5 + vote_ratio * 0.5;
    }

    if let Some(r) = avg_rating {
        score = score * 0.6 + (r / 5.0) * 0.4;
    }

    if let Some(tsr) = tool_sr {
        score = score * 0.7 + tsr * 0.3;
    }

    score -= retry_rate * 0.2;

    score.clamp(0.0, 1.0)
}

fn generate_recommendations(
    score: &f64,
    tool_sr: Option<f64>,
    retry_rate: f64,
    thumbs_down: i64,
    total: i64,
) -> Vec<String> {
    let mut recs = Vec::new();

    if *score < 0.4 {
        recs.push(
            "Overall satisfaction is low. Consider reviewing and revising the system prompt."
                .into(),
        );
    }

    if let Some(tsr) = tool_sr {
        if tsr < 0.7 {
            recs.push(format!(
                "Tool success rate is {:.0}%. Review tool definitions and error handling.",
                tsr * 100.0
            ));
        }
    }

    if retry_rate > 0.15 {
        recs.push(format!(
            "Retry rate is {:.0}%. Users are rephrasing frequently — improve response clarity.",
            retry_rate * 100.0
        ));
    }

    if total > 10 && thumbs_down as f64 / total as f64 > 0.2 {
        recs.push(
            "High negative feedback ratio. Analyze thumbs-down sessions for patterns.".into(),
        );
    }

    if recs.is_empty() {
        recs.push("Performance looks healthy. Continue monitoring.".into());
    }

    recs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::{FeedbackKind, InteractionSignal};
    use sqlx::SqlitePool;

    async fn setup() -> FeedbackStore {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        FeedbackStore::open(pool).await.unwrap()
    }

    #[tokio::test]
    async fn evaluate_empty_agent() {
        let store = setup().await;
        let eval = StrategyEvaluator::new(&store);
        let report = eval.evaluate("main").await.unwrap();
        assert_eq!(report.total_interactions, 0);
        assert_eq!(report.satisfaction_score, 0.5);
    }

    #[tokio::test]
    async fn evaluate_with_feedback() {
        let store = setup().await;

        for _ in 0..8 {
            store
                .record_feedback("s1", "main", None, &FeedbackKind::ThumbsUp)
                .await
                .unwrap();
        }
        for _ in 0..2 {
            store
                .record_feedback("s1", "main", None, &FeedbackKind::ThumbsDown)
                .await
                .unwrap();
        }
        for _ in 0..5 {
            store
                .record_signal(
                    "s1",
                    "main",
                    &InteractionSignal::ToolSuccess {
                        tool_name: "calc".into(),
                    },
                )
                .await
                .unwrap();
        }
        store
            .record_signal(
                "s1",
                "main",
                &InteractionSignal::ToolFailure {
                    tool_name: "http".into(),
                    error: "timeout".into(),
                },
            )
            .await
            .unwrap();

        let eval = StrategyEvaluator::new(&store);
        let report = eval.evaluate("main").await.unwrap();

        assert!(report.satisfaction_score > 0.5);
        assert!(report.tool_success_rate.unwrap() > 0.8);
        assert_eq!(report.thumbs_up, 8);
        assert_eq!(report.thumbs_down, 2);
    }

    #[tokio::test]
    async fn evaluate_tool_success_rate_matches_store_signals() {
        let store = setup().await;
        let n_success: i64 = 7;
        let n_failure: i64 = 3;
        for _ in 0..n_success {
            store
                .record_signal(
                    "s1",
                    "main",
                    &InteractionSignal::ToolSuccess {
                        tool_name: "t".into(),
                    },
                )
                .await
                .unwrap();
        }
        for _ in 0..n_failure {
            store
                .record_signal(
                    "s1",
                    "main",
                    &InteractionSignal::ToolFailure {
                        tool_name: "t".into(),
                        error: "e".into(),
                    },
                )
                .await
                .unwrap();
        }

        let eval = StrategyEvaluator::new(&store);
        let report = eval.evaluate("main").await.unwrap();
        let expected = n_success as f64 / (n_success + n_failure) as f64;
        assert!((report.tool_success_rate.unwrap() - expected).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn evaluate_rating_average_matches_payload_values() {
        let store = setup().await;
        store
            .record_feedback("s1", "main", None, &FeedbackKind::Rating(4.0))
            .await
            .unwrap();
        store
            .record_feedback("s1", "main", None, &FeedbackKind::Rating(2.0))
            .await
            .unwrap();

        let eval = StrategyEvaluator::new(&store);
        let report = eval.evaluate("main").await.unwrap();
        assert!((report.avg_rating.unwrap() - 3.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn evaluate_retry_rate_matches_user_retry_fraction() {
        let store = setup().await;
        let n_retry: i64 = 3;
        let n_other: i64 = 7;
        for _ in 0..n_retry {
            store
                .record_signal("s1", "main", &InteractionSignal::UserRetry)
                .await
                .unwrap();
        }
        for _ in 0..n_other {
            store
                .record_feedback("s1", "main", None, &FeedbackKind::ThumbsUp)
                .await
                .unwrap();
        }

        let eval = StrategyEvaluator::new(&store);
        let report = eval.evaluate("main").await.unwrap();
        let expected = n_retry as f64 / (n_retry + n_other) as f64;
        assert!((report.retry_rate - expected).abs() < f64::EPSILON);
    }
}
