//! Deserialization types for the `/metrics` endpoint response.

use serde::Deserialize;
use serde_json::Value;

use crate::db::task::TaskInsert;
use crate::db::scrape::ScrapeInsert;

/// Deserialize `available`/`wait`/`error` which may be `Vec<String>` or
/// `Vec<{model, status}>` depending on the llama-server version.
fn deserialize_string_or_object_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Array(arr) => arr
            .into_iter()
            .map(|v| {
                if let Some(s) = v.as_str() {
                    Ok(s.to_string())
                } else if let Some(obj) = v.as_object() {
                    Ok(obj.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string())
                } else {
                    Ok(String::new())
                }
            })
            .collect(),
        _ => Ok(Vec::new()),
    }
}

/// Top-level response from `/metrics?model=<name>`.
#[derive(Debug, Deserialize)]
pub struct MetricsResponse {
    pub active: Vec<ActiveModel>,
    #[serde(default, deserialize_with = "deserialize_string_or_object_list")]
    pub available: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_object_list")]
    pub wait: Vec<String>,
    #[serde(default)]
    pub error: Vec<String>,
}

/// A model entry in the `active[]` array with its metrics.
#[derive(Debug, Deserialize)]
pub struct ActiveModel {
    pub model: String,
    pub status: String,
    pub metrics: FullMetrics,
}

/// Complete metrics object from the server.
#[derive(Debug, Deserialize)]
pub struct FullMetrics {
    pub context_max: i64,
    pub tasks: TaskCounts,
    pub prompt: PromptMetrics,
    pub prediction: PredictionMetrics,
    pub decode: DecodeMetrics,
    pub kvcache: KvCacheMetrics,
    #[serde(default)]
    pub slots: Vec<SlotMetrics>,
    pub memory: MemoryMetrics,
    pub recent: Vec<TaskRecord>,
}

/// Aggregate task counts.
#[derive(Debug, Deserialize)]
pub struct TaskCounts {
    pub processing: i64,
    pub queued: i64,
    pub completed: i64,
    pub duration_seconds: f64,
}

/// Prompt processing metrics.
#[derive(Debug, Deserialize)]
pub struct PromptMetrics {
    pub tokens_total: i64,
    pub tokens_cached_total: i64,
    pub tokens_processed: i64,
}

/// Token generation metrics.
#[derive(Debug, Deserialize)]
pub struct PredictionMetrics {
    pub tokens_total: i64,
    pub tokens_processed: i64,
}

/// Accepted tokens from speculative decoding.
#[derive(Debug, Default, Deserialize)]
pub struct AcceptedTokens {
    pub total: i64,
    #[serde(default, alias = "draft_pos_0")]
    pub draft_pos_0: i64,
    #[serde(default, alias = "draft_pos_1")]
    pub draft_pos_1: i64,
    #[serde(default, alias = "draft_pos_2")]
    pub draft_pos_2: i64,
    #[serde(default, alias = "draft_pos_3")]
    pub draft_pos_3: i64,
    #[serde(default, alias = "draft_pos_4")]
    pub draft_pos_4: i64,
}

/// Speculative decoding statistics.
#[derive(Debug, Default, Deserialize)]
pub struct SpeculativeMetrics {
    #[serde(default)]
    pub draft_tokens_total: i64,
    #[serde(default)]
    pub draft_steps_total: i64,
    #[serde(default)]
    pub accepted_tokens: AcceptedTokens,
}

/// Decode statistics.
#[derive(Debug, Default, Deserialize)]
pub struct DecodeMetrics {
    pub total: i64,
    pub n_tokens_max: i64,
    pub n_busy_slots: i64,
    #[serde(default)]
    pub speculative: Option<SpeculativeMetrics>,
}

/// KV cache state.
#[derive(Debug, Deserialize)]
pub struct KvCacheMetrics {
    pub active_tokens: i64,
}

