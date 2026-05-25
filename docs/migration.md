# Config Migration Guide

## What Changed

The normalized protocol is now the default and only request path. All requests go through the `ProviderAdapter` trait, which unifies how Models Assemble talks to every upstream provider.

### Removed

- `experimental.use_normalized` flag — no longer needed. The normalized path is automatic.

### Added

- `server.first_token_timeout_secs` — optional `u64`, default `15`.
  - Controls how long to wait for the first SSE chunk when using stream fallback.
  - If the first chunk doesn't arrive within this window, the request may be retried on the next fallback model.

## Breaking Changes

None. Old configs still work. The normalized path is applied automatically; you don't need to opt in.

## How to Test the New Path

1. Start the server with your existing config:
   ```bash
   cargo run -p ma-cli -- serve --config examples/config.example.yaml
   ```

2. Send a chat completion request:
   ```bash
   curl http://127.0.0.1:8787/v1/chat/completions \
     -H "Authorization: Bearer ma-local-dev-key" \
     -H "Content-Type: application/json" \
     -d '{"model":"assemble-main","messages":[{"role":"user","content":"hello"}]}'
   ```

3. Verify the response uses the correct upstream model and format.

4. Test a specific provider directly:
   ```bash
   cargo run -p ma-cli -- test-provider assemble-main --config examples/config.example.yaml
   ```

## Updating Your Config (Optional)

If you previously had `experimental.use_normalized: true`, remove that line. It's no longer recognized.

If you want to customize stream fallback behavior, add:

```yaml
server:
  first_token_timeout_secs: 15
```

The default is 15 seconds. Increase it for slower providers, decrease it for faster fallback switching.
