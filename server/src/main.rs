use anyhow::Context;
use clap::{Parser, Subcommand};
use emby_manager::{db, migrate, openapi::ApiDoc, settings::Settings, state::AppState};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utoipa::OpenApi;

#[derive(Debug, Parser)]
#[command(
    name = "emby-manager",
    version,
    about = "Rust + Docker rewrite of emby-manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve,
    Openapi,
    Migrate {
        #[arg(long, env = "EMBY_MANAGER_LEGACY_DIR", default_value = "/legacy")]
        legacy_dir: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = false)]
        apply: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve().await,
        Command::Openapi => {
            println!("{}", serde_json::to_string_pretty(&ApiDoc::openapi())?);
            Ok(())
        }
        Command::Migrate {
            legacy_dir,
            database_url,
            dry_run,
            apply,
        } => {
            if dry_run == apply {
                anyhow::bail!("choose exactly one of --dry-run or --apply");
            }
            let pool = db::connect_url(&database_url).await?;
            if apply {
                db::migrate(&pool).await?;
            }
            let report = migrate::run(&pool, legacy_dir.into(), apply).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
    }
}

async fn serve() -> anyhow::Result<()> {
    let settings = Settings::from_env()?;
    let pool = db::connect_url(&settings.database_url).await?;
    db::migrate(&pool).await?;
    emby_manager::auth::ensure_default_admin(&pool, &settings).await?;
    emby_manager::tasks::reconcile_interrupted(&pool).await?;
    emby_manager::scheduler::reconcile_interrupted(&pool).await?;

    let state = AppState::new(pool, settings.clone());
    emby_manager::scheduler::spawn_scheduler_loop(state.clone());
    let app = emby_manager::api::router_with_state(state);
    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port)
        .parse()
        .context("invalid bind host/port")?;
    tracing::info!(%addr, "starting emby-manager-rs");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "emby_manager=info,tower_http=info".into());
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}
