use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use ma_core::{AppConfig, provider_test};

#[derive(Debug, Parser)]
#[command(name = "ma")]
#[command(about = "Models Assemble coding-agent provider gateway")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve {
        #[arg(short, long, default_value = "examples/config.example.yaml")]
        config: PathBuf,
    },
    Validate {
        #[arg(short, long, default_value = "examples/config.example.yaml")]
        config: PathBuf,
    },
    Doctor {
        #[arg(short, long, default_value = "examples/config.example.yaml")]
        config: PathBuf,
    },
    TestProvider {
        model_alias: String,
        #[arg(short, long, default_value = "examples/config.example.yaml")]
        config: PathBuf,
        #[arg(long)]
        stream: bool,
    },
    CompatProbe {
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: SocketAddr,
        #[arg(long, value_enum, default_value_t = CompatMode::Both)]
        mode: CompatMode,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CompatMode {
    Anthropic,
    Openai,
    Both,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve { config } => {
            let config = load_and_validate(config)?;
            ma_server::serve(config).await
        }
        Command::Validate { config } => {
            load_and_validate(config)?;
            println!("config is valid");
            Ok(())
        }
        Command::Doctor { config } => {
            let config = load_and_validate(config)?;
            run_doctor(&config);
            Ok(())
        }
        Command::TestProvider {
            model_alias,
            config,
            stream,
        } => {
            let config = load_and_validate(config)?;
            let result = provider_test::test_provider(&config, &model_alias, stream).await?;
            println!("status: {}", result.status);
            println!("text preview: {}", result.text_preview);
            Ok(())
        }
        Command::CompatProbe { bind, mode } => {
            println!("starting compat-probe on http://{bind}");
            match mode {
                CompatMode::Anthropic => {
                    println!("anthropic endpoint: POST http://{bind}/v1/messages");
                }
                CompatMode::Openai => {
                    println!("openai endpoint: POST http://{bind}/v1/chat/completions");
                }
                CompatMode::Both => {
                    println!("anthropic endpoint: POST http://{bind}/v1/messages");
                    println!("openai endpoint: POST http://{bind}/v1/chat/completions");
                }
            }
            ma_server::serve(ma_server::handlers::openai::compat_config(bind)).await
        }
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "ma_cli=info,ma_server=info".to_string()),
        )
        .init();
}

fn load_and_validate(config: PathBuf) -> anyhow::Result<AppConfig> {
    let config = AppConfig::load(&config).inspect_err(|error| {
        if let Some(raw_debug) = &error.raw_debug {
            eprintln!("config debug: {raw_debug}");
        }
    }).with_context(|| {
        format!(
            "failed to load config. Try `ma compat-probe` for a config-free mock server, or create {}",
            config.display()
        )
    })?;

    if let Err(errors) = config.validate() {
        for error in &errors {
            eprintln!("config error: {}", error.safe_message);
        }
        anyhow::bail!("config validation failed");
    }

    Ok(config)
}

fn run_doctor(config: &AppConfig) {
    println!("server.bind: {}", config.server.bind);
    println!("models: {}", config.models.len());
    println!("providers: {}", config.providers.len());

    if config.server.bind.ip().is_unspecified() {
        println!("warning: server is configured to listen on all interfaces");
    }

    if config.server.api_keys.is_empty() {
        println!("warning: local API key auth is disabled");
    }

    for (name, provider) in &config.providers {
        match &provider.api_key_env {
            Some(env_name) if std::env::var(env_name).is_ok() => {
                println!("provider {name}: api key env {env_name} is set");
            }
            Some(env_name) => {
                println!("provider {name}: warning api key env {env_name} is not set");
            }
            None => {
                println!("provider {name}: no api key env configured");
            }
        }
    }
}
