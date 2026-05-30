import {
  invoke,
} from '@tauri-apps/api/core'
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
  PanelLeft,
  PanelLeftClose,
  Plus,
  Route,
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

type GatewayStatus = {
  running: boolean
  bind: string
  last_error?: string | null
}

type ModelSelection = {
  planId: string
  model: string
}

type AssembleProfile = {
  main: ModelSelection
  fast: ModelSelection
  max: ModelSelection
  subagent: ModelSelection
}

const STORAGE_KEY = 'models-assemble.desktop.plans.v1'
const PROFILE_KEY = 'models-assemble.desktop.profile.v1'
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
  const [profile, setProfile] = useState<AssembleProfile>(() =>
    readProfile(readPlans()),
  )
  const [selectedId, setSelectedId] = useState('home')
  const [search, setSearch] = useState('')
  const [showSecret, setShowSecret] = useState(false)
  const [notice, setNotice] = useState('Ready')
  const [isTesting, setIsTesting] = useState(false)
  const [gatewayStatus, setGatewayStatus] = useState<GatewayStatus>({
    running: false,
    bind: '127.0.0.1:8787',
  })
  const [isGatewayLoading, setIsGatewayLoading] = useState(false)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    const stored = localStorage.getItem('models-assemble.desktop.sidebar.v1')
    return stored === 'true'
  })
  const [isSmallScreen, setIsSmallScreen] = useState(false)

  const selectedPlan = plans.find((plan) => plan.id === selectedId) ?? plans[0]
  const isHome = selectedId === 'home'
  const filteredPlans = plans.filter((plan) =>
    `${plan.name} ${plan.providerId} ${plan.models.join(' ')}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  )

  const effectiveCollapsed = isSmallScreen ? true : sidebarCollapsed

  const generatedConfig = useMemo(
    () => buildConfigPreview(plans, profile),
    [plans, profile],
  )
  const claudeEnv = useMemo(() => buildClaudeEnv(), [])

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(stripSecrets(plans)))
  }, [plans])

  useEffect(() => {
    setProfile((current) => normalizeProfile(current, plans))
  }, [plans])

  useEffect(() => {
    localStorage.setItem(PROFILE_KEY, JSON.stringify(profile))
  }, [profile])

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
    invokeOrMock<AssembleProfile | null>('load_profile', {})
      .then((loadedProfile) => {
        if (loadedProfile) setProfile(loadedProfile)
      })
      .catch(() => {
        // Browser preview mode keeps using localStorage.
      })
  }, [])

  useEffect(() => {
    localStorage.setItem(THEME_KEY, theme)
  }, [theme])

  useEffect(() => {
    refreshGatewayStatus()
    const interval = window.setInterval(refreshGatewayStatus, 5000)
    return () => window.clearInterval(interval)
  }, [])

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme)
  }, [theme])

  useEffect(() => {
    const mql = window.matchMedia('(max-width: 768px)')
    const handler = (e: MediaQueryListEvent | MediaQueryList) => setIsSmallScreen(e.matches)
    handler(mql)
    mql.addEventListener('change', handler)
    return () => mql.removeEventListener('change', handler)
  }, [])

  useEffect(() => {
    localStorage.setItem('models-assemble.desktop.sidebar.v1', String(sidebarCollapsed))
  }, [sidebarCollapsed])

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
      await invokeOrMock('save_plans', {
        plans,
        profile: normalizeProfile(profile, plans),
      })
      setNotice(
        gatewayStatus.running
          ? 'Saved; restart Gateway to apply provider keys'
          : 'Saved',
      )
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

  async function refreshGatewayStatus() {
    try {
      const status = await invokeOrMock<GatewayStatus>('get_gateway_status', {})
      setGatewayStatus(status)
    } catch {
      setGatewayStatus((prev) => prev)
    }
  }

  async function toggleGateway() {
    setIsGatewayLoading(true)
    try {
      if (gatewayStatus.running) {
        const status = await invokeOrMock<GatewayStatus>('stop_gateway', {})
        setGatewayStatus(status)
        setNotice('Gateway stopped')
      } else {
        await invokeOrMock('save_plans', {
          plans,
          profile: normalizeProfile(profile, plans),
        })
        const status = await invokeOrMock<GatewayStatus>('start_gateway', {
          configPath: '',
        })
        setGatewayStatus(status)
        setNotice('Gateway started')
      }
    } catch (error) {
      setNotice(
        error instanceof Error ? error.message : 'Gateway operation failed',
      )
      refreshGatewayStatus()
    } finally {
      setIsGatewayLoading(false)
    }
  }

  return (
    <div className={`app-shell ${effectiveCollapsed ? 'sidebar-collapsed' : ''}`} data-theme={theme}>
      <aside className="rail">
        <div className="brand">
          <div className="brand-mark">
            <Network size={20} />
          </div>
          <div className="brand-text">
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
            <span>Add coding plan</span>
          </button>
        </div>

        <div className="plan-list">
          <button
            className={`plan-row home-row ${isHome ? 'active' : ''}`}
            onClick={() => setSelectedId('home')}
          >
            <div className="home-mark">
              <Route size={15} />
            </div>
            <div className="plan-row-copy">
              <strong>Assemble Home</strong>
              <span>Claude Code local provider</span>
            </div>
            <ChevronRight size={16} />
          </button>

          {filteredPlans.map((plan, index) => (
            <button
              key={plan.id}
              className={`plan-row ${plan.id === selectedPlan.id ? 'active' : ''}`}
              onClick={() => setSelectedId(plan.id)}
              style={{ ['--i' as any]: index }}
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
          <div className="gateway-text">
            <strong>Gateway</strong>
            <span>{gatewayStatus.last_error || gatewayStatus.bind}</span>
          </div>
          <button
            className={`live-pill ${gatewayStatus.running ? 'running' : 'stopped'}`}
            onClick={toggleGateway}
            disabled={isGatewayLoading}
          >
            {isGatewayLoading
              ? '...'
              : gatewayStatus.running
                ? 'Running'
                : 'Stopped'}
          </button>
        </div>

        <button
          className="collapse-btn"
          onClick={() => setSidebarCollapsed((v) => !v)}
          aria-label={effectiveCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          title={effectiveCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          {effectiveCollapsed ? <PanelLeft size={18} /> : <PanelLeftClose size={18} />}
          <span>Collapse</span>
        </button>
      </aside>

      <main className="workbench">
        <div className="workbench-content" key={selectedId}>
        <header className="topbar">
          <div>
            <p className="eyebrow">{isHome ? 'Local provider' : 'Provider'}</p>
            <h1>{isHome ? 'Assemble Profile' : selectedPlan.name}</h1>
          </div>
          <div className="topbar-actions">
            {!isHome && (
              <button
                className="ghost-button"
                onClick={() => testSelectedPlan(false)}
                disabled={isTesting}
              >
                <Activity size={16} />
                Test provider
              </button>
            )}
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

        {isHome ? (
          <>
            <section className="summary-strip">
              <Metric label="Main" value={selectionLabel(profile.main, plans)} />
              <Metric label="Fast" value={selectionLabel(profile.fast, plans)} />
              <Metric label="Max" value={selectionLabel(profile.max, plans)} />
              <Metric
                label="Subagent"
                value={selectionLabel(profile.subagent, plans)}
              />
            </section>

            <section className="panel profile-panel">
              <div className="section-heading">
                <div>
                  <p className="eyebrow">Assemble profile</p>
                  <h2>Claude Code sees one local provider</h2>
                </div>
                <Route size={18} />
              </div>
              <div className="profile-grid">
                <ProfileSlot
                  label="Main / Sonnet"
                  value={profile.main}
                  plans={plans}
                  onChange={(main) =>
                    setProfile((current) => ({ ...current, main }))
                  }
                />
                <ProfileSlot
                  label="Fast / Haiku"
                  value={profile.fast}
                  plans={plans}
                  onChange={(fast) =>
                    setProfile((current) => ({ ...current, fast }))
                  }
                />
                <ProfileSlot
                  label="Max / Opus"
                  value={profile.max}
                  plans={plans}
                  onChange={(max) =>
                    setProfile((current) => ({ ...current, max }))
                  }
                />
                <ProfileSlot
                  label="Subagent"
                  value={profile.subagent}
                  plans={plans}
                  onChange={(subagent) =>
                    setProfile((current) => ({ ...current, subagent }))
                  }
                />
              </div>
            </section>
          </>
        ) : (
          <section className="summary-strip provider-strip">
            <Metric label="Editing Provider" value={selectedPlan.name} />
            <Metric label="Protocol" value={prettyProtocol(selectedPlan.protocol)} />
            <Metric label="Provider Models" value={`${selectedPlan.models.length}`} />
            <Metric label="Last test" value={selectedPlan.lastTest} />
          </section>
        )}

        <div className="notice-bar" key={notice}>
          <span>{notice}</span>
        </div>

        {isHome && (
          <div className="main-grid home-grid">
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
          </div>
        )}

        {!isHome && (
        <div className="main-grid">
          <section className="panel editor-panel">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Provider details</p>
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
              <Field label="API key">
                <div className="secret-input">
                  <input
                    type={showSecret ? 'text' : 'password'}
                    value={selectedPlan.apiKeyPreview || selectedPlan.apiKeyEnv}
                    onChange={(event) =>
                      updateSelectedPlan({
                        apiKeyPreview: event.target.value,
                        apiKeyEnv: defaultApiKeyEnv(selectedPlan),
                      })
                    }
                    placeholder="Stored in memory for this app session"
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
                  <span>{selectedPlan.providerId} / {selectedPlan.mainModel}</span>
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
                  <span>{selectedPlan.providerId} / {selectedPlan.mainModel}</span>
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
        )}
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

function readProfile(plans: Plan[]): AssembleProfile {
  try {
    const raw = localStorage.getItem(PROFILE_KEY)
    if (raw) return normalizeProfile(JSON.parse(raw), plans)
  } catch {
    // Fall through to template defaults.
  }
  return defaultProfile(plans)
}

function defaultProfile(plans: Plan[]): AssembleProfile {
  const glm = plans.find((plan) => plan.id === 'glm') ?? plans[0]
  const deepseek = plans.find((plan) => plan.id === 'deepseek') ?? plans[0]
  return {
    main: {
      planId: glm.id,
      model: glm.mainModel || glm.models[0] || '',
    },
    fast: {
      planId: deepseek.id,
      model: deepseek.fastModel || deepseek.models[0] || '',
    },
    max: {
      planId: glm.id,
      model: glm.maxModel || glm.models[0] || '',
    },
    subagent: {
      planId: deepseek.id,
      model: deepseek.subagentModel || deepseek.models[0] || '',
    },
  }
}

function normalizeProfile(profile: AssembleProfile, plans: Plan[]): AssembleProfile {
  const fallback = defaultProfile(plans)
  return {
    main: normalizeSelection(profile?.main, plans, fallback.main),
    fast: normalizeSelection(profile?.fast, plans, fallback.fast),
    max: normalizeSelection(profile?.max, plans, fallback.max),
    subagent: normalizeSelection(profile?.subagent, plans, fallback.subagent),
  }
}

function normalizeSelection(
  selection: ModelSelection | undefined,
  plans: Plan[],
  fallback: ModelSelection,
): ModelSelection {
  const plan = plans.find((item) => item.id === selection?.planId)
  if (!plan) return fallback
  const model = plan.models.includes(selection?.model ?? '')
    ? selection?.model ?? ''
    : plan.models[0] || ''
  return { planId: plan.id, model }
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
  if (isTauriRuntime()) {
    return invoke<T>(command, args)
  }

  await new Promise((resolve) => window.setTimeout(resolve, 420))
  if (command === 'get_gateway_status') {
    return {
      running: false,
      bind: '127.0.0.1:8787',
    } as T
  }
  if (command === 'start_gateway' || command === 'stop_gateway') {
    return {
      running: command === 'start_gateway',
      bind: '127.0.0.1:8787',
    } as T
  }
  if (command === 'load_plans') {
    return [] as T
  }
  if (command === 'load_profile') {
    return null as T
  }
  if (command === 'test_provider') {
    return {
      ok: false,
      status: 'browser-preview',
      text_preview: 'Tauri command bridge is not active in browser dev mode',
    } as T
  }
  return {
    ok: true,
    status: 'browser-preview',
    text_preview: 'Tauri command bridge is not active in browser dev mode',
  } as T
}

function isTauriRuntime() {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
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

function ProfileSlot({
  label,
  value,
  plans,
  onChange,
}: {
  label: string
  value: ModelSelection
  plans: Plan[]
  onChange: (value: ModelSelection) => void
}) {
  const plan = plans.find((item) => item.id === value.planId) ?? plans[0]
  const model = plan.models.includes(value.model)
    ? value.model
    : plan.models[0] || ''

  return (
    <div className="profile-slot">
      <span>{label}</span>
      <select
        value={plan.id}
        onChange={(event) => {
          const nextPlan = plans.find((item) => item.id === event.target.value)
          if (!nextPlan) return
          onChange({
            planId: nextPlan.id,
            model: nextPlan.models[0] || '',
          })
        }}
      >
        {plans.map((item) => (
          <option key={item.id} value={item.id}>
            {item.name}
          </option>
        ))}
      </select>
      <select
        value={model}
        onChange={(event) =>
          onChange({ planId: plan.id, model: event.target.value })
        }
      >
        {plan.models.map((item) => (
          <option key={item} value={item}>
            {item}
          </option>
        ))}
      </select>
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

function selectionLabel(selection: ModelSelection, plans: Plan[]) {
  const plan = plans.find((item) => item.id === selection.planId)
  if (!plan) return 'Unassigned'
  return `${plan.template}: ${selection.model || plan.models[0] || 'model'}`
}

function buildConfigPreview(plans: Plan[], profile: AssembleProfile) {
  const routes = [
    ['assemble-main', profile.main],
    ['assemble-fast', profile.fast],
    ['assemble-max', profile.max],
    ['assemble-subagent', profile.subagent],
  ]
    .map(([alias, selection]) => {
      const selected = selection as ModelSelection
      const plan = plans.find((item) => item.id === selected.planId) ?? plans[0]
      return `  ${alias}:
    provider: ${plan.providerId}
    model: ${selected.model || plan.models[0] || ''}`
    })
    .join('\n')

  const providers = plans
    .map((plan) => {
      const providerType =
        plan.protocol === 'anthropic_compatible'
          ? 'anthropic_compatible'
          : 'openai_compatible'
      return `  ${plan.providerId}:
    type: ${providerType}
    base_url: ${plan.baseUrl}
    api_key_env: ${defaultApiKeyEnv(plan)}`
    })
    .join('\n')

  return `server:
  bind: 127.0.0.1:8787
  api_keys:
    - ma-local-dev-key

models:
${routes}

providers:
${providers}

routing:
  default: assemble-main
`
}

function defaultApiKeyEnv(plan: Plan) {
  return `${plan.providerId
    .replace(/[^a-zA-Z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .toUpperCase()}_API_KEY`
}

function buildClaudeEnv() {
  return JSON.stringify(
    {
      env: {
        ANTHROPIC_BASE_URL: 'http://127.0.0.1:8787',
        ANTHROPIC_AUTH_TOKEN: 'ma-local-dev-key',
        ANTHROPIC_MODEL: 'assemble-main',
        ANTHROPIC_DEFAULT_HAIKU_MODEL: 'assemble-fast',
        ANTHROPIC_DEFAULT_SONNET_MODEL: 'assemble-main',
        ANTHROPIC_DEFAULT_OPUS_MODEL: 'assemble-max',
        CLAUDE_CODE_SUBAGENT_MODEL: 'assemble-subagent',
      },
      theme: 'dark',
      includeCoAuthoredBy: false,
    },
    null,
    2,
  )
}

export default App
