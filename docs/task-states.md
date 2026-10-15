# Slot State Machine

## States

Defined in `tools/server/server-context.cpp` (enum `slot_state`):

| Value | Name                  | Meaning                                                                                              |
|-------|-----------------------|------------------------------------------------------------------------------------------------------|
| 0     | `SLOT_STATE_IDLE`     | Slot is free. Not processing any task.                                                               |
| 1     | `SLOT_STATE_WAIT_OTHER` | Assigned to a child slot. Waiting for the parent slot to finish processing the shared prompt.        |
| 2     | `SLOT_STATE_STARTED`  | Task assigned to a parent slot. About to begin prompt processing.                                    |
| 3     | `SLOT_STATE_PROCESSING_PROMPT` | Currently processing prompt tokens (may span multiple `build_batch` iterations).          |
| 4     | `SLOT_STATE_DONE_PROMPT` | Prompt fully processed. Logits computed for the last token. Ready to sample the first generated token. |
| 5     | `SLOT_STATE_GENERATING` | Sampling tokens one by one.                                                                          |

### String representation

`slot_state_name()` in `tools/server/server-task.cpp` maps these to strings:

```
0 → idle
1 → wait_other
2 → started
3 → processing_prompt
4 → done_prompt
5 → generating
```

## Transitions

```
[IDLE]
  │
  │ task assigned (slot_select())
  ├──────────────┐
  ▼              │
[STARTED]        │ (parent task)
  │              │
  │ first decode step
  ▼              │
[PROCESSING_PROMPT]
  │
  │ prompt tokens consumed
  ▼
[DONE_PROMPT] ───────────────┐
  │                          │
  │ child slot activated     │ parent activates
  │ (copy_state_to +         │ children in WAIT_OTHER
  │  state = DONE_PROMPT)    │
  ▼                          │
[GENERATING] ◄───────────────┘
  │
  │ generation complete (n_remaining <= 0, stop token, or max_tokens)
  │ process_token() returns false → send_final_response() → release()
  │
  ▼
[IDLE]
```

### Detail of each transition

1. **IDLE → STARTED** (parent) / **IDLE → WAIT_OTHER** (child)
   - Set in `launch_slots_with_parent_task()` (line ~1823)
   - Parent gets `SLOT_STATE_STARTED`
   - Child gets `SLOT_STATE_WAIT_OTHER`
   - `stats.update_prompt_start()` is called at this point (first timestamp)

2. **STARTED → PROCESSING_PROMPT**
   - Set in `build_batch()` when the slot first enters prompt token processing (line ~3174)
   - `stats.update_prompt_start()` is called here (ensures timestamp even if slot was skipped earlier)

3. **PROCESSING_PROMPT → DONE_PROMPT**
   - Set in `build_batch()` when `slot.prompt.n_tokens() == slot.task->n_tokens()` (line ~3622)
   - Batch output flag set on last token position

4. **DONE_PROMPT → GENERATING**
   - Set in `post_decode()` after logits are computed and sampled (line ~3866)
   - Speculative decoding is initialized if enabled

5. **GENERATING → IDLE**
   - Triggered when `process_token()` returns `false` (stop token, truncation, budget exceeded)
   - Calls `send_final_response()` → `release()` → sets `state = SLOT_STATE_IDLE` (line ~553)
   - `release()` also clears prompt tokens for child slots and calls `callback_on_release()`

6. **WAIT_OTHER → DONE_PROMPT**
   - Set in `decode()` after parent completes prompt (line ~3805)
   - Parent copies its KV state to the child via `copy_state_to()`

7. **Special: prompt tokens cached**
   - If prompt tokens are already in KV cache, the slot may go directly from STARTED to DONE_PROMPT without entering PROCESSING_PROMPT (the batch skips tokens already cached)

## Per-slot Metrics (`server_slot_stats`)

Defined in `tools/server/server-common.h`. Captured during slot lifetime:

### Timestamps (absolute, microseconds)

| Field           | Set When                          | Notes                            |
|-----------------|-----------------------------------|----------------------------------|
| `t_start`       | `update_prompt_start()`           | First timestamp, transition STARTED → PROCESSING_PROMPT |
| `t_prompt_last` | `update_prompt_last()`            | Updated continuously during PROCESSING_PROMPT |
| `t_gen_last`    | `update_gen_last()`               | Updated per generated token in GENERATING |

### Counters

