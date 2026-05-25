# Models!Assemble!


这个项目旨在为了订阅了多个模型的用户，可以在claude code里面切换不同的模型，例如主模型使用强大的Opus或者GLM5.1这样的模型，子模型用较为便宜的比如deepseek-v4-flash这样的模型。（达到类似Opencode + OMO的效果）

# 核心场景

比如说在CC-Switch中，我们只需要订阅这个项目的地址，就可以在配置中设置好不同的模型了。

## 当前脚手架

本项目当前采用 Rust workspace：

- `crates/ma-core`：配置、标准化协议类型（`NormalizedRequest`、`NormalizedEvent`）、内部错误模型和 `ProviderAdapter` trait。
- `crates/ma-server`：Axum HTTP 服务、健康检查、模型列表、兼容性 mock endpoint，以及 `OpenAiAdapter`、`AnthropicAdapter`、`MockAdapter` 和 tool mapping 模块。
- `crates/ma-cli`：命令行入口。

常用命令：

```powershell
cargo check --workspace
cargo run -p ma-cli -- validate --config examples/config.example.yaml
cargo run -p ma-cli -- doctor --config examples/config.example.yaml
cargo run -p ma-cli -- test-provider assemble-mock --config examples/config.example.yaml
cargo run -p ma-cli -- compat-probe --bind 127.0.0.1:8787
cargo run -p ma-cli -- serve --config examples/config.example.yaml
```

当前 `compat-probe` 已提供：

- `GET /health`
- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /v1/messages`

这些 endpoint 先用于验证 Claude Code、OpenCode、CC-Switch 等客户端是否接受自定义 provider、模型列表和最小流式协议。真实 provider adapter 已在后续里程碑中接入，采用 Normalized Protocol 架构。

## OpenAI-compatible 转发

`/v1/chat/completions` 已支持 OpenAI-compatible provider 转发。请求里的 `model` 使用本项目的模型别名，例如：

```json
{
  "model": "assemble-main",
  "messages": [
    { "role": "user", "content": "hello" }
  ],
  "stream": false
}
```

服务会在 `examples/config.example.yaml` 中查到：

```yaml
assemble-main:
  provider: openai-compatible
  model: gpt-coding-model
```

然后把上游请求里的 `model` 改写成真实模型名 `gpt-coding-model`，并通过 `OpenAiAdapter` 转换为标准化请求后转发到 provider 的 `base_url`。响应也会经过标准化事件转换后返回给客户端。

运行示例：

```powershell
$env:OPENAI_API_KEY="你的 key"
cargo run -p ma-cli -- serve --config examples/config.example.yaml
```

本地请求需要带配置里的 local API key：

```powershell
Invoke-RestMethod `
  -Uri http://127.0.0.1:8787/v1/chat/completions `
  -Method Post `
  -Headers @{ Authorization = "Bearer ma-local-dev-key" } `
  -ContentType "application/json" `
  -Body '{"model":"assemble-main","messages":[{"role":"user","content":"hello"}],"stream":false}'
```

当前实现采用 Normalized Protocol 架构：客户端请求先被解析为 `NormalizedRequest`，然后通过 `ProviderAdapter` trait 路由到对应的上游 adapter。OpenAI-compatible 和 Anthropic-compatible 请求都会经过统一的标准化层，再转换为各 provider 的原生协议。非流式响应和流式 SSE 响应都会从上游返回，但中间会经过标准化事件转换。

## Provider 协议类型

配置 provider 时需要显式选择上游协议类型。这个选择决定 Models Assemble 用哪种原生协议和上游通信。

```yaml
providers:
  openai-compatible:
    type: openai_compatible
    base_url: https://api.openai.com/v1
    api_key_env: OPENAI_API_KEY

  anthropic-compatible:
    type: anthropic_compatible
    base_url: https://api.anthropic.com/v1
    api_key_env: ANTHROPIC_API_KEY
```

然后模型别名挂到对应 provider：

```yaml
models:
  assemble-main:
    provider: openai-compatible
    model: gpt-coding-model

  assemble-claude:
    provider: anthropic-compatible
    model: claude-opus-coding-model

  assemble-deepseek:
    provider: deepseek-anthropic
    model: deepseek-v4-pro
```

