use std::pin::Pin;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use ma_core::normalized::{NormalizedEvent, NormalizedRequest};
use ma_core::{ErrorCategory, NormalizedError};

use crate::AppState;

/// First-chunk timeout fallback for streaming requests.
///
/// Tries each adapter in the fallback chain sequentially. If the first SSE
/// chunk does not arrive within `server.first_token_timeout_secs` (default 15s),
/// cancels the current stream and tries the next adapter.
///
/// Fallback only applies *before* any data is sent to the client. Once the
/// first chunk is received, the stream continues without further fallback.
pub async fn stream_with_fallback(
    state: &AppState,
    request: NormalizedRequest,
    model_alias: &str,
) -> Result<Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>, NormalizedError>
{
    let timeout = state
        .config
        .server
        .first_token_timeout_secs
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(15));

    let chain = build_fallback_chain(&state.config.fallback, model_alias);
    let mut last_error = None;

    for alias in &chain {
        match try_first_chunk(state, &request, alias, timeout).await {
            FirstChunk::GotFirst {
                stream,
                first_event,
            } => {
                tracing::info!(alias, "first chunk received, streaming from adapter");
                return Ok(wrap_with_first(stream, first_event));
            }
            FirstChunk::Timeout => {
                tracing::warn!(alias, ?timeout, "first chunk timeout, trying fallback");
                last_error = Some(timeout_error(timeout));
            }
            FirstChunk::StreamError(e) | FirstChunk::AdapterError(e) => {
                tracing::warn!(alias, error = %e.safe_message, "trying fallback");
                last_error = Some(e);
            }
            FirstChunk::EmptyStream => {
                tracing::warn!(alias, "empty stream, trying fallback");
                last_error = Some(fallback_error(
                    "upstream stream ended before producing any events",
                ));
            }
            FirstChunk::ConfigError(msg) => {
                tracing::warn!(alias, %msg, "config error, skipping");
                last_error = Some(fallback_error(&msg));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| fallback_error("all fallback adapters exhausted")))
}



enum FirstChunk {
    GotFirst {
        stream: Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>,
        first_event: NormalizedEvent,
    },
    Timeout,
    StreamError(NormalizedError),
    EmptyStream,
    ConfigError(String),
    AdapterError(NormalizedError),
}

async fn try_first_chunk(
    state: &AppState,
    request: &NormalizedRequest,
    alias: &str,
    timeout: Duration,
) -> FirstChunk {
    let route = match state.config.models.get(alias) {
        Some(r) => r,
        None => {
            return FirstChunk::ConfigError(format!(
                "fallback alias `{alias}` references unknown model"
            ));
        }
    };

    let adapter = match state.adapter_registry.get(&route.provider) {
        Some(a) => a.clone(),
        None => {
            return FirstChunk::ConfigError(format!(
                "fallback alias `{alias}` references unknown provider `{}`",
                route.provider
            ));
        }
    };

    let mut req = request.clone();
    req.model_alias = route.model.clone();

    let mut stream = match adapter.stream(req).await {
        Ok(s) => s,
        Err(e) => return FirstChunk::AdapterError(e),
    };

    match tokio::time::timeout(timeout, stream.next()).await {
        Ok(Some(Ok(event))) => FirstChunk::GotFirst {
            stream,
            first_event: event,
        },
        Ok(Some(Err(e))) => FirstChunk::StreamError(e),
        Ok(None) => FirstChunk::EmptyStream,
        Err(_) => FirstChunk::Timeout,
    }
}

fn wrap_with_first(
    mut stream: Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>,
    first_event: NormalizedEvent,
) -> Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>> {
    Box::pin(async_stream::stream! {
        yield Ok(first_event);
        while let Some(item) = stream.next().await {
            yield item;
        }
    })
}

fn build_fallback_chain(
    fallback_config: &std::collections::BTreeMap<String, Vec<String>>,
    model_alias: &str,
) -> Vec<String> {
    let mut chain = vec![model_alias.to_string()];
    if let Some(fallbacks) = fallback_config.get(model_alias) {
        chain.extend(fallbacks.iter().cloned());
    }
    chain
}

