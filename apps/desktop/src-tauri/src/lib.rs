use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use ma_core::{
    AppConfig, ProviderCompliance, ProviderConfig, ProviderTestResult, ProviderType, ServerConfig,
    config::{ModelRoute, RoutingConfig},
    provider_test,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tokio::sync::oneshot;

#[derive(Debug, Serialize)]
struct GatewayStatus {
    running: bool,
    bind: String,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiPlan {
    id: String,
    name: String,
    provider_id: String,
    protocol: String,
    base_url: String,
    api_key_env: String,
    api_key_preview: String,
    models: Vec<String>,
    main_model: String,
    fast_model: String,
    max_model: String,
    subagent_model: String,
    request_overrides: String,
    status: String,
    template: String,
    last_test: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiProfile {
    main: UiModelSelection,
    fast: UiModelSelection,
    max: UiModelSelection,
    subagent: UiModelSelection,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiModelSelection {
    plan_id: String,
    model: String,
}

struct GatewayState {
    task: Mutex<Option<thread::JoinHandle<()>>>,
    shutdown: Mutex<Option<oneshot::Sender<()>>>,
    bind: Mutex<String>,
    last_error: Arc<Mutex<Option<String>>>,
    runtime_keys: Mutex<BTreeMap<String, String>>,
}

#[tauri::command]
fn get_gateway_status(state: tauri::State<GatewayState>) -> GatewayStatus {
    clear_finished_gateway(&state);

    let task = state.task.lock().unwrap();
    let bind = state.bind.lock().unwrap();
    let running = task.is_some() && is_gateway_listening(&bind);
    GatewayStatus {
        running,
        bind: bind.clone(),
        last_error: state.last_error.lock().unwrap().clone(),
    }
}

#[tauri::command]
async fn start_gateway(
    app: tauri::AppHandle,
    state: tauri::State<'_, GatewayState>,
    config_path: String,
) -> Result<GatewayStatus, String> {
    clear_finished_gateway(&state);

    {
        let task = state.task.lock().unwrap();
        if task.is_some() {
            let bind = state.bind.lock().unwrap();
            return Ok(GatewayStatus {
                running: true,
                bind: bind.clone(),
                last_error: state.last_error.lock().unwrap().clone(),
            });
        }
    }

    let config_path = resolve_gateway_config_path(&app, config_path)?;
    if !config_path.exists() {
        let bind = state.bind.lock().unwrap();
        let message = format!(
            "gateway config not found at {}. Save plans before starting gateway on {bind}.",
            config_path.display()
        );
        *state.last_error.lock().unwrap() = Some(message.clone());
        return Err(message);
    }

    let config = AppConfig::load(&config_path).map_err(|error| {
        *state.last_error.lock().unwrap() = Some(error.safe_message.clone());
        error.safe_message
    })?;
    if let Err(errors) = config.validate() {
        let messages = errors
            .into_iter()
            .map(|error| error.safe_message)
            .collect::<Vec<_>>()
            .join("; ");
        let message = format!("gateway config validation failed: {messages}");
        *state.last_error.lock().unwrap() = Some(message.clone());
        return Err(message);
    }

    let bind_addr = config.server.bind;
    apply_runtime_keys(&state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
    let last_error = Arc::clone(&state.last_error);

    let task = thread::Builder::new()
        .name("models-assemble-gateway".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("models-assemble-gateway-worker")
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let message = format!("failed to create gateway runtime: {error}");
                    *last_error.lock().unwrap() = Some(message.clone());
                    let _ = startup_tx.send(Err(message));
                    return;
                }
            };

            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::bind(bind_addr).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        let message = format!("failed to bind gateway on {bind_addr}: {error}");
                        *last_error.lock().unwrap() = Some(message.clone());
                        let _ = startup_tx.send(Err(message));
                        return;
                    }
                };

                let app_router = ma_server::router(config);
                let _ = startup_tx.send(Ok(()));
                let result = axum::serve(listener, app_router)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
                let message = match result {
                    Ok(()) => "gateway stopped normally; shutdown signal was received".to_string(),
                    Err(error) => format!("gateway stopped with error: {error}"),
                };
                *last_error.lock().unwrap() = Some(message.clone());
                eprintln!("Models Assemble {message}");
            });
        })
        .map_err(|error| {
            let message = format!("failed to spawn gateway thread: {error}");
            *state.last_error.lock().unwrap() = Some(message.clone());
            message
        })?;

    match startup_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(message)) => {
            let _ = task.join();
            return Err(message);
        }
        Err(error) => {
            let message =
                format!("gateway startup timed out before listening on {bind_addr}: {error}");
            *state.last_error.lock().unwrap() = Some(message.clone());
            let _ = shutdown_tx.send(());
            let _ = task.join();
            return Err(message);
        }
    }

    *state.task.lock().unwrap() = Some(task);
    *state.shutdown.lock().unwrap() = Some(shutdown_tx);
    *state.bind.lock().unwrap() = bind_addr.to_string();

    if let Err(message) = wait_for_gateway(bind_addr).await {
        let _ = state.shutdown.lock().unwrap().take().map(|tx| tx.send(()));
        if let Some(task) = state.task.lock().unwrap().take() {
            let _ = task.join();
        }
        *state.last_error.lock().unwrap() = Some(message.clone());
        return Err(message);
    }
    *state.last_error.lock().unwrap() = None;

    let bind = state.bind.lock().unwrap();
    Ok(GatewayStatus {
        running: true,
        bind: bind.clone(),
        last_error: None,
    })
}