这样用户在客户端只看到 `assemble-main`、`assemble-claude` 这样的统一模型名。每个上游仍然使用它自己的原生协议，但请求和响应都会经过标准化层进行协议转换。

当前支持：

- `openai_compatible`：`POST /v1/chat/completions`，通过 `OpenAiAdapter` 支持非流式和 SSE 流式转发。
- `anthropic_compatible`：`POST /v1/messages`，通过 `AnthropicAdapter` 支持非流式和 SSE 流式转发。
- `mock`：用于 `compat-probe` 和本地客户端兼容性测试，通过 `MockAdapter` 提供。

所有 provider 类型都通过统一的 `ProviderAdapter` trait 接入，支持标准化的请求转换、响应转换和错误处理。

DeepSeek Anthropic-compatible 示例：

```yaml
providers:
  deepseek-anthropic:
    type: anthropic_compatible
    base_url: https://api.deepseek.com/anthropic
    api_key_env: DEEPSEEK_API_KEY
```

运行前设置环境变量：

```powershell
$env:DEEPSEEK_API_KEY="你的 DeepSeek API key"
cargo run -p ma-cli -- serve --config examples/config.example.yaml
```

然后客户端请求 `model: "assemble-deepseek"` 即可走 DeepSeek 的 Anthropic-compatible endpoint。

Kimi Coding Anthropic-compatible 示例：

```yaml
models:
  assemble-kimi:
    provider: kimi-coding
    model: kimi-for-coding

providers:
  kimi-coding:
    type: anthropic_compatible
    base_url: https://api.kimi.com/coding/v1
    api_key_env: KIMI_API_KEY
```

注意：Kimi 给 Claude Code 直连时常见 base URL 是 `https://api.kimi.com/coding/`；Models Assemble 的 `anthropic_compatible` 会在 `base_url` 后追加 `/messages`，所以这里应配置到 `https://api.kimi.com/coding/v1`。

GLM Anthropic-compatible 示例：

```yaml
models:
  assemble-glm:
    provider: glm-anthropic
    model: glm-5.1

providers:
  glm-anthropic:
    type: anthropic_compatible
    base_url: https://open.bigmodel.cn/api/anthropic/v1
    api_key_env: GLM_API_KEY
```

注意：智谱文档里一些 SDK/Claude Code 配置会写 `https://open.bigmodel.cn/api/anthropic`，但实际 messages endpoint 是 `/api/anthropic/v1/messages`。Models Assemble 会追加 `/messages`，所以这里应配置到 `/api/anthropic/v1`。

可以先用 `test-provider` 直接测试上游配置：

```powershell
cargo run -p ma-cli -- test-provider assemble-deepseek --config examples/config.example.yaml
cargo run -p ma-cli -- test-provider assemble-deepseek --config examples/config.example.yaml --stream
cargo run -p ma-cli -- test-provider assemble-kimi --config examples/config.example.yaml
cargo run -p ma-cli -- test-provider assemble-kimi --config examples/config.example.yaml --stream
cargo run -p ma-cli -- test-provider assemble-glm --config examples/config.example.yaml
cargo run -p ma-cli -- test-provider assemble-glm --config examples/config.example.yaml --stream
```

`test-provider` 会检查模型别名、provider 类型、真实上游模型名、API key 环境变量，并发送一条最小测试请求。非流式模式会打印返回模型和首段文本；流式模式会打印 SSE 预览。

## Fallback 策略

非流式 `/v1/chat/completions` 已支持基础 fallback。配置示例：

```yaml
fallback:
  assemble-main:
    - assemble-cheap
```

当 `assemble-main` 的上游返回以下可重试错误时，服务会尝试下一个模型别名：

- `429 Too Many Requests`
- `408 Request Timeout`
- `502 Bad Gateway`
- `503 Service Unavailable`
- `504 Gateway Timeout`
- 其他 `5xx`
- 网络连接或超时错误

`400`、`401` 等非重试错误不会 fallback，会直接返回给客户端。

流式请求支持 first-chunk fallback：如果上游在 `first_token_timeout_secs` 内没有返回任何 SSE chunk，服务会在尚未向客户端输出任何数据前切换到 fallback provider。一旦开始向客户端输出，就不再做透明 fallback，避免破坏 SSE event 状态机。

配置示例（添加 `first_token_timeout_secs`）：

