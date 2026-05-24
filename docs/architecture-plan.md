# Models Assemble 架构计划

## 1. 项目定位

Models Assemble 的目标不是再做一个普通的模型 API SDK，而是做一个面向 Coding Agent 的多模型聚合 Provider。

它向 Claude Code、OpenCode、CC-Switch 以及其他支持自定义 Provider 的工具暴露统一的模型接口，然后在后端连接 Kimi、GLM、GPT、DeepSeek、OpenRouter、本地模型等不同供应商。

核心价值是：

- 一个入口地址，接入多个 coding plan 或模型 API。
- 一个统一模型名体系，屏蔽各家真实模型名和 endpoint 差异。
- 一个路由策略层，根据任务类型、上下文长度、成本、可用性选择模型。
- 一个协议转换层，让 Claude Code / OpenCode 能调用不同 Provider。
- 一个可靠代理层，处理 streaming、tool use、fallback、重试、日志、预算和限速。

简化后的关系如下：

```text
Claude Code / OpenCode / CC-Switch
        |
        | Anthropic-compatible / OpenAI-compatible request
        v
Models Assemble Gateway
        |
        | route / normalize / stream / fallback / budget
        v
Kimi Coding Plan / GLM Coding Plan / GPT / DeepSeek / OpenRouter / Local Models
```

## 2. 设计原则

### 2.1 不把项目做成“绕过限制的脚本”

有些 coding plan 可能只允许官方支持的工具或框架接入。Models Assemble 不应该把自己定位成绕过限制的 Python 脚本，而应该定位成一个 Provider Gateway：

- 客户端仍然是 Claude Code、OpenCode 等 coding agent。
- 网关负责协议适配、路由和可观测性。
- 对特殊 coding plan 优先采用官方文档支持的 endpoint 和协议语义。
- 对无法稳定或合规接入的 provider，明确标记为 experimental 或 unsupported。

### 2.2 协议优先，而不是模型优先

Coding Agent 调用模型时，不只是发送一段 prompt。它还依赖：

- SSE streaming。
- tool calls。
- tool results。
- system prompt。
- stop reason。
- usage 统计。
- long context。
- reasoning / thinking 字段。
- 错误格式和取消请求语义。

因此系统核心应该先抽象协议，再接入模型。

### 2.3 Provider 可插拔

每家模型厂商的协议都不同，字段也会变化。系统要避免让核心网关绑定某一家格式。

推荐采用：

```text
Client Request
    -> Normalized Request
    -> Provider Request
    -> Provider Stream / Response
    -> Normalized Event
    -> Client-compatible Stream / Response
```

## 3. 推荐技术栈

### 3.1 核心网关

推荐使用 Rust。

核心依赖：

- axum：HTTP server。
- tokio：异步运行时。
- reqwest：上游 HTTP client。
- serde / serde_json：协议序列化。
- futures / async-stream：streaming 管道。
- tracing / tracing-subscriber：结构化日志。
- config / figment：配置加载。
- clap：CLI。
- thiserror / anyhow：错误处理。
- tower / tower-http：中间件、超时、CORS、trace。
- governor 或 tower-governor：限速。

Rust 适合这个项目的原因：

- 长连接和 SSE 转发稳定。
- 并发资源控制清晰。
- 单文件二进制分发体验好。
- 适合作为本地常驻 Provider 服务。
- 强类型能降低协议转换中的字段错误。

### 3.2 辅助工具

Python 可以用于：

- provider 探测脚本。
- 回归测试脚本。
- mock server。
- 数据分析和日志分析。

TypeScript / React 可以用于后期 dashboard：

- Provider 状态。
- API key 配置。
- 使用量统计。
- 路由规则编辑。
- 测试请求面板。

初期不建议先做 UI。先把 provider 网关跑通。

## 4. 目标架构

建议采用 Rust workspace，但早期不要拆得太细。MVP 阶段先保持 3 个 crate，等 normalized schema、adapter trait 和路由边界稳定后再拆分：

