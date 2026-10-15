# Model States

## Enum: `server_model_status` (server-models.h)

| Status | String | Meaning |
|---|---|---|
| `SERVER_MODEL_STATUS_DOWNLOADING` | `"downloading"` | Model file is being downloaded (child process in `download` mode) |
| `SERVER_MODEL_STATUS_DOWNLOADED` | `"downloaded"` | Download completed successfully |
| `SERVER_MODEL_STATUS_UNLOADED` | `"unloaded"` | Model instance is not running (child process exited or never started) |
| `SERVER_MODEL_STATUS_LOADING` | `"loading"` | Child process has been spawned, waiting for it to report READY |
| `SERVER_MODEL_STATUS_LOADED` | `"loaded"` | Child process reports READY, model is accepting requests |
| `SERVER_MODEL_STATUS_SLEEPING` | `"sleeping"` | Model is idle — all slots finished processing, child process waiting for next request |

### Helper predicates (`server_model_meta`)

- **`is_ready()`** — `status == LOADED`
- **`is_running()`** — `status` is LOADED, LOADING, or SLEEPING
- **`is_ready_or_sleep()`** — `status` is LOADED or SLEEPING
- **`is_failed()`** — `status == UNLOADED` AND `exit_code != 0`

## State Diagram

```
                    ┌─────────────────────────────────┐
                    │                                 │
                    ▼                                 │
  UNLOADED ◄──── LOADING ◄─── spawn child process     │
     │              │         │                       │
     │              │         │ child reports READY   │
     │              │         ▼                       │
     │           (fail)  LOADED ──────────────────────┘
     │              │         │
     │              │         │ all slots idle, child reports SLEEPING
     │              │         ▼
     │              │   SLEEPING ──────────────────────┐
     │              │         │                         │
     │              │         │ new request arrives,   │
     │              │         │ child reloads &        │
     │              │         │ reports READY          │
     │              │         ▼                         │
     │              └────── LOADED (loop)              │
     │                                                 │
     └────── reload() called (reset exit_code=0) ──────┘
```

## State Transitions

### LOADING → LOADED
- **Trigger:** Child process sends `SERVER_STATE_READY` via router notification.
- **Code:** `server_models.cpp` line ~1883, `update_status(name, {SERVER_MODEL_STATUS_LOADED, ...})`.
- **Side effect:** `sched->tick()` fires (slot count changes).

### LOADED → SLEEPING
- **Trigger:** Child process detects all slots idle and sends `SERVER_STATE_SLEEPING`.
- **Code:** `server_models.cpp` line ~1895, `update_status(name, {SERVER_MODEL_STATUS_SLEEPING})`.
- **Side effect:** `sched->tick()` fires. Cached responses updated.

### SLEEPING → LOADED
- **Trigger:** New request arrives. Child process reloads the model and reports `SERVER_STATE_READY`.
- **Code:** Same as LOADING → LOADED, via `callback_state(SERVER_STATE_READY)`.
- **Note:** The child process handles the reload internally; the router just observes the state change.

### LOADING → UNLOADED (failure)
- **Trigger:** Child process exits with non-zero exit code during load phase.
- **Code:** `server_models.cpp` line ~1459, `on_child_exit()` → `update_status(name, {SERVER_MODEL_STATUS_UNLOADED, exit_code})`.
- **Side effect:** `meta.exit_code` is set. `is_failed()` returns true if `exit_code != 0`.

### LOADED → UNLOADED (unload)
- **Trigger:** `unload()` called — sends exit signal to child, waits for it to terminate.
- **Code:** `server_models.cpp` line ~1459, `on_child_exit()` → `update_status(name, {SERVER_MODEL_STATUS_UNLOADED, exit_code})`.

### UNLOADED → LOADING (reload)
- **Trigger:** `reload()` called — same code path as initial `load()`.
- **Code:** `server_models.cpp` line ~1375, `inst.meta.status = SERVER_MODEL_STATUS_LOADING`.
- **Prerequisite:** `exit_code` must be 0 (cleared before reload is allowed).

### DOWNLOADING → DOWNLOADED
- **Trigger:** Child process in `download` mode completes the file download.
- **Code:** `server_models.cpp` line ~1564, `it->second.meta.status = SERVER_MODEL_STATUS_DOWNLOADED`.
- **Note:** Downloaded models are cleaned up on the next `load_models()` call.

### DOWNLOADING → LOADING
- **Trigger:** Download complete, model is now loaded for inference.
- **Code:** Transition happens on the next `load()` or `load_models()` cycle.

## Status in `/models` JSON Response

The `router_models_json` handler (line ~185) maps these internal statuses to the JSON output:

| Internal Status | JSON `status` value | Collection |
|---|---|---|
| Not in `all_meta` | `"not_found"` | `available` |
| UNLOADED | `"unloaded"` | `available` |
| LOADED | `"loaded"` | `active` |
| SLEEPING | `"sleeping"` | `active` |
| LOADING (transient) | falls through to `"unloaded"` | `available` |
| DOWNLOADING / DOWNLOADED | `"unloaded"` | `available` |
