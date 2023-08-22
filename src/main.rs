use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use chrono::{offset::Local, DateTime};
use clap::Parser;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::{
    task,
    time::{sleep, Duration},
};
use tracing;
use tracing_subscriber;

use animals::{url, validate_batch, Animal};
use errors::AppError;

pub mod animals;
pub mod errors;

#[derive(Clone, Parser)]
pub struct ServerConfig {
    #[arg(short, long, default_value_t = 3000)]
    pub port: u16,

    /// Number of shards per animal type
    #[arg(long, default_value_t = 1)]
    pub shard_num: usize,

    /// Number of animal facts per shard
    #[arg(long, default_value_t = 50)]
    pub shard_size: usize, // TODO: validate

    /// Frequency of shard refreshing (sec)
    #[arg(long, default_value_t = 2)]
    pub shard_refresh_sec: u64,

    #[arg(short, long, default_value_t = tracing::Level::INFO)]
    pub verbosity: tracing::Level,

    /// Animals you are interested in (comma-separated)
    #[arg(long, value_parser, value_delimiter = ',', default_values_t = vec![Animal::Cat, Animal::Dog])]
    pub animals: Vec<Animal>, // TODO: deduplicate
}

#[derive(Default)]
pub struct Shard {
    pub facts: Vec<String>,
    pub timestamp: Option<DateTime<Local>>,
}

impl Shard {
    pub fn new(facts: Vec<String>) -> Self {
        Self {
            facts,
            timestamp: Some(Local::now()),
        }
    }
}

struct ShardSet {
    animal: Animal,
    // Obviously, `Mutex` is not oblifatory to work with the cached facts.
    // The alternative I find the most natural is the usage of `tokio::sync::watch`
    // with the receivers in the AppState and the producer in the fact-requesting task;
    // however, I don't really feel like dealing with the RW-locks `watch` uses under the hood.
    shards: Vec<Mutex<Shard>>,
}

#[derive(Clone)]
struct AppState {
    cache: Arc<Vec<ShardSet>>,
    cfg: ServerConfig,
}

fn init_state(cfg: ServerConfig) -> AppState {
    let mut cache = Vec::with_capacity(cfg.shard_num);
    for animal in &cfg.animals {
        let mut shards = Vec::with_capacity(cfg.shard_num);
        for _ in 0..cfg.shard_num {
            shards.push(Mutex::new(Shard::default()));
        }
        cache.push(ShardSet {
            animal: animal.clone(),
            shards,
        });
    }
    AppState {
        cache: Arc::new(cache),
        cfg,
    }
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let cfg = ServerConfig::parse();

    tracing_subscriber::fmt()
        .with_max_level(cfg.verbosity)
        .init();

    let state = init_state(cfg);
    get_animal_facts(&state).await?;

    let state_clone = state.clone();
    task::spawn(async move {
        loop {
            sleep(Duration::from_secs(state_clone.cfg.shard_refresh_sec)).await;
            if let Err(e) = get_animal_facts(&state_clone).await {
                eprintln!("Fact fetching error: {:?}", e);
            };
        }
    });

    let socket_addr = format!("0.0.0.0:{}", state.cfg.port)
        .parse()
        .expect("Unable to parse socket address");
    let app = Router::new().route("/fact", get(fact)).with_state(state);
    axum::Server::bind(&socket_addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

// I assume it's OK to return a fact without checking if it's "fresh";
// this policy allows the server to keep runnig in case a source of facts
// is temporary unavailable. Naturally, this check could have been performed and
// a special "no fresh animal facts" error message could have been added.
async fn fact(State(state): State<AppState>) -> Result<Json<HashMap<String, String>>, AppError> {
    let mut rng = rand::thread_rng();
    let shard_set = state.cache.choose(&mut rng).ok_or(AppError::NoData)?;
    let shard = shard_set.shards.choose(&mut rng).ok_or(AppError::NoData)?;
    let facts = &shard.lock().unwrap().facts;
    let result = facts.choose(&mut rng).ok_or(AppError::NoData)?;
    Ok(Json(HashMap::from([
        ("animal".to_string(), shard_set.animal.to_string()),
        ("fact".to_string(), result.clone()),
    ])))
}

// For the sake of simplicity each shard contains all facts from a signle response.
// As both cat- and dog-related APIs are expected to respond with OK, it seems convenient
// to check response staus codes within this function. Though in future one may move
// this logic to other functions, only shard filling is essential here.
async fn get_animal_facts(state: &AppState) -> Result<(), AppError> {
    for shard_set in state.cache.as_ref() {
        let client = reqwest::Client::new();
        let url = url(&shard_set.animal, state.cfg.shard_size);
        for shard in &shard_set.shards {
            let response = client.get(&url).send().await?;
            match response.status() {
                StatusCode::OK => (),
                code => return Err(AppError::UnexpectedStatusCode(code)),
            }
            let new_shard = validate_batch(
                response.text().await?,
                &shard_set.animal,
                state.cfg.shard_size,
            )?;
            *shard.lock().unwrap() = new_shard;
        }
    }
    Ok(())
}