/// Slot state info.
#[derive(Debug, Default, Deserialize)]
pub struct SlotMetrics {
    #[serde(default)]
    pub id: i64,
}

/// Memory usage.
#[derive(Debug, Deserialize)]
pub struct MemoryMetrics {
    pub context_bytes: i64,
    pub model_bytes: i64,
}

/// A single completed task from `metrics.recent[]`.
///
/// Field names vary across llama-server versions; aliases cover the common forms.
/// The live server uses `draft_accepted` (not `n_draft_accepted`).
#[derive(Debug, Default, Deserialize)]
pub struct TaskRecord {
    pub task_id: i64,
    pub slot: i64,
    #[serde(alias = "prompt_cached")]
    pub n_prompt_cached: i64,
    #[serde(alias = "prompt_new")]
    pub n_prompt_processed: i64,
    #[serde(alias = "generated")]
    pub n_gen: i64,
    #[serde(alias = "draft_tokens")]
    pub n_draft_tokens: i64,
    #[serde(alias = "n_draft_accepted", alias = "draft_accepted", default)]
    pub n_draft_accepted: i64,
    pub prompt_ms: f64,
    pub gen_ms: f64,
    pub prompt_tps: f64,
    pub gen_tps: f64,
    pub prompt_per_token_ms: f64,
    pub gen_per_token_ms: f64,
}

/// Convert a `TaskRecord` to a database insert struct.
impl From<&TaskRecord> for TaskInsert {
    fn from(record: &TaskRecord) -> Self {
        Self {
            task_id: record.task_id,
            model_load_id: 0, // set by caller
            slot_id: record.slot,
            n_prompt_cached: record.n_prompt_cached,
            n_prompt_processed: record.n_prompt_processed,
            prompt_ms: record.prompt_ms,
            prompt_tps: record.prompt_tps,
            prompt_per_token_ms: record.prompt_per_token_ms,
            n_gen: record.n_gen,
            gen_ms: record.gen_ms,
            gen_tps: record.gen_tps,
            gen_per_token_ms: record.gen_per_token_ms,
            n_draft_tokens: record.n_draft_tokens,
            n_draft_accepted: record.n_draft_accepted,
            captured_at: 0, // set by caller
        }
    }
}

/// Sanitize a float value: clamp NaN and negative values to 0.0.
fn sanitize_f64(v: f64) -> f64 {
    if v.is_nan() || v < 0.0 {
        tracing::warn!(value = v, "Invalid metric value — clamping to 0.0");
        0.0
    } else {
        v
    }
}

/// Sanitize an integer value: clamp negative values to 0.
fn sanitize_i64(v: i64) -> i64 {
    if v < 0 {
        tracing::warn!(value = v, "Invalid metric value — clamping to 0");
        0
    } else {
        v
    }
}