```yaml
server:
  bind: 127.0.0.1:8787
  api_keys:
    - ma-local-dev-key
  first_token_timeout_secs: 15
```

`first_token_timeout_secs` 是可选配置，不设置时使用内部默认值 15 秒。

## Normalized Protocol 架构

Models Assemble 的核心架构是协议感知网关（protocol-aware gateway），不再是简单的透明代理。请求会经过以下流程：

```text
Client Request (OpenAI or Anthropic format)
    -> Handler parses into client-specific schema
    -> NormalizedRequest (统一标准格式)
    -> ProviderAdapter converts to provider-specific request
    -> Upstream provider
    -> ProviderAdapter maps response to NormalizedEvent
    -> Handler converts back to client-compatible format
```

### 核心组件

**Normalized Types** (`crates/ma-core/src/normalized.rs`)

定义了统一的标准化请求和事件类型：

- `NormalizedRequest`：包含 `model_alias`、`messages`、`tools`、`tool_choice`、`max_tokens`、`temperature`、`stream` 等字段
- `NormalizedMessage`：支持 `system`、`user`、`assistant`、`tool` 角色，内容可以是文本、图片或混合类型
- `NormalizedEvent`：标准化事件流，包括 `message_start`、`content_block_start`、`content_delta`、`content_block_stop`、`tool_call_delta`、`thinking_delta`、`usage_delta`、`message_stop`、`error`
- `ToolDef`、`ToolChoice`、`ThinkingConfig`：工具定义和选择配置

**ProviderAdapter Trait** (`crates/ma-core/src/adapter.rs`)

所有 provider 都实现统一的 trait：

```rust
trait ProviderAdapter {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    async fn health_check(&self) -> ProviderHealth;
    async fn complete(&self, request: NormalizedRequest) -> Result<NormalizedResponse>;
    async fn stream(&self, request: NormalizedRequest) -> Result<NormalizedStream>;
}
```

已实现的 adapter：

- `OpenAiAdapter` (`crates/ma-server/src/adapters/openai.rs`)：处理 OpenAI-compatible 协议
- `AnthropicAdapter` (`crates/ma-server/src/adapters/anthropic.rs`)：处理 Anthropic-compatible 协议
- `MockAdapter` (`crates/ma-server/src/adapters/mock.rs`)：用于测试和兼容性验证

### 请求生命周期

1. 客户端发送请求到 `/v1/messages` 或 `/v1/chat/completions`
2. Gateway 验证本地 API key
3. Handler 将请求解析为客户端特定格式
4. 转换为 `NormalizedRequest`
5. 根据模型别名选择对应的 `ProviderAdapter`
6. Adapter 将 `NormalizedRequest` 转换为 provider 特定请求
7. 向上游发送请求
8. Adapter 将上游响应映射为 `NormalizedEvent`
9. Handler 将 `NormalizedEvent` 转换回客户端兼容格式
10. 返回给客户端

## Tool Use 跨协议映射

OpenAI 和 Anthropic 的 tool use 格式不同，Models Assemble 通过 `tool_mapping.rs` 模块处理跨协议转换：

- **Anthropic `tool_use` -> OpenAI `tool_calls`**：将 Anthropic 的 `tool_use` content block 转换为 OpenAI 的 `tool_calls` 数组
- **OpenAI `tool_calls` -> Anthropic `tool_use`**：将 OpenAI 的 `tool_calls` 转换为 Anthropic 的 `tool_use` content block
- **Tool result 回传**：统一处理 tool result 的格式转换
- **流式 tool call**：支持分片 tool call 参数的跨协议映射

## Request ID 中间件

每个请求都会分配唯一的 request ID，贯穿整个请求生命周期：

- 从客户端请求头 `x-request-id` 读取，或自动生成 UUID
- 通过 `request_id` 中间件注入到所有日志和 tracing span 中
- 上游请求会携带相同的 request ID，便于全链路追踪
- 错误响应包含 `x-request-id` 头，方便问题定位

## 测试

当前共有 69 个测试通过，覆盖以下模块：

- `ma-core`：配置解析、标准化类型、错误模型（23 个测试）
- `ma-server`：OpenAI adapter、Anthropic adapter、tool mapping、fallback 逻辑、请求路由
- 集成测试：mock provider、流式响应、错误处理

运行测试：

```powershell
cargo test --workspace
```