#[tauri::command]
fn stop_gateway(state: tauri::State<GatewayState>) -> Result<GatewayStatus, String> {
    if let Some(shutdown) = state.shutdown.lock().unwrap().take() {
        let _ = shutdown.send(());
    }
    if let Some(task) = state.task.lock().unwrap().take() {
        let _ = task.join();
    }

    let bind = state.bind.lock().unwrap();
    Ok(GatewayStatus {
        running: false,
        bind: bind.clone(),
        last_error: state.last_error.lock().unwrap().clone(),
    })
}

#[tauri::command]
async fn test_provider(plan: UiPlan, stream: bool) -> Result<ProviderTestResult, String> {
    let mut provider = plan.to_provider_config()?;
    provider.api_key_env = None;
    let route = plan.to_model_route()?;
    let api_key_override = plan.api_key_preview.trim();
    provider_test::test_provider_route(
        &provider,
        &route,
        stream,
        (!api_key_override.is_empty()).then_some(api_key_override),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_plans(
    app: tauri::AppHandle,
    state: tauri::State<GatewayState>,
    plans: Vec<UiPlan>,
    profile: Option<UiProfile>,
) -> Result<ProviderTestResult, String> {
    let count = plans.len();
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|error| error.to_string())?;

    let sanitized_plans = plans.iter().map(UiPlan::without_secret).collect::<Vec<_>>();
    let plans_path = config_dir.join("plans.json");
    let plans_json =
        serde_json::to_string_pretty(&sanitized_plans).map_err(|error| error.to_string())?;
    fs::write(&plans_path, plans_json).map_err(|error| error.to_string())?;

    if let Some(profile) = &profile {
        let profile_path = config_dir.join("profile.json");
        let profile_json =
            serde_json::to_string_pretty(profile).map_err(|error| error.to_string())?;
        fs::write(&profile_path, profile_json).map_err(|error| error.to_string())?;
    }

    let gateway_config = build_app_config(&plans, profile.as_ref())?;
    let config_yaml = serde_yaml::to_string(&gateway_config).map_err(|error| error.to_string())?;
    let config_path = config_dir.join("config.yaml");
    fs::write(&config_path, config_yaml).map_err(|error| error.to_string())?;
    store_runtime_keys(&state, &plans);

    Ok(ProviderTestResult {
        ok: true,
        status: "saved".to_string(),
        text_preview: format!("{count} plan(s) saved to {}", config_dir.display()),
    })
}

#[tauri::command]
fn load_plans(app: tauri::AppHandle) -> Result<Vec<UiPlan>, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let plans_path = config_dir.join("plans.json");

    match fs::read_to_string(&plans_path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|error| error.to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
fn load_profile(app: tauri::AppHandle) -> Result<Option<UiProfile>, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let profile_path = config_dir.join("profile.json");

    match fs::read_to_string(&profile_path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn resolve_gateway_config_path(
    app: &tauri::AppHandle,
    config_path: String,
) -> Result<PathBuf, String> {
    let trimmed = config_path.trim();
    if !trimmed.is_empty() {
        return Ok(PathBuf::from(trimmed));
    }

    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("config.yaml"))
}

impl UiPlan {
    fn to_provider_config(&self) -> Result<ProviderConfig, String> {
        Ok(ProviderConfig {
            provider_type: parse_provider_type(&self.protocol)?,
            base_url: Some(self.base_url.trim().to_string()).filter(|value| !value.is_empty()),
            api_key_env: Some(provider_env_name(self)).filter(|value| !value.is_empty()),
            compliance: ProviderCompliance::OfficialCodingEndpoint,
        })
    }

    fn to_model_route(&self) -> Result<ModelRoute, String> {
        let model = fallback_model(&self.main_model, &self.models)?;

        Ok(ModelRoute {
            provider: self.provider_id.clone(),
            model,
        })
    }

    fn without_secret(&self) -> Self {
        Self {
            api_key_preview: String::new(),
            ..self.clone()
        }
    }
}

fn build_app_config(plans: &[UiPlan], profile: Option<&UiProfile>) -> Result<AppConfig, String> {
    let mut models = BTreeMap::new();
    let mut providers = BTreeMap::new();

    for plan in plans {
        models.insert(plan.id.clone(), plan.to_model_route()?);
        models.insert(
            format!("{}-fast", plan.id),
            ModelRoute {
                provider: plan.provider_id.clone(),
                model: fallback_model(&plan.fast_model, &plan.models)?,
            },
        );
        models.insert(
            format!("{}-max", plan.id),
            ModelRoute {
                provider: plan.provider_id.clone(),
                model: fallback_model(&plan.max_model, &plan.models)?,
            },
        );
        models.insert(
            format!("{}-subagent", plan.id),
            ModelRoute {
                provider: plan.provider_id.clone(),
                model: fallback_model(&plan.subagent_model, &plan.models)?,
            },
        );
        providers.insert(plan.provider_id.clone(), plan.to_provider_config()?);
    }

    if let Some(profile) = profile {
        insert_profile_route(&mut models, plans, "assemble-main", &profile.main)?;
        insert_profile_route(&mut models, plans, "assemble-fast", &profile.fast)?;
        insert_profile_route(&mut models, plans, "assemble-max", &profile.max)?;
        insert_profile_route(&mut models, plans, "assemble-subagent", &profile.subagent)?;
    }

    let default = if models.contains_key("assemble-main") {
        "assemble-main".to_string()
    } else {
        plans
            .first()
            .map(|plan| plan.id.clone())
            .unwrap_or_else(|| "assemble-mock".to_string())
    };

    Ok(AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:8787"
                .parse::<SocketAddr>()
                .expect("default desktop bind address is valid"),
            api_keys: vec!["ma-local-dev-key".to_string()],
            first_token_timeout_secs: None,
        },
        models,
        providers,
        routing: RoutingConfig { default },
        fallback: BTreeMap::new(),
    })
}