/// Extract aggregate metrics into a database scrape insert struct.
impl From<(&FullMetrics, i64)> for ScrapeInsert {
    fn from((metrics, captured_at): (&FullMetrics, i64)) -> Self {
        Self {
            model_name: String::new(), // set by caller
            model_created: 0,           // set by caller
            model_load_id: 0,           // set by caller
            scraped_at: captured_at,
            context_max: Some(metrics.context_max),
            tasks_processing: sanitize_i64(metrics.tasks.processing),
            tasks_queued: sanitize_i64(metrics.tasks.queued),
            tasks_completed: sanitize_i64(metrics.tasks.completed),
            tasks_duration_s: sanitize_f64(metrics.tasks.duration_seconds),
            prompt_tokens_total: Some(metrics.prompt.tokens_total),
            prompt_tokens_cached: Some(metrics.prompt.tokens_cached_total),
            prompt_tokens_processed: Some(metrics.prompt.tokens_processed),
            prediction_tokens_total: Some(metrics.prediction.tokens_total),
            prediction_tokens_processed: Some(metrics.prediction.tokens_processed),
            decode_total: Some(metrics.decode.total),
            kvcache_active_tokens: Some(metrics.kvcache.active_tokens),
            memory_context_bytes: Some(metrics.memory.context_bytes),
            memory_model_bytes: Some(metrics.memory.model_bytes),
            task_ids_seen: String::new(), // set by caller
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metrics_sample() {
        let json = r#"{
            "active": [
                {
                    "model": "example-model",
                    "status": "loaded",
                    "metrics": {
                        "context_max": 16384,
                        "tasks": {
                            "processing": 0,
                            "queued": 0,
                            "completed": 1,
                            "duration_seconds": 3.1
                        },
                        "prompt": {
                            "tokens_total": 44,
                            "tokens_cached_total": 0,
                            "tokens_processed": 44
                        },
                        "prediction": {
                            "tokens_total": 10,
                            "tokens_processed": 10
                        },
                        "decode": {
                            "total": 10,
                            "n_tokens_max": 53,
                            "n_busy_slots": 10
                        },
                        "kvcache": {
                            "active_tokens": 0
                        },
                        "slots": [],
                        "memory": {
                            "context_bytes": 1879048192,
                            "model_bytes": 2959223072
                        },
                        "recent": [
                            {
                                "task_id": 12,
                                "slot": 1,
                                "n_prompt_cached": 0,
                                "n_prompt_processed": 44,
                                "n_gen": 10,
                                "n_draft_tokens": 0,
                                "n_draft_accepted": 0,
                                "prompt_ms": 1490.8,
                                "gen_ms": 1609.5,
                                "prompt_tps": 29.5,
                                "gen_tps": 5.6,
                                "prompt_per_token_ms": 33.9,
                                "gen_per_token_ms": 178.8
                            }
                        ]
                    }
                }
            ],
            "available": [],
            "wait": [],
            "error": []
        }"#;

        let resp: MetricsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.active.len(), 1);
        assert_eq!(resp.active[0].model, "example-model");
        assert_eq!(resp.active[0].metrics.context_max, 16384);
        assert_eq!(resp.active[0].metrics.tasks.completed, 1);
        assert_eq!(resp.active[0].metrics.recent.len(), 1);
        assert_eq!(resp.active[0].metrics.recent[0].task_id, 12);
        assert_eq!(resp.active[0].metrics.recent[0].n_prompt_processed, 44);
        assert_eq!(resp.active[0].metrics.recent[0].n_gen, 10);
        assert!((resp.active[0].metrics.recent[0].prompt_tps - 29.5).abs() < 0.01);
    }

    #[test]
    fn test_parse_aggregated_response_with_populated_slots_and_draft_accepted() {
        // Real response shape from llama-server when slots are active
        // and field names use the newer convention (prompt_cached, draft_accepted)
        let json = r#"{
            "active": [{
                "model": "test-model",
                "status": "loaded",
                "metrics": {
                    "context_max": 131072,
                    "tasks": {"processing": 1, "queued": 0, "completed": 337, "duration_seconds": 25277.3},
                    "prompt": {"tokens_total": 370487, "tokens_cached_total": 22575409, "tokens_processed": 371511},
                    "prediction": {"tokens_total": 88927, "tokens_processed": 88927},
                    "decode": {"total": 29722, "n_tokens_max": 115352, "n_busy_slots": 29742,
                        "speculative": {"draft_tokens_total": 63464, "draft_steps_total": 23840,
                            "accepted_tokens": {"total": 60926, "draft_pos_0": 23154, "draft_pos_1": 19948, "draft_pos_2": 17824}}},
                    "kvcache": {"active_tokens": 0},
                    "slots": [
                        {"id": 1, "prompt": 90, "cache": 50731, "n_past": 50821, "n_beg": 90,
                         "cached_tokens": 90, "prompt_eval_time": 27000, "tokens_count": 238,
                         "tokens_evicted": 0}
                    ],
                    "memory": {"context_bytes": 1952972800, "model_bytes": 27076821536},
                    "recent": [
                        {"task_id": 188892, "slot": 1, "prompt_cached": 51582, "prompt_new": 90,
                         "generated": 160, "draft_tokens": 100, "draft_accepted": 95,
                         "prompt_ms": 5352.2, "gen_ms": 20855.3,
                         "prompt_per_token_ms": 59.5, "gen_per_token_ms": 131.2,
                         "prompt_tps": 16.8, "gen_tps": 7.6}
                    ]
                }
            }],
            "available": [{"model": "other", "status": "unloaded"}],
            "wait": [],
            "error": []
        }"#;

