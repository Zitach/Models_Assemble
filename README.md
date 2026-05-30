# Models!Assemble!

这个项目旨在为了订阅了多个模型的用户，可以在claude code里面切换不同的模型，例如主模型使用强大的Opus或者GLM5.1这样的模型，子模型用较为便宜的比如deepseek-v4-flash这样的模型。（达到类似Opencode + OMO的效果）

# 核心场景

比如说在CC-Switch中，我们只需要订阅这个项目的地址，就可以在配置中设置好不同的模型了。

## 当前脚手架

本项目当前采用 Rust workspace：

- `crates/ma-core`：配置、协议和内部错误模型。
- `crates/ma-server`：Axum HTTP 服务、健康检查、模型列表和兼容性 mock endpoint。
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

这些 endpoint 先用于验证 Claude Code、OpenCode、CC-Switch 等客户端是否接受自定义 provider、模型列表和最小流式协议。真实 provider adapter 会在后续里程碑接入。

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

然后把上游请求里的 `model` 改写成真实模型名 `gpt-coding-model`，并转发到 provider 的 `base_url`。

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

当前实现会透传 OpenAI-compatible JSON 请求体，只读取并改写 `model` 字段；非流式响应和流式 SSE 响应都会从上游直接返回。

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

这样用户在客户端只看到 `assemble-main`、`assemble-claude` 这样的统一模型名，但每个上游仍然使用它自己的原生协议。

当前支持：

- `openai_compatible`：`POST /v1/chat/completions`，支持非流式和 SSE 流式转发。
- `anthropic_compatible`：`POST /v1/messages`，支持非流式和 SSE 流式转发。
- `mock`：用于 `compat-probe` 和本地客户端兼容性测试。

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

当前流式请求不会做透明 fallback。原因是 SSE 一旦开始向客户端输出，切换 provider 会破坏客户端看到的 event 状态机。后续可以只在“上游尚未输出任何 chunk”时做更精细的 stream fallback。