fn fallback_error(message: &str) -> NormalizedError {
    NormalizedError {
        category: ErrorCategory::Timeout,
        retryable: true,
        http_status: 504,
        provider_code: None,
        safe_message: message.to_string(),
        raw_debug: None,
    }
}

fn timeout_error(timeout: Duration) -> NormalizedError {
    fallback_error(&format!(
        "first chunk timeout after {}s",
        timeout.as_secs()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures_util::StreamExt;
    use ma_core::adapter::{NormalizedResponse, ProviderCapabilities, ProviderAdapter};
    use ma_core::config::ModelRoute;
    use ma_core::ServerConfig;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;

    


    enum Behavior {
        Pending,
        Fast,
        StreamErr,
        FirstErr,
    }

    struct Mock(Behavior);

    #[async_trait]
    impl ProviderAdapter for Mock {
        fn name(&self) -> &str {
            "mock"
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::default()
        }
        async fn complete(
            &self,
            _: NormalizedRequest,
        ) -> Result<NormalizedResponse, NormalizedError> {
            unimplemented!()
        }
        async fn stream(
            &self,
            _: NormalizedRequest,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>,
            NormalizedError,
        > {
            match &self.0 {
                Behavior::Pending => Ok(Box::pin(futures_util::stream::once(
                    std::future::pending(),
                ))),
                Behavior::Fast => Ok(Box::pin(futures_util::stream::once(async {
                    Ok(NormalizedEvent::MessageStop {
                        extra: HashMap::new(),
                    })
                }))),
                Behavior::StreamErr => Err(NormalizedError {
                    category: ErrorCategory::Overloaded,
                    retryable: true,
                    http_status: 503,
                    provider_code: None,
                    safe_message: "service unavailable".into(),
                    raw_debug: None,
                }),
                Behavior::FirstErr => Ok(Box::pin(futures_util::stream::once(async {
                    Err(NormalizedError {
                        category: ErrorCategory::Overloaded,
                        retryable: true,
                        http_status: 503,
                        provider_code: None,
                        safe_message: "first chunk error".into(),
                        raw_debug: None,
                    })
                }))),
            }
        }
    }

    


    fn make_state(
        models: BTreeMap<String, ModelRoute>,
        fallback: BTreeMap<String, Vec<String>>,
        adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
        timeout: Option<u64>,
    ) -> AppState {
        use ma_core::AppConfig;
        AppState {
            config: Arc::new(AppConfig {
                server: ServerConfig {
                    bind: "127.0.0.1:0".parse().unwrap(),
                    api_keys: vec![],
                    first_token_timeout_secs: timeout,
                },
                models,
                providers: BTreeMap::new(),
                routing: ma_core::config::RoutingConfig {
                    default: String::new(),
                },
                fallback,
            }),
            http: reqwest::Client::new(),
            adapter_registry: Arc::new(adapters),
        }
    }

    fn req() -> NormalizedRequest {
        NormalizedRequest::new("primary".into())
    }

    


    #[tokio::test]
    async fn fast_adapter_no_fallback() {
        let mut models = BTreeMap::new();
        models.insert(
            "primary".into(),
            ModelRoute {
                provider: "fast".into(),
                model: "m".into(),
            },
        );
        let mut adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        adapters.insert("fast".into(), Arc::new(Mock(Behavior::Fast)));

        let state = make_state(models, BTreeMap::new(), adapters, Some(1));
        let mut s = stream_with_fallback(&state, req(), "primary")
            .await
            .unwrap();
        assert!(matches!(
            s.next().await.unwrap().unwrap(),
            NormalizedEvent::MessageStop { .. }
        ));
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn fallback_on_timeout() {
        let mut models = BTreeMap::new();
        models.insert(
            "primary".into(),
            ModelRoute {
                provider: "slow".into(),
                model: "m".into(),
            },
        );
        models.insert(
            "backup".into(),
            ModelRoute {
                provider: "fast".into(),
                model: "m".into(),
            },
        );
        let mut fb = BTreeMap::new();
        fb.insert("primary".into(), vec!["backup".into()]);
        let mut adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        adapters.insert("slow".into(), Arc::new(Mock(Behavior::Pending)));
        adapters.insert("fast".into(), Arc::new(Mock(Behavior::Fast)));

        let state = make_state(models, fb, adapters, Some(1));
        let mut s = stream_with_fallback(&state, req(), "primary")
            .await
            .unwrap();
        assert!(matches!(
            s.next().await.unwrap().unwrap(),
            NormalizedEvent::MessageStop { .. }
        ));
    }

    #[tokio::test]
    async fn fallback_on_stream_creation_error() {
        let mut models = BTreeMap::new();
        models.insert(
            "primary".into(),
            ModelRoute {
                provider: "err".into(),
                model: "m".into(),
            },
        );
        models.insert(
            "backup".into(),
            ModelRoute {
                provider: "fast".into(),
                model: "m".into(),
            },
        );
        let mut fb = BTreeMap::new();
        fb.insert("primary".into(), vec!["backup".into()]);
        let mut adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        adapters.insert("err".into(), Arc::new(Mock(Behavior::StreamErr)));
        adapters.insert("fast".into(), Arc::new(Mock(Behavior::Fast)));

        let state = make_state(models, fb, adapters, Some(1));
        let mut s = stream_with_fallback(&state, req(), "primary")
            .await
            .unwrap();
        assert!(matches!(
            s.next().await.unwrap().unwrap(),
            NormalizedEvent::MessageStop { .. }
        ));
    }

    #[tokio::test]
    async fn fallback_on_first_chunk_error() {
        let mut models = BTreeMap::new();
        models.insert(
            "primary".into(),
            ModelRoute {
                provider: "ferr".into(),
                model: "m".into(),
            },
        );
        models.insert(
            "backup".into(),
            ModelRoute {
                provider: "fast".into(),
                model: "m".into(),
            },
        );
        let mut fb = BTreeMap::new();
        fb.insert("primary".into(), vec!["backup".into()]);
        let mut adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        adapters.insert("ferr".into(), Arc::new(Mock(Behavior::FirstErr)));
        adapters.insert("fast".into(), Arc::new(Mock(Behavior::Fast)));

        let state = make_state(models, fb, adapters, Some(1));
        let mut s = stream_with_fallback(&state, req(), "primary")
            .await
            .unwrap();
        assert!(matches!(
            s.next().await.unwrap().unwrap(),
            NormalizedEvent::MessageStop { .. }
        ));
    }

    #[tokio::test]
    async fn all_fallbacks_exhausted_returns_504() {
        let mut models = BTreeMap::new();
        models.insert(
            "primary".into(),
            ModelRoute {
                provider: "slow".into(),
                model: "m".into(),
            },
        );
        let mut adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        adapters.insert("slow".into(), Arc::new(Mock(Behavior::Pending)));

        let state = make_state(models, BTreeMap::new(), adapters, Some(1));
        let result = stream_with_fallback(&state, req(), "primary").await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.http_status, 504);
        assert!(err.retryable);
    }

    #[tokio::test]
    async fn unknown_model_returns_error() {
        let state = make_state(BTreeMap::new(), BTreeMap::new(), HashMap::new(), Some(1));
        assert!(stream_with_fallback(&state, req(), "nonexistent")
            .await
            .is_err());
    }

    #[test]
    fn fallback_chain_primary_then_fallbacks() {
        let mut fb = BTreeMap::new();
        fb.insert("a".into(), vec!["b".into(), "c".into()]);
        assert_eq!(
            build_fallback_chain(&fb, "a"),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn fallback_chain_no_fallbacks() {
        assert_eq!(
            build_fallback_chain(&BTreeMap::new(), "a"),
            vec!["a".to_string()]
        );
    }
}
