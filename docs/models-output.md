```sh
curl -s http://localhost:8080/models | jq '.data[0].created'
1791993598
```

```sh
$ curl -s http://localhost:8080/models | jq '.data[0].status.preset' -r
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