fn insert_profile_route(
    models: &mut BTreeMap<String, ModelRoute>,
    plans: &[UiPlan],
    alias: &str,
    selection: &UiModelSelection,
) -> Result<(), String> {
    let plan = plans
        .iter()
        .find(|plan| plan.id == selection.plan_id)
        .ok_or_else(|| format!("profile alias `{alias}` references unknown plan"))?;
    let model = if selection.model.trim().is_empty() {
        fallback_model(&plan.main_model, &plan.models)?
    } else {
        selection.model.trim().to_string()
    };

    models.insert(
        alias.to_string(),
        ModelRoute {
            provider: plan.provider_id.clone(),
            model,
        },
    );
    Ok(())
}

fn fallback_model(value: &str, models: &[String]) -> Result<String, String> {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }
    models
        .first()
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "plan must include at least one model".to_string())
}

fn parse_provider_type(protocol: &str) -> Result<ProviderType, String> {
    match protocol {
        "anthropic_compatible" => Ok(ProviderType::AnthropicCompatible),
        "openai_compatible" => Ok(ProviderType::OpenAiCompatible),
        other => Err(format!("unsupported provider protocol `{other}`")),
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(GatewayState {
            task: Mutex::new(None),
            shutdown: Mutex::new(None),
            bind: Mutex::new("127.0.0.1:8787".to_string()),
            last_error: Arc::new(Mutex::new(None)),
            runtime_keys: Mutex::new(BTreeMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_gateway_status,
            start_gateway,
            stop_gateway,
            test_provider,
            save_plans,
            load_plans,
            load_profile
        ])
        .run(tauri::generate_context!())
        .expect("error while running Models Assemble desktop");
}

fn store_runtime_keys(state: &GatewayState, plans: &[UiPlan]) {
    let mut keys = BTreeMap::new();
    for plan in plans {
        let env_name = provider_env_name(plan);
        let key = provider_key_value(plan);
        if !env_name.is_empty() && !key.is_empty() {
            keys.insert(env_name, key);
        }
    }

    *state.runtime_keys.lock().unwrap() = keys;
}

fn apply_runtime_keys(state: &GatewayState) {
    for (env_name, key) in state.runtime_keys.lock().unwrap().iter() {
        // The embedded gateway reads provider keys from env vars when it builds adapters.
        // We set these immediately before starting the in-process server.
        unsafe {
            std::env::set_var(env_name, key);
        }
    }
}

fn provider_env_name(plan: &UiPlan) -> String {
    let value = plan.api_key_env.trim();
    if !value.is_empty() && !looks_like_api_key(value) {
        return value.to_string();
    }

    format!(
        "{}_API_KEY",
        plan.provider_id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

fn provider_key_value(plan: &UiPlan) -> String {
    let explicit = plan.api_key_preview.trim();
    if !explicit.is_empty() {
        return explicit.to_string();
    }

    let env_or_key = plan.api_key_env.trim();
    if looks_like_api_key(env_or_key) {
        env_or_key.to_string()
    } else {
        String::new()
    }
}

fn looks_like_api_key(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("sk_")
        || value.contains('.')
        || value.len() >= 32 && value.chars().any(|ch| ch.is_ascii_lowercase())
}

fn clear_finished_gateway(state: &GatewayState) {
    let finished = state
        .task
        .lock()
        .unwrap()
        .as_ref()
        .map(|task| task.is_finished())
        .unwrap_or(false);

    if finished {
        let _ = state.shutdown.lock().unwrap().take();
        if let Some(task) = state.task.lock().unwrap().take() {
            let _ = task.join();
        }
    }
}

async fn wait_for_gateway(bind: SocketAddr) -> Result<(), String> {
    for _ in 0..20 {
        if tokio::net::TcpStream::connect(bind).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(format!(
        "gateway started but health check failed; 127.0.0.1:{port} may be blocked or occupied",
        port = bind.port()
    ))
}

fn is_gateway_listening(bind: &str) -> bool {
    bind.parse::<SocketAddr>()
        .ok()
        .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(120)).ok())
        .is_some()
}