```text
models-assemble/
  crates/
    ma-cli/             # CLI: serve, validate, doctor, compat-probe
    ma-server/          # HTTP server, auth, SSE, request lifecycle
    ma-core/            # config, protocol, routing, providers, errors
  docs/
    architecture-plan.md
    architecture-plan.html
  examples/
    config.example.yaml
```

稳定后可以再演进为：

```text
ma-protocol / ma-router / ma-providers / ma-config / ma-observability
```

### 4.1 请求生命周期

```text
1. Client sends request to /v1/messages or /v1/chat/completions
2. Gateway authenticates local API key
3. Gateway parses request into client-specific schema
4. Protocol layer converts it into NormalizedRequest
5. Router selects ProviderTarget
6. Provider adapter converts NormalizedRequest into provider-specific request
7. Gateway opens upstream streaming request
8. Provider adapter maps upstream chunks into NormalizedEvent
9. Protocol layer maps NormalizedEvent back to client-compatible stream
10. Gateway records usage, latency, selected model, errors and fallback chain
```

### 4.2 内部标准模型

内部不要直接使用 OpenAI 或 Anthropic 的结构作为唯一真相。建议定义自己的 normalized schema。

核心结构：

```text
NormalizedRequest
  id
  model_alias
  messages
  system
  tools
  tool_choice
  max_tokens
  temperature
  stream
  metadata
  client_capabilities

NormalizedMessage
  role: system | user | assistant | tool
  content: text | image | tool_result | mixed

NormalizedEvent
  message_start
  content_block_start
  content_delta
  content_block_stop
  tool_call_delta
  thinking_delta
  usage_delta
  message_stop
  error
```

这样可以把 Claude Code、OpenCode、OpenAI-compatible provider、Anthropic-compatible provider 之间的差异隔离开。

### 4.3 Streaming 与 tool use 状态机

只定义 `NormalizedEvent` 还不够。实际实现必须把流式协议当成状态机处理，否则多 tool call、参数分片、content block 顺序和半截 JSON 都容易出错。

Anthropic-compatible stream 的目标顺序：

```text
message_start
  content_block_start
    content_block_delta*
  content_block_stop
  message_delta*
message_stop
```

OpenAI-compatible stream 的目标顺序：

```text
chat.completion.chunk*
data: [DONE]
```

内部 stream assembler 至少要维护：

- message id。
- choice index。
- content block index。
- tool call index。
- tool call id。
- partial JSON buffer。
- finish reason。
- token usage。
- upstream request state。

fallback 原则：

- stream 开始前可以 fallback。
- 已经向客户端输出 token 或 tool delta 后，不做透明 fallback。
- stream 中途失败时，返回客户端协议兼容的错误或终止事件，并记录 provider error。

### 4.4 内部错误模型

fallback、重试和客户端错误映射都应该依赖统一错误结构。

```text
NormalizedError
  category: auth | rate_limited | timeout | overloaded | invalid_request | provider_bug | network | unknown
  retryable: true | false
  http_status
  provider_code
  safe_message
  raw_debug
```

`safe_message` 可以返回给客户端；`raw_debug` 只进入本地 debug 日志，并且必须脱敏。

## 5. 对外接口

### 5.1 Anthropic-compatible API

优先支持：

```text
POST /v1/messages
GET  /v1/models
```

目标客户端：

- Claude Code。
- 兼容 Anthropic Messages API 的工具。
- 可能通过 CC-Switch 间接接入的工具。

需要重点支持：

- `stream: true`。
- `content_block_start` / `content_block_delta` / `message_delta` / `message_stop`。
- tool use。
- stop reason。
- usage。

### 5.2 OpenAI-compatible API

同时支持：

```text
POST /v1/chat/completions
GET  /v1/models
```

目标客户端：

- OpenCode。
- Cline / Roo Code。
- Continue。
- 常见 OpenAI-compatible 工具。

需要重点支持：

- SSE `data: ...`。
- tool_calls。
- finish_reason。
- usage。
- model list。

### 5.3 管理接口

初期可以只做 CLI，不做 HTTP dashboard。

推荐 CLI：

