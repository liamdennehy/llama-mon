```sh
$ curl -s http://localhost:8080/metrics?model=example-model -H "Accept: application/json" | jq
{
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
}
```
