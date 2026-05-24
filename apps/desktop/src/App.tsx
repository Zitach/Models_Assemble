import {
  Activity,
  Braces,
  ChevronRight,
  Copy,
  DatabaseZap,
  Eye,
  EyeOff,
  Moon,
  Network,
  Plus,
  Save,
  Search,
  Server,
  Sun,
  TerminalSquare,
  Trash2,
  Zap,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import './App.css'

type ProtocolType = 'anthropic_compatible' | 'openai_compatible'
type Theme = 'dark' | 'light'
type PlanStatus = 'ready' | 'missing-key' | 'untested'

type Plan = {
  id: string
  name: string
  providerId: string
  protocol: ProtocolType
  baseUrl: string
  apiKeyEnv: string
  apiKeyPreview: string
  authMode: 'x-api-key' | 'bearer'
  models: string[]
  mainModel: string
  fastModel: string
  maxModel: string
  subagentModel: string
  requestOverrides: string
  status: PlanStatus
  template: 'DeepSeek' | 'Kimi' | 'GLM' | 'Custom'
  lastTest: string
}

type TestResult = {
  ok: boolean
  status: string
  text_preview: string
}

declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>
      }
    }
  }
}

const STORAGE_KEY = 'models-assemble.desktop.plans.v1'
const THEME_KEY = 'models-assemble.desktop.theme.v1'

const templates: Plan[] = [
  {
    id: 'deepseek',
    name: 'DeepSeek Coding',
    providerId: 'deepseek-anthropic',
    protocol: 'anthropic_compatible',
    baseUrl: 'https://api.deepseek.com/anthropic',
    apiKeyEnv: 'DEEPSEEK_API_KEY',
    apiKeyPreview: '',
    authMode: 'x-api-key',
    models: ['deepseek-v4-pro', 'deepseek-v4-flash'],
    mainModel: 'deepseek-v4-pro',
    fastModel: 'deepseek-v4-flash',
    maxModel: 'deepseek-v4-pro',
    subagentModel: 'deepseek-v4-flash',
    requestOverrides: '{\n  "max_tokens": 8192\n}',
    status: 'ready',
    template: 'DeepSeek',
    lastTest: 'stream + messages verified',
  },
  {
    id: 'kimi',
    name: 'Kimi Coding',
    providerId: 'kimi-coding',
    protocol: 'anthropic_compatible',
    baseUrl: 'https://api.kimi.com/coding/v1',
    apiKeyEnv: 'KIMI_API_KEY',
    apiKeyPreview: '',
    authMode: 'x-api-key',
    models: ['kimi-for-coding'],
    mainModel: 'kimi-for-coding',
    fastModel: 'kimi-for-coding',
    maxModel: 'kimi-for-coding',
    subagentModel: 'kimi-for-coding',
    requestOverrides: '{\n  "max_tokens": 8192\n}',
    status: 'ready',
    template: 'Kimi',
    lastTest: 'stream + messages verified',
  },
  {
    id: 'glm',
    name: 'GLM 5.1',
    providerId: 'glm-anthropic',
    protocol: 'anthropic_compatible',
    baseUrl: 'https://open.bigmodel.cn/api/anthropic/v1',
    apiKeyEnv: 'GLM_API_KEY',
    apiKeyPreview: '',
    authMode: 'x-api-key',
    models: ['glm-5.1'],
    mainModel: 'glm-5.1',
    fastModel: 'glm-5.1',
    maxModel: 'glm-5.1',
    subagentModel: 'glm-5.1',
    requestOverrides: '{\n  "max_tokens": 8192\n}',
    status: 'ready',
    template: 'GLM',
    lastTest: 'stream + messages verified',
  },
]