```text
ma serve --config config.yaml
ma validate --config config.yaml
ma test-provider glm-main
ma list-models
ma doctor
```

后续再考虑：

```text
GET /admin/providers
GET /admin/usage
POST /admin/reload
POST /admin/test-provider
```

管理 HTTP API 默认不启用。即使启用，也应该默认只绑定 `127.0.0.1`，并复用本地管理 key。

## 6. 配置设计

建议使用 YAML 或 TOML。YAML 更适合复杂路由规则。

示例：

```yaml
server:
  bind: 127.0.0.1:8787
  api_keys:
    - ma-local-dev-key

models:
  assemble-main:
    provider: glm-coding
    model: glm-5.1

  assemble-plan:
    provider: openai-coding
    model: gpt-5.1-codex

  assemble-cheap:
    provider: deepseek
    model: deepseek-v4-flash

  assemble-long:
    provider: kimi
    model: kimi-long-context

providers:
  glm-coding:
    type: zhipu_coding_plan
    base_url: https://example.zhipu.endpoint
    api_key_env: ZHIPU_CODING_API_KEY

  openai-coding:
    type: openai_compatible
    base_url: https://api.openai.com/v1
    api_key_env: OPENAI_API_KEY

  deepseek:
    type: openai_compatible
    base_url: https://api.deepseek.com/v1
    api_key_env: DEEPSEEK_API_KEY

  kimi:
    type: openai_compatible
    base_url: https://api.moonshot.cn/v1
    api_key_env: KIMI_API_KEY

routing:
  default: assemble-main
  rules:
    - name: long-context-to-kimi
      when:
        context_tokens_gt: 120000
      use: assemble-long

    - name: subagent-cheap-model
      when:
        metadata:
          subagent: true
      use: assemble-cheap

fallback:
  assemble-main:
    - assemble-plan
    - assemble-cheap
```

### 6.1 安全与凭据模型

默认安全模型是 single-user local gateway。第一版不承诺多用户隔离，也不应该默认暴露到公网。

必须遵守：

- 默认绑定 `127.0.0.1`，只有用户显式配置时才监听 `0.0.0.0`。
- 本地客户端 API key 可以写入配置，但 provider API key 不建议写入 YAML。
- provider key 优先从环境变量、系统 keychain 或 secret file 读取。
- `ma doctor` 检查配置文件权限，发现过宽权限时警告。
- 日志默认脱敏 `Authorization`、API key、cookies、token、常见 secret 字段。
- 默认不记录 prompt、response、tool 参数正文。
- debug trace 可以记录脱敏后的 chunk，但必须显式开启。
- dashboard/admin API 默认关闭；开启时默认只允许 localhost。
- provider token 绝不透传给客户端。

### 6.2 缓存边界

第一版不做 response cache。Coding Agent 请求高度上下文相关，缓存回答容易带来隐私、污染和错误复用风险。

可以优先做：

- model list cache。
- provider health cache。
- token counting cache。
- provider capability cache。

后续如果做 semantic cache，必须支持 TTL、项目隔离、provider 隔离、显式开启和一键清理。

## 7. Provider Adapter 设计

每个 adapter 实现统一 trait。

概念接口：

```rust
trait ProviderAdapter {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn compliance(&self) -> ProviderCompliance;
    async fn health_check(&self) -> ProviderHealth;
    async fn list_models(&self) -> Vec<ModelInfo>;
    async fn complete(&self, request: NormalizedRequest) -> ProviderResult<NormalizedResponse>;
    async fn stream(&self, request: NormalizedRequest) -> ProviderResult<NormalizedStream>;
}
```

第一批 adapter：

- OpenAI-compatible adapter。
- Anthropic-compatible adapter。
- Zhipu GLM Coding Plan adapter。
- Kimi adapter。
- DeepSeek adapter。
- OpenRouter adapter。

实际开发时，先做 OpenAI-compatible adapter，因为它能覆盖最多 provider。

### 7.1 能力协商

router 不应该只按模型名路由，还要确认 provider 是否具备请求所需能力。

能力字段至少包括：