| Field                | Meaning                                           | Set When                          |
|----------------------|---------------------------------------------------|-----------------------------------|
| `n_prompt_cached`    | Tokens reused from prompt cache                   | `metrics_add_prompt_cached()`     |
| `n_prompt_processed` | Tokens actually processed through decode          | `metrics_queue_prompt()`          |
| `n_gen`              | Tokens generated                                  | Each call to `process_token()`    |
| `n_draft_tokens`     | Total draft tokens generated (speculative)        | `metrics_update_gen()`            |
| `n_draft_accepted`   | Draft tokens actually accepted by target model    | `metrics_update_gen()`            |
| `n_draft_verif_steps`| Speculative verification steps                    | Each speculative verification     |

### Derived metrics (computed on demand)

| Method                     | Calculation                                              |
|----------------------------|----------------------------------------------------------|
| `t_elapsed_us()`           | `ggml_time_us() - t_start`                               |
| `t_prompt_ms()`            | `(t_prompt_last - t_start) / 1000.0`                     |
| `t_gen_us()`               | `t_gen_last - t_prompt_last` (min 1 us)                  |
| `t_gen_ms()`               | `t_gen_us() / 1000.0`                                    |
| `n_gen_steps()`            | `n_gen > 0 ? n_gen - 1 : 0`                              |
| `t_prompt_per_token_ms()`  | `t_prompt_ms() / n_prompt_processed`                     |
| `t_gen_per_token_ms()`     | `t_gen_ms() / n_gen_steps()`                             |
| `n_prompt_tps()`           | `1000.0 / t_prompt_ms() * n_prompt_processed`            |
| `n_gen_tps()`              | `1000.0 / t_gen_ms() * n_gen_steps()`                    |

## Server-global Metrics (`server_metrics`)

Defined in `tools/server/server-common.h`. Cumulative since server start:

### Buckets (reset every scrape)

| Field            | Count                | Steps                        | Time            |
|------------------|----------------------|------------------------------|-----------------|
| `prompt_bucket`  | Tokens processed     | Tokens (same as count)       | Microseconds    |
| `predict_bucket` | Tokens generated     | Decode steps (excl. first)   | Microseconds    |

### Cumulative counters

| Field                       | Meaning                                      |
|-----------------------------|----------------------------------------------|
| `prompt.count`              | Total prompt tokens (excl. cached)           |
| `prompt.time`               | Total prompt processing time                 |
| `predict.count`             | Total generated tokens                       |
| `predict.time`              | Total generation time                        |
| `n_prompt_cached`           | Total prompt tokens reused from cache        |
| `n_decode`                  | Total llama_decode() calls                   |
| `n_tokens_max`              | Largest sequence length observed             |
| `n_busy_slots`              | Sum of busy slots per decode call            |
| `n_draft_tokens`            | Total draft tokens (speculative)             |
| `n_draft_accepted`          | Total accepted draft tokens                  |
| `n_draft_verif_steps`       | Total verification steps (speculative)       |
| `n_accepted_per_pos[]`      | Accepted tokens per draft position           |

### KV cache metrics

| Field                    | Meaning                                          |
|--------------------------|--------------------------------------------------|
| `kvcache_capacity_tokens`| Total context window (`llama_n_ctx`)             |
| `kvcache_used_tokens`    | Sum of prompt tokens across active slots (capped) |
| `kvcache_slots[]`        | Per-slot: id, state string, n_tokens             |

### Memory metrics

| Field                 | Meaning                                |
|-----------------------|----------------------------------------|
| `memory_context_bytes`| Context/KV cache buffer bytes (all devices) |
| `memory_model_bytes`  | Model weights bytes (all devices)      |

## How Metrics Flow

1. **Prompt processing**: `metrics_pre_decode()` → `metrics_queue_prompt(n_tokens)` → `metrics_flush_prompt()`
2. **Generation**: `metrics_post_decode()` increments `n_decode`, `n_busy_slots`, `n_tokens_max`
3. **Generation stats**: `metrics_update_gen()` adds draft tokens/accepted to both slot stats and server metrics
4. **Bucket reset**: `reset_bucket()` called via `SERVER_TASK_TYPE_METRICS` task after each metrics scrape
5. **Readout**: `get_metrics()` returns a snapshot of `server_metrics` for JSON serialization

## Multi-slot Task (parent/child)

For tasks with `n_cmpl > 1` (same prompt, multiple completions):
- **Parent** slot processes the prompt (STARTED → PROCESSING_PROMPT → DONE_PROMPT)
- **Child** slots wait in WAIT_OTHER
- After parent's prompt decode, `decode()` copies KV state to children (line ~3789)
- All slots then independently generate (DONE_PROMPT → GENERATING)
- Children clear their prompt tokens in `release()` (parent context is sufficient)
