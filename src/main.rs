use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use chrono::{offset::Local, DateTime};
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

// TODO: validate it
const SHARD_NUM: usize = 1;
const SHARD_SIZE: usize = 10;
const REFRESH_INTERVAL_SEC: u64 = 2;

const ANIMAL: Animal = Animal::Cat; // TODO: remove me

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

type AppState = Arc<AnimalShards>;

fn init_state() -> AppState {
    let mut cache = Vec::with_capacity(SHARD_NUM);
    for _ in 0..SHARD_NUM {
        cache.push(Mutex::new(Shard::default()));
    }
    Arc::new(AnimalShards {
        animal: ANIMAL,
        shards: cache,
    })
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let state = init_state();
    get_animal_facts(&state).await?;

    let state_clone = state.clone();
    task::spawn(async move {
        loop {
            sleep(Duration::from_secs(REFRESH_INTERVAL_SEC)).await;
            if let Err(e) = get_animal_facts(&state_clone).await {
                eprintln!("Fact fetching error: {:?}", e);
            };
        }
    });

    let app = Router::new().route("/fact", get(fact)).with_state(state);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
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
    let (shard_index, fact_index) = (rng.gen_range(0..SHARD_NUM), rng.gen_range(0..SHARD_SIZE));
    let fact = state.shards[shard_index].lock().unwrap().facts[fact_index].clone();
    Json(HashMap::from([
        ("animal".to_string(), state.animal.to_string()),
        ("fact".to_string(), fact),
    ]))
}

// For the sake of simplicity each shard contains all facts from a signle response.
// As both cat- and dog-related APIs are expected to respond with OK, it seems convenient
// to check response staus codes within this function. Though in future one may move
// this logic to other functions, only shard filling is essential here.
async fn get_animal_facts(state: &AppState) -> Result<(), AppError> {
    let animal = &state.animal;
    let client = reqwest::Client::new();
    let url = url(animal);
    for shard_index in 0..SHARD_NUM {
        let response = client.get(&url).send().await?;
        match response.status() {
            StatusCode::OK => (),
            code => return Err(AppError::UnexpectedStatusCode(code)),
        }
        let shard = validate_batch(response.text().await?, animal)?;
        *state.shards[shard_index].lock().unwrap() = shard;
    }
    Ok(())
}