- streaming。
- tools。
- parallel tool calls。
- vision。
- JSON mode。
- reasoning / thinking。
- max context tokens。
- max output tokens。
- native protocol。

如果请求需要 tool use，但目标 provider 不支持 tools，router 应该拒绝或选择 fallback，而不是静默降级。

### 7.2 Provider 接入等级

每个 adapter 都要声明合规接入等级：

```text
official_api              官方公开 API。
official_coding_endpoint  官方文档允许的 coding 工具 endpoint。
compatible_proxy          协议兼容，但官方未明确承诺该用途。
unsupported               不接入。
```

adapter 文档中必须包含 `compliance_notes`，说明接入方式、已知限制和用户需要自行确认的条款。

## 8. 路由策略

路由层要支持从简单到复杂逐步升级。

### 8.1 第一阶段

只按请求里的 model alias 路由：

```text
assemble-main -> glm-5.1
assemble-plan -> gpt-codex
assemble-cheap -> deepseek-flash
```

### 8.2 第二阶段

支持 fallback：

```text
glm-5.1 timeout -> gpt-codex
gpt-codex rate_limited -> kimi
cheap model failed -> main model
```

### 8.3 第三阶段

支持规则路由：

- 长上下文走 Kimi。
- 子任务走 cheap model。
- 规划任务走强 reasoning 模型。
- 简单编辑走低成本模型。
- provider 当前不可用时自动绕开。

不要一开始做复杂 DSL。早期只支持显式 model alias、上下文长度、provider health、错误 fallback。任务语义分类放到后期，否则很容易变成不稳定的分类系统。

### 8.4 限流与预算

限流和预算要进入路由决策，而不是只做日志统计。

第一版建议支持：

- local client rate limit。
- per-provider concurrency limit。
- per-provider retry budget。
- daily/monthly token cap。
- daily/monthly cost cap。
- fallback cost policy。

fallback cost policy 尤其重要。默认不应该在用户无感知的情况下从便宜模型 fallback 到昂贵模型，除非配置中明确允许。

## 9. MVP 路线

### Milestone 0: 项目骨架与兼容性探针

目标：

- Rust workspace 初始化。
- CLI 可启动 server。
- 配置文件可加载。
- `/health` 可访问。
- `ma compat-probe` 可启动最小 mock provider。
- 最小 mock `/v1/messages`、`/v1/chat/completions`、`/v1/models`。
- 输出最小 SSE event sequence，用于验证 Claude Code / OpenCode / CC-Switch 是否接受自定义 base_url。

完成标准：

```text
ma serve --config examples/config.example.yaml
curl http://127.0.0.1:8787/health
ma compat-probe --anthropic
ma compat-probe --openai
```

### Milestone 1: OpenAI-compatible 入口和出口

目标：

- 实现 `POST /v1/chat/completions`。
- 接入一个 OpenAI-compatible provider。
- 支持 non-stream 和 stream。
- 支持 `/v1/models`。

完成标准：

```text
OpenCode -> Models Assemble -> OpenAI-compatible provider
```

验收用例：

- non-stream chat 成功。
- stream 首 token 延迟可接受。
- stream 正常输出完整 `[DONE]`。
- tool_calls roundtrip。
- 401 / 429 / 5xx 错误映射正确。
- client disconnect 能取消上游请求。

### Milestone 2: 模型别名和 fallback

目标：

- `assemble-main`、`assemble-cheap` 等别名可用。
- provider timeout / 429 / 5xx 时 fallback。
- 日志记录实际命中的 provider 和模型。

完成标准：

```text
请求 assemble-main，上游失败后自动切到 fallback provider。
```

### Milestone 3: Anthropic-compatible 入口

目标：

- 实现 `POST /v1/messages`。
- 支持 Anthropic SSE event。
- 支持基础 tool use 映射。

完成标准：

```text
Claude Code / Anthropic-compatible client -> Models Assemble -> OpenAI-compatible provider
```

### Milestone 4: Coding Plan Provider

目标：