function App() {
  const [theme, setTheme] = useState<Theme>(() => readTheme())
  const [plans, setPlans] = useState<Plan[]>(() => readPlans())
  const [selectedId, setSelectedId] = useState(templates[0].id)
  const [search, setSearch] = useState('')
  const [showSecret, setShowSecret] = useState(false)
  const [notice, setNotice] = useState('Ready')
  const [isTesting, setIsTesting] = useState(false)

  const selectedPlan = plans.find((plan) => plan.id === selectedId) ?? plans[0]
  const filteredPlans = plans.filter((plan) =>
    `${plan.name} ${plan.providerId} ${plan.models.join(' ')}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  )

  const generatedConfig = useMemo(
    () => buildConfigPreview(selectedPlan),
    [selectedPlan],
  )
  const claudeEnv = useMemo(() => buildClaudeEnv(selectedPlan), [selectedPlan])

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(stripSecrets(plans)))
  }, [plans])

  useEffect(() => {
    invokeOrMock<Plan[]>('load_plans', {})
      .then((loadedPlans) => {
        if (Array.isArray(loadedPlans) && loadedPlans.length > 0) {
          setPlans(loadedPlans.map(clearPlanSecret))
          setSelectedId(loadedPlans[0].id)
        }
      })
      .catch(() => {
        // Browser preview mode keeps using localStorage.
      })
  }, [])

  useEffect(() => {
    localStorage.setItem(THEME_KEY, theme)
  }, [theme])

  function updateSelectedPlan(update: Partial<Plan>) {
    setPlans((current) =>
      current.map((plan) =>
        plan.id === selectedPlan.id ? { ...plan, ...update } : plan,
      ),
    )
  }

  function addCustomPlan() {
    const next: Plan = {
      id: `custom-${Date.now()}`,
      name: 'Custom Plan',
      providerId: 'custom-provider',
      protocol: 'anthropic_compatible',
      baseUrl: 'https://provider.example.com/v1',
      apiKeyEnv: 'CUSTOM_API_KEY',
      apiKeyPreview: '',
      authMode: 'x-api-key',
      models: ['custom-model'],
      mainModel: 'custom-model',
      fastModel: 'custom-model',
      maxModel: 'custom-model',
      subagentModel: 'custom-model',
      requestOverrides: '{\n  "max_tokens": 4096\n}',
      status: 'untested',
      template: 'Custom',
      lastTest: 'not tested',
    }
    setPlans((current) => [next, ...current])
    setSelectedId(next.id)
  }

  function updateModels(value: string) {
    const models = value
      .split('\n')
      .map((item) => item.trim())
      .filter(Boolean)
    updateSelectedPlan({
      models,
      mainModel: models.includes(selectedPlan.mainModel)
        ? selectedPlan.mainModel
        : (models[0] ?? ''),
      fastModel: models.includes(selectedPlan.fastModel)
        ? selectedPlan.fastModel
        : (models[0] ?? ''),
      maxModel: models.includes(selectedPlan.maxModel)
        ? selectedPlan.maxModel
        : (models[0] ?? ''),
      subagentModel: models.includes(selectedPlan.subagentModel)
        ? selectedPlan.subagentModel
        : (models[0] ?? ''),
    })
  }

  function deleteSelectedPlan() {
    if (plans.length === 1) return
    const remaining = plans.filter((plan) => plan.id !== selectedPlan.id)
    setPlans(remaining)
    setSelectedId(remaining[0].id)
    setNotice('Plan deleted')
  }

  async function savePlans() {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(stripSecrets(plans)))
    try {
      await invokeOrMock('save_plans', { plans: stripSecrets(plans) })
      setNotice('Saved')
    } catch {
      setNotice('Saved locally; Tauri persistence unavailable')
    }
  }

  async function copyText(text: string, label: string) {
    await navigator.clipboard.writeText(text)
    setNotice(`${label} copied`)
  }

  async function testSelectedPlan(stream: boolean) {
    setIsTesting(true)
    setNotice(stream ? 'Running stream test...' : 'Running provider test...')
    try {
      const result = await invokeOrMock<TestResult>('test_provider', {
        plan: selectedPlan,
        stream,
      })
      updateSelectedPlan({
        status: result.ok ? 'ready' : 'missing-key',
        lastTest: `${result.status}: ${result.text_preview || 'no preview'}`,
      })
      setNotice(result.ok ? 'Provider test passed' : 'Provider test failed')
    } catch (error) {
      updateSelectedPlan({
        status: 'missing-key',
        lastTest: error instanceof Error ? error.message : 'test failed',
      })
      setNotice('Provider test failed')
    } finally {
      setIsTesting(false)
    }
  }

  return (
    <div className="app-shell" data-theme={theme}>
      <aside className="rail">
        <div className="brand">
          <div className="brand-mark">
            <Network size={20} />
          </div>
          <div>
            <strong>Models Assemble</strong>
            <span>Provider Console</span>
          </div>
        </div>

        <div className="search-box">
          <Search size={16} />
          <input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search plans, models"
          />
        </div>

        <div className="rail-actions">
          <button className="primary-action" onClick={addCustomPlan}>
            <Plus size={16} />
            Add coding plan
          </button>
        </div>

        <div className="plan-list">
          {filteredPlans.map((plan) => (
            <button
              key={plan.id}
              className={`plan-row ${plan.id === selectedPlan.id ? 'active' : ''}`}
              onClick={() => setSelectedId(plan.id)}
            >
              <div className={`status-dot ${plan.status}`} />
              <div className="plan-row-copy">
                <strong>{plan.name}</strong>
                <span>{plan.providerId}</span>
              </div>
              <ChevronRight size={16} />
            </button>
          ))}
        </div>

        <div className="gateway-card">
          <div className="gateway-icon">
            <Server size={17} />
          </div>
          <div>
            <strong>Gateway</strong>
            <span>127.0.0.1:8787</span>
          </div>
          <span className="live-pill">Ready</span>
        </div>
      </aside>

      <main className="workbench">
        <header className="topbar">
          <div>
            <p className="eyebrow">Coding plan</p>
            <h1>{selectedPlan.name}</h1>
          </div>
          <div className="topbar-actions">
            <button
              className="ghost-button"
              onClick={() => testSelectedPlan(false)}
              disabled={isTesting}
            >
              <Activity size={16} />
              Test provider
            </button>
            <button className="ghost-button" onClick={savePlans}>
              <Save size={16} />
              Save
            </button>
            <button
              className="icon-button"
              onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
              aria-label="Toggle color theme"
            >
              {theme === 'dark' ? <Sun size={17} /> : <Moon size={17} />}
            </button>
          </div>
        </header>

        <section className="summary-strip">
          <Metric label="Protocol" value={prettyProtocol(selectedPlan.protocol)} />
          <Metric label="Models" value={`${selectedPlan.models.length}`} />
          <Metric label="Template" value={selectedPlan.template} />
          <Metric label="Last test" value={selectedPlan.lastTest} />
        </section>

        <div className="notice-bar">
          <span>{notice}</span>
        </div>

        <div className="main-grid">
          <section className="panel editor-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Plan details</p>
                <h2>Connection</h2>
              </div>
              <button className="danger-button" onClick={deleteSelectedPlan}>
                <Trash2 size={15} />
                Delete
              </button>
            </div>

            <div className="form-grid">
              <Field label="Plan name">
                <input
                  value={selectedPlan.name}
                  onChange={(event) =>
                    updateSelectedPlan({ name: event.target.value })
                  }
                />
              </Field>
              <Field label="Provider ID">
                <input
                  value={selectedPlan.providerId}
                  onChange={(event) =>
                    updateSelectedPlan({ providerId: event.target.value })
                  }
                />
              </Field>
              <Field label="API format">
                <select
                  value={selectedPlan.protocol}
                  onChange={(event) =>
                    updateSelectedPlan({
                      protocol: event.target.value as ProtocolType,
                      authMode:
                        event.target.value === 'anthropic_compatible'
                          ? 'x-api-key'
                          : 'bearer',
                    })
                  }
                >
                  <option value="anthropic_compatible">
                    Anthropic Messages
                  </option>
                  <option value="openai_compatible">
                    OpenAI Chat Completions
                  </option>
                </select>
              </Field>
              <Field label="Auth mode">
                <select
                  value={selectedPlan.authMode}
                  onChange={(event) =>
                    updateSelectedPlan({
                      authMode: event.target.value as Plan['authMode'],
                    })
                  }
                >
                  <option value="x-api-key">x-api-key</option>
                  <option value="bearer">Bearer token</option>
                </select>
              </Field>
            </div>

            <Field label="Request base URL">
              <div className="url-input">
                <DatabaseZap size={16} />
                <input
                  value={selectedPlan.baseUrl}
                  onChange={(event) =>
                    updateSelectedPlan({ baseUrl: event.target.value })
                  }
                />
              </div>
            </Field>
            <p className="endpoint-note">
              Final endpoint:{' '}
              <code>
                {selectedPlan.baseUrl.replace(/\/$/, '')}
                {selectedPlan.protocol === 'anthropic_compatible'
                  ? '/messages'
                  : '/chat/completions'}
              </code>
            </p>

            <div className="form-grid">
              <Field label="API key env">
                <input
                  value={selectedPlan.apiKeyEnv}
                  onChange={(event) =>
                    updateSelectedPlan({ apiKeyEnv: event.target.value })
                  }
                />
              </Field>
              <Field label="API key preview">
                <div className="secret-input">
                  <input
                    type={showSecret ? 'text' : 'password'}
                    value={selectedPlan.apiKeyPreview}
                    onChange={(event) =>
                      updateSelectedPlan({ apiKeyPreview: event.target.value })
                    }
                    placeholder="Optional; prefer env vars"
                  />
                  <button
                    onClick={() => setShowSecret((value) => !value)}
                    aria-label="Toggle key visibility"
                  >
                    {showSecret ? <EyeOff size={15} /> : <Eye size={15} />}
                  </button>
                </div>
              </Field>
            </div>

            <div className="split-section">
              <Field label="Model list">
                <textarea
                  className="models-textarea"
                  value={selectedPlan.models.join('\n')}
                  onChange={(event) => updateModels(event.target.value)}
                />
              </Field>
              <div className="model-map">
                <ModelSelect
                  label="Main model"
                  value={selectedPlan.mainModel}
                  models={selectedPlan.models}
                  onChange={(mainModel) => updateSelectedPlan({ mainModel })}
                />
                <ModelSelect
                  label="Fast / Haiku"
                  value={selectedPlan.fastModel}
                  models={selectedPlan.models}
                  onChange={(fastModel) => updateSelectedPlan({ fastModel })}
                />
                <ModelSelect
                  label="Max / Opus"
                  value={selectedPlan.maxModel}
                  models={selectedPlan.models}
                  onChange={(maxModel) => updateSelectedPlan({ maxModel })}
                />
                <ModelSelect
                  label="Subagent"
                  value={selectedPlan.subagentModel}
                  models={selectedPlan.models}
                  onChange={(subagentModel) =>
                    updateSelectedPlan({ subagentModel })
                  }
                />
              </div>
            </div>

            <Field label="Request overrides JSON">
              <textarea
                className="code-textarea"
                value={selectedPlan.requestOverrides}
                onChange={(event) =>
                  updateSelectedPlan({ requestOverrides: event.target.value })
                }
              />
            </Field>
          </section>

          <aside className="right-stack">
            <section className="panel test-panel">
              <div className="section-heading">
                <div>
                  <p className="eyebrow">Validation</p>
                  <h2>Provider tests</h2>
                </div>
                <Zap size={18} />
              </div>
              <div className="test-row">
                <TerminalSquare size={17} />
                <div>
                  <strong>Non-stream test</strong>
                  <span>ma test-provider {selectedPlan.id}</span>
                </div>
                <button
                  className="mini-button"
                  onClick={() => testSelectedPlan(false)}
                  disabled={isTesting}
                >
                  Run
                </button>
              </div>
              <div className="test-row">
                <Activity size={17} />
                <div>
                  <strong>Stream test</strong>
                  <span>ma test-provider {selectedPlan.id} --stream</span>
                </div>
                <button
                  className="mini-button"
                  onClick={() => testSelectedPlan(true)}
                  disabled={isTesting}
                >
                  Run
                </button>
              </div>
            </section>

            <section className="panel preview-panel">
              <div className="section-heading">
                <div>
                  <p className="eyebrow">Generated</p>
                  <h2>Gateway config</h2>
                </div>
                <button
                  className="icon-button"
                  aria-label="Copy config"
                  onClick={() => copyText(generatedConfig, 'Gateway config')}
                >
                  <Copy size={16} />
                </button>
              </div>
              <pre>{generatedConfig}</pre>
            </section>

            <section className="panel preview-panel">
              <div className="section-heading">
                <div>
                  <p className="eyebrow">Claude Code</p>
                  <h2>Environment</h2>
                </div>
                <button
                  className="icon-button"
                  aria-label="Copy Claude Code environment"
                  onClick={() => copyText(claudeEnv, 'Claude Code env')}
                >
                  <Braces size={18} />
                </button>
              </div>
              <pre>{claudeEnv}</pre>
            </section>
          </aside>
        </div>
      </main>
    </div>
  )
}

function readPlans() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return templates
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) && parsed.length > 0
      ? parsed.map(clearPlanSecret)
      : templates
  } catch {
    return templates
  }
}

function stripSecrets(plans: Plan[]) {
  return plans.map(clearPlanSecret)
}

function clearPlanSecret(plan: Plan): Plan {
  return { ...plan, apiKeyPreview: '' }
}

function readTheme(): Theme {
  const stored = localStorage.getItem(THEME_KEY)
  if (stored === 'dark' || stored === 'light') return stored
  return window.matchMedia?.('(prefers-color-scheme: light)').matches
    ? 'light'
    : 'dark'
}

async function invokeOrMock<T>(
  command: string,
  args: Record<string, unknown>,
): Promise<T> {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke<T>(command, args)
  }

  await new Promise((resolve) => window.setTimeout(resolve, 420))
  return {
    ok: true,
    status: 'browser-preview',
    text_preview: 'Tauri command bridge is not active in browser dev mode',
  } as T
}

function Field({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  )
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  )
}

function ModelSelect({
  label,
  value,
  models,
  onChange,
}: {
  label: string
  value: string
  models: string[]
  onChange: (value: string) => void
}) {
  return (
    <Field label={label}>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {models.map((model) => (
          <option key={model} value={model}>
            {model}
          </option>
        ))}
      </select>
    </Field>
  )
}

function prettyProtocol(protocol: ProtocolType) {
  return protocol === 'anthropic_compatible'
    ? 'Anthropic Messages'
    : 'OpenAI Chat'
}

function buildConfigPreview(plan: Plan) {
  const providerType =
    plan.protocol === 'anthropic_compatible'
      ? 'anthropic_compatible'
      : 'openai_compatible'
  return `models:
  ${plan.id}:
    provider: ${plan.providerId}
    model: ${plan.mainModel}
    request_overrides: ${indentInline(plan.requestOverrides)}

providers:
  ${plan.providerId}:
    type: ${providerType}
    base_url: ${plan.baseUrl}
    api_key_env: ${plan.apiKeyEnv}
`
}

function buildClaudeEnv(plan: Plan) {
  return JSON.stringify(
    {
      env: {
        ANTHROPIC_BASE_URL: 'http://127.0.0.1:8787',
        ANTHROPIC_AUTH_TOKEN: 'ma-local-dev-key',
        ANTHROPIC_MODEL: plan.id,
        ANTHROPIC_DEFAULT_HAIKU_MODEL: plan.fastModel,
        ANTHROPIC_DEFAULT_SONNET_MODEL: plan.mainModel,
        ANTHROPIC_DEFAULT_OPUS_MODEL: plan.maxModel,
        CLAUDE_CODE_SUBAGENT_MODEL: plan.subagentModel,
      },
      theme: 'dark',
      includeCoAuthoredBy: false,
    },
    null,
    2,
  )
}

function indentInline(value: string) {
  const trimmed = value.trim()
  if (!trimmed) return '{}'
  return `|\n${trimmed
    .split('\n')
    .map((line) => `      ${line}`)
    .join('\n')}`
}

export default App
