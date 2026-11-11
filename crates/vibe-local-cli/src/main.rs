//! `vibe-local` — local AI inference manager CLI.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use vibe_local_cli::{commands, discovery, server};
use vibe_local_core::{Db, resolve_db_path, resolve_models_dir};
use vibe_local_engine::Engine;

#[derive(Parser)]
#[command(
    name = "vibe-local",
    version,
    about = "Vibe Local — zero-cloud local GGUF inference, hardware-aware MoE vs Dense recommendations and the 10-pillar quantization leaderboard",
    long_about = "Vibe Local runs GGUF models fully offline, recommends MoE vs Dense architectures for your exact hardware, benchmarks quantizations across 10 independent pillars, and exposes an OpenAI-compatible Vibe-Link endpoint for Vibe Router on 127.0.0.1:20129."
)]
struct Cli {
    /// Explicit SQLite database path (overrides portable/default resolution)
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    /// Prefer portable USB storage (portable_data/vibe_local.db)
    #[arg(long, global = true)]
    portable: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Detect RAM, CPU features, GPU/VRAM and the MoE-vs-Dense tier
    Scan {
        #[arg(long)]
        json: bool,
    },
    /// Discover GGUF quants on Hugging Face (bartowski, unsloth, ggml-org, MaziyarPanahi)
    Discover {
        query: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Parse a GGUF header and compute RAM/VRAM needs at 2k/4k/8k/16k context
    InspectGguf {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Run the 15-second VibeMark speed benchmark on a GGUF file or HF model id
    Bench {
        target: String,
        /// Submit the measured entry to a Vibe-Link server, e.g. http://127.0.0.1:20129
        #[arg(long)]
        submit: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 15)]
        budget: u64,
    },
    /// Start the Vibe-Link HTTP server (default 127.0.0.1:20129)
    Serve {
        #[arg(long, default_value_t = server::DEFAULT_PORT)]
        port: u16,
        /// Bind 0.0.0.0 for LAN Pocket Worker mode (requires token auth)
        #[arg(long)]
        lan: bool,
        /// Pre-shared LAN token (generated and stored if omitted)
        #[arg(long)]
        token: Option<String>,
        /// Idle auto-unload timeout in seconds
        #[arg(long, default_value_t = 300)]
        idle: u64,
    },
    /// Print the 10-pillar leaderboard (from a running server or the local DB)
    Leaderboard {
        #[arg(long, default_value_t = server::DEFAULT_PORT)]
        port: u16,
        #[arg(long, default_value = "")]
        profile: String,
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let db_path = resolve_db_path(cli.db.as_deref(), cli.portable);
    let db = Db::open(&db_path)?.shared();

    match cli.command {
        Command::Scan { json } => {
            let report = commands::scan(&db)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let report: vibe_local_core::HardwareReport = serde_json::from_value(report)?;
                commands::print_hardware(&report);
                let rec = vibe_local_core::recommend(&report);
                println!();
                commands::print_recommendation(&rec);
            }
        }
        Command::Discover { query, json, limit } => {
            let files = commands::discover(&db, &query, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&files)?);
            } else if files.is_empty() {
                println!("No GGUF files found for '{query}'.");
            } else {
                for f in &files {
                    println!(
                        "{}{} {} [{}] {:.2} GB  (downloads: {}, likes: {})",
                        if f.trusted { "★ " } else { "  " },
                        f.hf_repo,
                        f.filename,
                        f.quant.as_deref().unwrap_or("?"),
                        f.size_bytes as f64 / 1e9,
                        f.downloads,
                        f.likes
                    );
                    println!("    {}", f.download_url);
                }
            }
        }
        Command::InspectGguf { path, json } => {
            let info = commands::inspect(&db, &path)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                commands::print_inspection(&info);
            }
        }
        Command::Bench {
            target,
            submit,
            json,
            budget,
        } => {
            commands::import_seed_catalog(&db)?;
            let models_dir = resolve_models_dir(None, cli.portable);
            let engine = Engine::start(commands::engine_config(300));
            let outcome = commands::bench(&db, &engine, &target, &models_dir, budget);
            engine.shutdown();
            let outcome = outcome?;
            if let Some(url) = submit {
                let resp = commands::submit_entry(&url, &outcome.entry)?;
                eprintln!("submitted: {resp}");
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "vibemark": outcome.result,
                        "entry": outcome.entry,
                    }))?
                );
            } else {
                commands::print_vibemark(&outcome.result);
            }
        }
        Command::Serve {
            port,
            lan,
            token,
            idle,
        } => {
            let imported = commands::import_seed_catalog(&db)?;
            let token = if lan {
                Some(commands::resolve_lan_token(&db, token)?)
            } else {
                token
            };
            let engine = Engine::start(commands::engine_config(idle));
            let config = server::ServerConfig {
                port,
                lan,
                token: token.clone(),
                db: db.clone(),
                engine: engine.clone(),
                web_root: std::env::var("VIBE_WEB_ROOT").ok().map(PathBuf::from),
                catalog_path: None,
            };
            let handle = server::start_server(config)?;
            let disc = discovery::start_discovery(discovery::DiscoveryConfig {
                http_port: handle.port,
                lan,
                udp_port: discovery::UDP_PROBE_PORT,
                mdns_port: discovery::MDNS_PORT,
            })
            .ok();
            println!("Vibe Local v{}", env!("CARGO_PKG_VERSION"));
            println!(
                "  Vibe-Link:   http://{}:{}",
                if lan { "0.0.0.0" } else { "127.0.0.1" },
                handle.port
            );
            println!(
                "  Leaderboard: http://127.0.0.1:{}/leaderboard",
                handle.port
            );
            println!("  Database:    {}", db_path.display());
            println!("  Seed catalog entries imported: {imported}");
            println!("  Idle auto-unload: {idle}s");
            if lan {
                println!("  LAN Pocket Worker mode: ENABLED");
                println!("  X-Vibe-Local-Token: {}", token.unwrap_or_default());
                println!(
                    "  (Vibe Router: pair with this token; all /api/* and /v1/* routes require it)"
                );
            }
            if disc.is_some() {
                println!(
                    "  Discovery: UDP :{} + mDNS _vibe-local._tcp.local",
                    discovery::UDP_PROBE_PORT
                );
            }
            if vibe_local_engine::supervisor::find_llama_server(None).is_none() {
                println!(
                    "  NOTE: llama-server not found — chat/VibeMark use the deterministic builtin micro-eval engine."
                );
                println!(
                    "        Set VIBE_LLAMA_SERVER or install llama.cpp for full LLM inference."
                );
            }
            // Block until terminated; Drop handlers clean up children.
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        Command::Leaderboard {
            port,
            profile,
            json,
        } => {
            // Prefer a running server; fall back to the local database.
            let url = format!("http://127.0.0.1:{port}/api/leaderboard?profile={profile}");
            let value = match ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .get(&url)
                .call()
            {
                Ok(resp) => resp.into_json()?,
                Err(_) => {
                    commands::import_seed_catalog(&db)?;
                    commands::leaderboard_json(&db, &profile)?
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                commands::print_leaderboard(&value);
            }
        }
    }
    Ok(())
}
