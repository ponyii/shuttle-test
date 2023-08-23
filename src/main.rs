use axum::{extract::State, routing::get, Json, Router};
use chrono::{offset::Local, DateTime};
use clap::Parser;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::{
    task,
    time::{sleep, Duration},
};
use tracing_subscriber;

use animals::{fetch_raw_facts, validate_batch, Animal};
use config::ServerConfig;
use errors::AppError;

pub mod animals;
pub mod config;
pub mod errors;

#[derive(Default)]
pub struct Shard {
    pub facts: Vec<String>,
    // Initially I planned to use timestamps for health checks:
    // stale shards indicate problems with data fetching.
    // But it seems that I don't have time for this feature now.
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
    // Obviously, `Mutex` is not oblifatory to work with the cached facts; sharding is also optional.
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
    let mut cfg = ServerConfig::parse();
    cfg.deduplicate_animals();

    tracing_subscriber::fmt()
        .with_max_level(cfg.verbosity)
        .init();

    let state = init_state(cfg);
    // Though fact providers are allowed to become unavailable as server runs,
    // it can't start unless they all have responded correctly.
    // Optionally, one could exclude the species whose fact providers are unavailable,
    // and keep the server running if at least one species' API responded correctly.
    get_animal_facts(&state).await?;

    let state_clone = state.clone();
    task::spawn(async move {
        loop {
            sleep(Duration::from_secs(state_clone.cfg.shard_refresh_sec)).await;
            if let Err(e) = get_animal_facts(&state_clone).await {
                tracing::error!("Fact fetching error: {:?}", e);
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
    let facts = &shard.lock()?.facts;
    let result = facts.choose(&mut rng).ok_or(AppError::NoData)?;
    Ok(Json(HashMap::from([
        ("animal".to_string(), shard_set.animal.to_string()),
        ("fact".to_string(), result.clone()),
    ])))
}

// For the sake of simplicity each shard contains all facts from a signle response.
// It's also for the sake of simplicity that requests are sent one by one;
// if need be, the requests to fact providers can become really async, naturally.
async fn get_animal_facts(state: &AppState) -> Result<(), AppError> {
    tracing::debug!("Fetching animal facts");
    for shard_set in state.cache.as_ref() {
        let client = reqwest::Client::new();
        for shard in &shard_set.shards {
            let new_shard = validate_batch(
                fetch_raw_facts(&client, &shard_set.animal, state.cfg.shard_size).await?,
                &shard_set.animal,
                state.cfg.shard_size,
            )?;
            *shard.lock()? = new_shard;
        }
    }
    Ok(())
}

// Due to lack of time, I have to limit myself to basic tests.
// Ideally, fact validators deserve thorough testing as they work with third-party data.
#[cfg(test)]
mod test {
    use crate::*;

    use axum::http::StatusCode;
    use axum_test::TestServer;
    use serde::Deserialize;
    use serde_json::Value;
    use std::collections::HashSet;

    fn get_test_config(animals: Vec<Animal>) -> ServerConfig {
        return ServerConfig {
            port: 3000,
            shard_num: 2,
            shard_size: 50,
            shard_refresh_sec: 2,
            verbosity: tracing::Level::TRACE,
            animals,
        };
    }

    fn validate_app_state(state: &AppState) {
        assert_eq!(
            state.cache.len(),
            state.cfg.animals.len(),
            "each animal should have its own shard set"
        );
        for shard_set in state.cache.as_ref() {
            assert_eq!(
                shard_set.shards.len(),
                state.cfg.shard_num,
                "incorrect number of shards"
            );
            for shard in &shard_set.shards {
                assert_eq!(
                    shard.lock().unwrap().facts.len(),
                    state.cfg.shard_size,
                    "incorrect shard size"
                );
            }
        }
    }

    async fn set_up_test_server(cfg: ServerConfig) -> TestServer {
        let state = init_state(cfg);
        get_animal_facts(&state).await.unwrap();
        validate_app_state(&state);
        let app = Router::new()
            .route("/fact", get(fact))
            .with_state(state)
            .into_make_service();
        TestServer::new(app).unwrap()
    }

    // In fact this struct is not necessary as
    // `validate_response` has to work anyway with a raw `Value`.
    #[derive(Deserialize, Debug)]
    struct RandomFact {
        fact: String,
        animal: String,
    }

    fn validate_response(body: &String, expected_animals: &HashSet<String>) {
        // Validate expected fields
        let parsed_response = serde_json::from_str::<RandomFact>(body).unwrap();
        assert!(
            expected_animals.contains(&parsed_response.animal),
            "a response includes incorrect animal: {:?}",
            parsed_response
        );
        // Currrently all we know is that each fact is a string, but further validation can be added later
        assert!(
            parsed_response.fact.len() > 0,
            "a response includes incorrect fact: {:?}",
            parsed_response
        );

        // Make sure there are no extra fields.
        // serde_json seems to simply ignore them, see https://github.com/serde-rs/json/issues/581
        let value: Value = serde_json::from_str(body).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(
            object.keys().len(),
            2,
            "a response includes extra fields: {:?}",
            value
        );
    }

    // An alternative to repetitive requests is `rng` mocking.
    const REQUEST_NUM: u8 = 10;

    async fn tets_api_inner(animals: Vec<Animal>) {
        let animal_set: HashSet<_> = animals.iter().map(|a| a.to_string()).collect();
        let server = set_up_test_server(get_test_config(animals)).await;
        for _ in 0..REQUEST_NUM {
            let response = server.get("/fact").await;
            assert_eq!(response.status_code(), StatusCode::OK);
            validate_response(&response.text(), &animal_set);
        }
    }

    #[tokio::test]
    async fn test_api() {
        tets_api_inner(vec![Animal::Cat]).await;
        tets_api_inner(vec![Animal::Dog]).await;
        tets_api_inner(vec![Animal::Cat, Animal::Dog]).await;
    }
}
