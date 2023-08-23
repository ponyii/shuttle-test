use axum::{extract::State, http::HeaderMap, http::StatusCode, routing::get, Json, Router};
use chrono::LocalResult;
use chrono::{TimeZone, Utc};
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
use errors::{AppError, HealthProblem};

pub mod animals;
pub mod config;
pub mod errors;

#[derive(Default)]
pub struct Shard {
    pub facts: Vec<String>,
    pub timestamp: i64,
}

impl Shard {
    pub fn new(facts: Vec<String>) -> Self {
        Self {
            facts,
            timestamp: Utc::now().timestamp(),
        }
    }
}

struct ShardSet {
    animal: Animal,
    // On the alternatives of the sharded `Mutex` see README.md
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
            shards.push(Mutex::new(Shard::new(vec![])));
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
    refresh_shards(&state).await?;

    let state_clone = state.clone();
    task::spawn(async move {
        loop {
            sleep(Duration::from_secs(state_clone.cfg.shard_refresh_sec)).await;
            if let Err(e) = refresh_shards(&state_clone).await {
                tracing::error!("Fact fetching error: {:?}", e);
            };
        }
    });

    let socket_addr = format!("0.0.0.0:{}", state.cfg.port)
        .parse()
        .expect("Unable to parse socket address");
    let app = Router::new()
        .route("/fact", get(fact))
        .route("/health", get(health))
        .with_state(state);
    axum::Server::bind(&socket_addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

// I assume it's OK to return a fact without checking if it's "fresh";
// this policy allows the server to keep runnig in case a fact provider
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

// Health check is accessible to anyone, hence it doesn't return anything but a status code;
// see logs for diagnostics.
async fn health(State(state): State<AppState>) -> (StatusCode, HeaderMap) {
    let mut headers = HeaderMap::new();
    headers.insert("Cache-Control", "no-cache".parse().unwrap());

    if let Err(_) = check_app_state(&state) {
        return (StatusCode::INTERNAL_SERVER_ERROR, headers);
    }
    (StatusCode::OK, headers)
}

fn check_app_state(state: &AppState) -> Result<(), HealthProblem> {
    if state.cache.len() != state.cfg.animals.len() {
        tracing::error!("Unexpected number of shard sets");
        return Err(HealthProblem::UnexpectedState);
    }
    for shard_set in state.cache.as_ref() {
        let shard_num = shard_set.shards.len();
        if shard_num != state.cfg.shard_num {
            tracing::error!(
                "Incorrect number of shards: {:?} ({:?} shard set)",
                shard_num,
                shard_set.animal
            );
            return Err(HealthProblem::UnexpectedState);
        }
        for i in 0..shard_set.shards.len() {
            let shard = shard_set.shards[i].lock()?;
            let fact_num = shard.facts.len();
            if fact_num != state.cfg.shard_size {
                tracing::error!(
                    "Incorrect number of facts: {:?} (shard {:?}, {:?} shard set)",
                    fact_num,
                    i,
                    shard_set.animal
                );
                return Err(HealthProblem::UnexpectedState);
            };
            match Utc.timestamp_opt(shard.timestamp, 0) {
                LocalResult::Single(time) => {
                    if (Utc::now() - time).num_seconds() >= state.cfg.shard_staleness_sec {
                        tracing::error!(
                            "Stale shard found (shard {:?}, {:?} shard set)",
                            i,
                            shard_set.animal
                        );
                        return Err(HealthProblem::StaleShard);
                    };
                }
                _ => {
                    tracing::error!(
                        "Invalid timestamp found (shard {:?}, {:?} shard set)",
                        i,
                        shard_set.animal
                    );
                    return Err(HealthProblem::UnexpectedState);
                }
            }
        }
    }
    Ok(())
}

// For the sake of simplicity each shard contains all facts from a signle response.
// It's also for the sake of simplicity that requests are sent one by one;
// if need be, the requests to fact providers can become really async, naturally.
async fn refresh_shards(state: &AppState) -> Result<(), AppError> {
    tracing::debug!("Fetching animal facts");
    let client = reqwest::Client::new();
    for shard_set in state.cache.as_ref() {
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
            shard_staleness_sec: 1,
            verbosity: tracing::Level::TRACE,
            animals,
        };
    }

    async fn set_up_test_server(cfg: ServerConfig) -> (TestServer, AppState) {
        let state = init_state(cfg);
        refresh_shards(&state).await.unwrap();
        if let Err(_) = check_app_state(&state) {
            panic!("Invalid initial state");
        }
        let app = Router::new()
            .route("/fact", get(fact))
            .route("/health", get(health))
            .with_state(state.clone())
            .into_make_service();
        (TestServer::new(app).unwrap(), state)
    }

    // In fact this struct is not necessary as
    // `validate_response` has to work anyway with a raw `Value`.
    #[derive(Deserialize, Debug)]
    struct RandomFact {
        fact: String,
        animal: String,
    }

    async fn get_fact(server: &TestServer, expected_animals: &HashSet<String>) {
        let response = server.get("/fact").await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let body = response.text();

        // Validate expected fields
        let parsed_response = serde_json::from_str::<RandomFact>(&body).unwrap();
        assert!(
            expected_animals.contains(&parsed_response.animal),
            "Incorrect animal in the response: {:?}",
            parsed_response
        );
        // Currrently all we know is that each fact is a string, but further validation can be added later
        assert!(
            parsed_response.fact.len() > 0,
            "Incorrect fact in the response: {:?}",
            parsed_response
        );

        // Make sure there are no extra fields.
        // serde_json seems to simply ignore them, see https://github.com/serde-rs/json/issues/581
        let value: Value = serde_json::from_str(&body).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(
            object.keys().len(),
            2,
            "Extra fields in the response: {:?}",
            value
        );
    }

    async fn get_health(server: &TestServer) {
        let response = server.get("/health").await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }

    // An alternative to repetitive requests is `rng` mocking.
    const REQUEST_NUM: u8 = 10;

    async fn tets_api_inner(animals: Vec<Animal>) {
        let animal_set: HashSet<_> = animals.iter().map(|a| a.to_string()).collect();
        let (server, _) = set_up_test_server(get_test_config(animals)).await;
        for _ in 0..REQUEST_NUM {
            get_fact(&server, &animal_set).await;
        }
    }

    #[tokio::test]
    async fn test_api() {
        tets_api_inner(vec![Animal::Cat]).await;
        tets_api_inner(vec![Animal::Dog]).await;
        tets_api_inner(vec![Animal::Cat, Animal::Dog]).await;
    }

    const UPDATE_NUM: u8 = 10;

    // If need be, one can split this test into fast (without staleness checks and sleeping)
    // and slow versions.
    #[tokio::test]
    async fn test_shard_refreshing() {
        let animals = vec![Animal::Cat];
        let animal_set: HashSet<_> = animals.iter().map(|a| a.to_string()).collect();
        let (server, state) = set_up_test_server(get_test_config(animals)).await;

        for _ in 0..UPDATE_NUM {
            refresh_shards(&state).await.unwrap();
            get_health(&server).await;
            get_fact(&server, &animal_set).await;
            get_health(&server).await;
            sleep(Duration::from_secs(state.cfg.shard_staleness_sec as u64)).await;
        }
    }
}
