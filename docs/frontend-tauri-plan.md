# Models Assemble Frontend + Tauri Plan

## Goal

Build a polished desktop-first configuration UI for Models Assemble. The app helps users manage coding plans/providers, prefill common templates, edit request URLs/API keys/model lists, test providers, and eventually launch or configure the local gateway.

The first implementation should be a React + Vite frontend prepared for Tauri v2 packaging. It should work as a normal web dev app today and be ready to call Tauri commands later.

## Product Shape

The UI should feel like a serious local control panel for coding-agent infrastructure:

- Dense, calm, professional layout.
- Dark/light mode toggle.
- Provider list with prefilled templates for DeepSeek, Kimi, and GLM.
- Detail editor for each coding plan.
- Clear inputs for request URL, API key env name or secret value, provider protocol type, and model list.
- Visible generated configuration preview.
- Test-provider actions for non-stream and stream checks.
- Tauri-ready backend bridge.

## Information Architecture

### Main Areas

1. **Plan Sidebar**
   - Search plans.
   - Template badges: DeepSeek, Kimi, GLM.
   - Custom plans.
   - Status markers: configured, missing key, last test passed/failed.

2. **Plan Detail**
   - Plan name.
   - Provider protocol:
     - `anthropic_compatible`
     - `openai_compatible`
     - future: `direct_config`
   - Request base URL.
   - API key environment variable.
   - Optional raw API key input, stored later through Tauri secure storage.
   - Models list editor.
   - Default model mapping:
     - main
     - haiku/fast
     - sonnet/main
     - opus/max
     - subagent
   - Request overrides JSON.

3. **Generated Config**
   - YAML preview for `examples/config.example.yaml`-style provider config.
   - Claude Code env preview.
   - Copy/save actions later.

4. **Test Panel**
   - `test-provider <alias>`.
   - Stream test toggle.
   - Output summary.

## Initial Templates

### DeepSeek

```yaml
provider: deepseek-anthropic
type: anthropic_compatible
base_url: https://api.deepseek.com/anthropic
model: deepseek-v4-pro
```

### Kimi

```yaml
provider: kimi-coding
type: anthropic_compatible
base_url: https://api.kimi.com/coding/v1
model: kimi-for-coding
```

Kimi direct Claude Code docs often show `https://api.kimi.com/coding/`, but Models Assemble appends `/messages`, so the provider base URL should include `/v1`.

### GLM

```yaml
provider: glm-anthropic
type: anthropic_compatible
base_url: https://open.bigmodel.cn/api/anthropic/v1
model: glm-5.1
```

GLM direct SDK/Claude-style examples may show `https://open.bigmodel.cn/api/anthropic`, but the messages endpoint is `/api/anthropic/v1/messages`.

## Frontend Technical Plan

- React 19 style with `createRoot` from `react-dom/client`.
- Vite + TypeScript.
- Local component state first; no global state library for MVP.
- CSS variables for day/night themes.
- `lucide-react` for icons.
- No backend dependency for first screen; include a local adapter abstraction:
  - Browser mode: in-memory/localStorage later.
  - Tauri mode: call `window.__TAURI__.core.invoke`.

## Tauri Plan

Directory:

```text
apps/desktop/
  package.json
  index.html
  src/
  src-tauri/
    Cargo.toml
    tauri.conf.json
    src/
```

Tauri v2 commands to add later:

- `load_config()`
- `save_config(config)`
- `test_provider(alias, stream)`
- `start_gateway()`
- `stop_gateway()`

For this pass, commands can be stubs so the desktop shell is structurally ready.

## Backend Preparation

Current Rust CLI already has:

- config parsing.
- provider templates in YAML.
- `test-provider`.
- proxy server.

Next backend step after UI scaffold:

- Extract config read/write into reusable crate functions.
- Add JSON DTOs for UI.
- Add Tauri commands that call existing CLI/core functions directly instead of shelling out.
- Avoid storing raw API keys in config; prefer environment variable names first, secure storage later.

## Implementation Phases

### Phase 1: UI Shell

- React + Vite app.
- Tauri config.
- Dark/light theme.
- Plan sidebar.
- Detail editor.
- Model list editor.
- Generated JSON/YAML preview.

### Phase 2: Local Persistence

- Save UI state to localStorage for browser dev.
- Add import/export config actions.

### Phase 3: Tauri Bridge

- Add commands for config load/save.
- Add test-provider command calling Rust logic.

### Phase 4: Real Gateway Management

- Start/stop gateway.
- Show listening URL.
- Show recent test results.