        let resp: MetricsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.active.len(), 1);
        let m = &resp.active[0].metrics;
        assert_eq!(m.context_max, 131072);
        assert_eq!(m.tasks.completed, 337);
        assert_eq!(m.decode.speculative.as_ref().unwrap().draft_tokens_total, 63464);
        // Slots with id field
        assert_eq!(m.slots.len(), 1);
        assert_eq!(m.slots[0].id, 1);
        // Recent tasks using draft_accepted (not n_draft_accepted)
        assert_eq!(m.recent.len(), 1);
        assert_eq!(m.recent[0].n_gen, 160);
        assert_eq!(m.recent[0].n_draft_accepted, 95);
        assert_eq!(m.recent[0].n_prompt_cached, 51582);
        assert_eq!(m.recent[0].n_prompt_processed, 90);
        // ScrapeInsert conversion
        let scrape: ScrapeInsert = (m, 1000).into();
        assert_eq!(scrape.tasks_completed, 337);
        assert_eq!(scrape.prompt_tokens_cached, Some(22575409));
        assert_eq!(scrape.memory_model_bytes, Some(27076821536));
    }

    #[test]
    fn test_parse_empty_active() {
        let json = r#"{
            "active": [],
            "available": [],
            "wait": [],
            "error": []
        }"#;
        let resp: MetricsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.active.is_empty());
    }

    #[test]
    fn test_task_record_to_insert() {
        let record = TaskRecord {
            task_id: 42, slot: 2,
            n_prompt_cached: 10, n_prompt_processed: 100,
            n_gen: 50, n_draft_tokens: 20, n_draft_accepted: 15,
            prompt_ms: 500.0, gen_ms: 1000.0,
            prompt_tps: 200.0, gen_tps: 50.0,
            prompt_per_token_ms: 5.0, gen_per_token_ms: 20.0,
        };
        let insert: TaskInsert = (&record).into();
        assert_eq!(insert.task_id, 42);
        assert_eq!(insert.slot_id, 2);
        assert_eq!(insert.n_prompt_processed, 100);
        assert_eq!(insert.n_gen, 50);
    }

    #[test]
    fn test_scrape_insert_from_metrics() {
        let json = r#"{
            "active": [{
                "model": "test",
                "status": "loaded",
                "metrics": {
                    "context_max": 8192,
                    "tasks": {"processing": 1, "queued": 0, "completed": 5, "duration_seconds": 10.0},
                    "prompt": {"tokens_total": 500, "tokens_cached_total": 100, "tokens_processed": 400},
                    "prediction": {"tokens_total": 200, "tokens_processed": 200},
                    "decode": {"total": 180, "n_tokens_max": 42, "n_busy_slots": 2},
                    "kvcache": {"active_tokens": 400},
                    "slots": [],
                    "memory": {"context_bytes": 1000000, "model_bytes": 2000000},
                    "recent": []
                }
            }],
            "available": [], "wait": [], "error": []
        }"#;
        let resp: MetricsResponse = serde_json::from_str(json).unwrap();
        let metrics = &resp.active[0].metrics;
        let scrape: ScrapeInsert = (metrics, 1000).into();
        assert_eq!(scrape.scraped_at, 1000);
        assert_eq!(scrape.context_max, Some(8192));
        assert_eq!(scrape.tasks_completed, 5);
        assert_eq!(scrape.memory_model_bytes, Some(2000000));
    }
}
