use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use chrono::{offset::Local, DateTime};
use clap::Parser;
use rand::Rng;
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
    pub animals: Vec<Animal>,
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

struct AnimalShards {
    animal: Animal,
    // Obviously, `Mutex` is not oblifatory to work with the cached facts.
    // The alternative I find the most natural is the usage of `tokio::sync::watch`
    // with the receivers in the AppState and the producer in the fact-requesting task;
    // however, I don't really feel like dealing with the RW-locks `watch` uses under the hood.
    shards: Vec<Mutex<Shard>>,
}

#[derive(Clone)]
struct AppState {
    cache: Arc<AnimalShards>,
    cfg: ServerConfig,
}

fn init_state(cfg: ServerConfig) -> AppState {
    let mut cache = Vec::with_capacity(cfg.shard_num);
    for _ in 0..cfg.shard_num {
        cache.push(Mutex::new(Shard::default()));
    }
    let animal = cfg.animals[0].clone(); // TODO
    AppState {
        cache: Arc::new(AnimalShards {
            animal: animal,
            shards: cache,
        }),
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
async fn fact(State(state): State<AppState>) -> Json<HashMap<String, String>> {
    let mut rng = rand::thread_rng();
    let shard_index = rng.gen_range(0..state.cfg.shard_num);
    let fact_index = rng.gen_range(0..state.cfg.shard_size);
    let fact = state.cache.shards[shard_index].lock().unwrap().facts[fact_index].clone();
    Json(HashMap::from([
        ("animal".to_string(), state.cache.animal.to_string()),
        ("fact".to_string(), fact),
    ]))
}

// For the sake of simplicity each shard contains all facts from a signle response.
// As both cat- and dog-related APIs are expected to respond with OK, it seems convenient
// to check response staus codes within this function. Though in future one may move
// this logic to other functions, only shard filling is essential here.
async fn get_animal_facts(state: &AppState) -> Result<(), AppError> {
    let animal = &state.cache.animal;
    let client = reqwest::Client::new();
    let url = url(animal, state.cfg.shard_size);
    for shard_index in 0..state.cfg.shard_num {
        let response = client.get(&url).send().await?;
        match response.status() {
            StatusCode::OK => (),
            code => return Err(AppError::UnexpectedStatusCode(code)),
        }
        let shard = validate_batch(response.text().await?, animal, state.cfg.shard_size)?;
        *state.cache.shards[shard_index].lock().unwrap() = shard;
    }
    Ok(())
}