- 接入 GLM Coding Plan。
- 接入 Kimi Coding Plan。
- 接入 GPT coding plan 或 OpenAI-compatible coding endpoint。
- 明确每个 provider 的能力矩阵。

能力矩阵示例：

```text
Provider        Streaming   Tool Use   Thinking   Long Context   Status
GLM Coding      yes         yes        unknown    yes            beta
Kimi            yes         partial    no         yes            beta
OpenAI          yes         yes        yes        yes            stable
DeepSeek        yes         yes        no         medium         stable
```

### Milestone 5: 可观测性和本地体验

目标：

- `ma doctor` 检查配置、网络和 API key。
- request id 全链路日志。
- token usage 统计。
- provider latency 统计。
- 本地 dashboard 可选。

默认采集字段：

- request id。
- client。
- model alias。
- provider。
- latency。
- status。
- input/output tokens。
- fallback chain。

默认不采集 prompt、response 和 tool 参数正文。

## 10. 风险清单

### 10.1 Claude Code 自定义 Provider 兼容性

这是项目最大前置风险。需要验证：

- 是否能配置自定义 Anthropic-compatible base_url。
- 对 SSE event 顺序是否严格。
- 对 tool use 字段是否严格。
- 对错误格式是否敏感。

### 10.2 Coding Plan 接入限制

不同 provider 对 coding plan 的限制可能不同。

处理策略：

- 只支持官方文档明确允许的接入方式。
- 对不稳定方案标注 experimental。
- 不把 provider token 暴露给客户端。
- 不承诺所有套餐都可被任意转发。

### 10.3 Tool Use 语义不完全一致

OpenAI 和 Anthropic 的 tool call 流式事件不同。需要专门测试：

- 多 tool call。
- tool call 参数分片。
- tool result 回传。
- agent 中断请求。
- provider 返回非法 JSON 参数。

### 10.4 Streaming 边界条件

需要覆盖：

- 上游超时。
- 下游断开连接。
- fallback 是否允许发生在 stream 开始后。
- 上游返回半截 JSON。
- 客户端取消请求。

建议原则：

- stream 开始前可以 fallback。
- stream 已经输出给客户端后，不做透明 fallback，只返回标准错误或终止事件。

## 11. 测试策略

### 11.1 单元测试

重点覆盖：

- OpenAI -> Normalized 转换。
- Anthropic -> Normalized 转换。
- Normalized -> provider request 转换。
- provider event -> client event 转换。
- routing rule。
- fallback decision。

### 11.2 集成测试

使用 mock provider：

- 正常 streaming。
- 429。
- 500。
- timeout。
- malformed chunk。
- tool call chunk。

### 11.3 真实客户端测试

按优先级：

```text
OpenCode -> Models Assemble -> mock/OpenAI-compatible provider
Claude Code -> Models Assemble -> mock/OpenAI-compatible provider
CC-Switch -> Models Assemble -> provider
```

## 12. 推荐的第一步

第一步不要直接接 GLM、Kimi、GPT 三家。应该先证明网关闭环。

最小闭环：

```text
OpenCode
  -> Models Assemble /v1/chat/completions
  -> OpenAI-compatible adapter
  -> 任意普通 OpenAI-compatible 模型
```

确认：

- OpenCode 能把你的服务当 provider。
- SSE streaming 没问题。
- 模型别名能工作。
- 日志能看到完整链路。

然后再做：

```text
Claude Code
  -> Models Assemble /v1/messages
  -> OpenAI-compatible adapter
```

最后再接入具体 coding plan。

## 13. 最终形态

最终用户体验应该是：

```text
ma serve --config config.yaml
```

然后在 Claude Code / OpenCode / CC-Switch 中配置：

```text
base_url = http://127.0.0.1:8787
api_key = local-ma-key
model = assemble-main
```

用户不需要关心真实后端是哪家模型。Models Assemble 负责：

- 选择模型。
- 转换协议。
- 保持 streaming。
- fallback。
- 控制成本。
- 记录使用量。
- 暴露统一模型列表。

这就是项目的核心边界：不是替代 Claude Code / OpenCode，而是成为它们背后的多模型 Provider。
