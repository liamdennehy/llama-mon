```sh
curl -s http://localhost:8080/models | jq '.data[0].created'
1791993598
```

The `status` field can be either a plain string (`"loaded"`) or a JSON object:
```json
{
  "value": "loaded",
  "args": ["--cache-prompt", "--host", "0.0.0.0", "--port", "8080", ...]
}
```

The `preset` (INI-style config) may appear in two locations:
1. As a top-level `status_preset` field on the model entry (older API)
2. Embedded in the status object under the `preset` key (newer API)

Example of the newer format with preset embedded in status:
```sh
curl -s http://localhost:8080/models | jq '.data[0].status.preset' -r
[example-model]
cache-prompt = true
jinja = true
log-colors = off
log-file = /var/log/llama-server/example-model.log
batch-size = 1024
ctx-size = 16384
cache-type-k = f16
cache-type-v = f16
flash-attn = true
kv-unified = true
log-verbosity = 2
model = ./example-model-Q6_K.gguf
n-gpu-layers = -1
parallel = 2
threads = 4
threads-batch = 12
ubatch-size = 256
```
