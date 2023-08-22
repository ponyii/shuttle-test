use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use chrono::{offset::Local, DateTime};
use rand::Rng;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::{
    task,
    time::{sleep, Duration},
};
use tracing;
use tracing_subscriber;

// TODO: validate it
const SHARD_NUM: usize = 1;
const SHARD_SIZE: usize = 10;
const REFRESH_INTERVAL_SEC: u64 = 2;

const DOG_ENDPOINT: &str = "https://dog-api.kinduff.com/api/facts?number=";

#[derive(Default)]
struct Shard {
    facts: Vec<String>,
    timestamp: Option<DateTime<Local>>,
}

// Obviously, there're other ways to work with the cached facts.
// The alternative I find the most natural is the usage of `tokio::sync::watch`
// with the receivers in the AppState and the producer in the fact-requesting task;
// however, I don't really feel like dealing with the RW-locks `watch` uses under the hood.
type AppState = Arc<Vec<Mutex<Shard>>>;

fn init_state() -> AppState {
    let mut cache = Vec::with_capacity(SHARD_NUM);
    for _ in 0..SHARD_NUM {
        cache.push(Mutex::new(Shard::default()));
    }
    Arc::new(cache)
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let state = init_state();
    get_dog_facts(&state).await?;

    let state_clone = state.clone();
    task::spawn(async move {
        loop {
            sleep(Duration::from_secs(REFRESH_INTERVAL_SEC)).await;
            if let Err(e) = get_dog_facts(&state_clone).await {
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
    let fact = state[shard_index].lock().unwrap().facts[fact_index].clone();
    Json(HashMap::from([
        ("animal".to_string(), "dog".to_string()),
        ("fact".to_string(), fact),
    ]))
}

#[derive(Deserialize, Debug)]
struct DogFactBatch {
    facts: Vec<String>,
    success: bool,
}

// For the sake of simplicity each shard contains all facts from a signle response.
async fn get_dog_facts(state: &AppState) -> Result<(), AppError> {
    let url = format!("{}{}", DOG_ENDPOINT, SHARD_SIZE);
    let client = reqwest::Client::new();
    for shard_index in 0..SHARD_NUM {
        let response = client.get(&url).send().await?;
        match response.status() {
            StatusCode::OK => (),
            code => return Err(AppError::UnexpectedStatusCode(code)),
        }
        let batch = validate_dog_fact_batch(response.text().await?)?;
        *state[shard_index].lock().unwrap() = Shard {
            facts: batch.facts,
            timestamp: Some(Local::now()),
        };
    }
    Ok(())
}

fn validate_dog_fact_batch(body: String) -> Result<DogFactBatch, AppError> {
    match serde_json::from_str::<DogFactBatch>(&body) {
        // TODO: how safe is it?
        Ok(batch) => {
            if !batch.success {
                return Err(AppError::InvalidData(
                    "The 'success' flag is false".to_string(),
                ));
            } else if batch.facts.len() != SHARD_SIZE {
                return Err(AppError::InvalidData(format!(
                    "Unexpected number of facts received: {} instead of {}",
                    batch.facts.len(),
                    SHARD_SIZE
                )));
            }
            // One may add more data checks here. E.g. it might make sense to exclude
            // too long facts from the batch so as to control the amount of memory
            // used per batch, especially if the fact source can't be trusted.
            Ok(batch)
        }
        Err(e) => Err(AppError::JsonParsingError(e)),
    }
}

#[derive(Debug)]
enum AppError {
    RequestError(reqwest::Error),
    JsonParsingError(serde_json::Error),
    UnexpectedStatusCode(StatusCode),
    InvalidData(String),
    // TODO: PoisonError
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        Self::RequestError(value)
    }
}
