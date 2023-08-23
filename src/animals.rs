// This module contains the code requesting facts about different animals,
// validating the responses, etc.

#[cfg(not(test))]
use axum::http::StatusCode;
use clap::ValueEnum;
use serde::Deserialize;
#[cfg(test)]
use serde::Serialize;

use crate::errors::AppError;
use crate::Shard;

#[derive(Clone, Copy, ValueEnum)]
pub enum Animal {
    Dog,
    Cat,
    // New animal can be added here
}

impl ToString for Animal {
    fn to_string(&self) -> String {
        match self {
            Self::Dog => "dog".to_string(),
            Self::Cat => "cat".to_string(),
        }
    }
}

pub fn url(animal: &Animal, shard_size: usize) -> String {
    match animal {
        Animal::Dog => format!(
            "https://dog-api.kinduff.com/api/facts?number={}",
            shard_size
        ),
        Animal::Cat => format!(
            "https://cat-fact.herokuapp.com/facts/random?type=cat&amount={}",
            shard_size
        ),
    }
}

// It could have been a method of the `Animal` trait implemented for both species.
// As there are not many sepcies-specific parameters, I decided not to create a separate struct for each.
pub fn validate_batch(body: String, animal: &Animal, shard_size: usize) -> Result<Shard, AppError> {
    match animal {
        // One may add other data checks to the validators. E.g. it might make sense
        // to exclude too long facts from the batches so as to control the amount of
        // memory used, the fact providers can't be really trusted.
        Animal::Dog => validate_dog_facts(body, shard_size),
        Animal::Cat => validate_cat_facts(body, shard_size),
    }
}

#[cfg(not(test))]
pub async fn fetch_raw_facts(
    client: &reqwest::Client,
    animal: &Animal,
    shard_size: usize,
) -> Result<String, AppError> {
    let url = url(animal, shard_size);
    let response = client.get(url).send().await?;
    match response.status() {
        StatusCode::OK => (),
        // It doesn't seem necessary to implement retries, as these requests
        // are being re-sent routinely. Just wait for the next run.
        code => return Err(AppError::UnexpectedStatusCode(code)),
    };
    Ok(response.text().await?)
}

// The `mockall` library could be used instead.
// If need be, a custom attribute can be created to allow running tests
// with both real and fake `fetch_raw_facts` by choice.
#[cfg(test)]
pub async fn fetch_raw_facts(
    _: &reqwest::Client,
    animal: &Animal,
    shard_size: usize,
) -> Result<String, AppError> {
    match animal {
        // All the fake raw facts should be valid.
        // Invalid raw facts can be fed directly to validators for the sake of testing.
        Animal::Dog => Ok(fake_raw_dog_facts(shard_size)),
        Animal::Cat => Ok(fake_raw_cat_facts(shard_size)),
    }
}

#[derive(Deserialize, Debug)]
#[cfg_attr(test, derive(Serialize))]
struct DogFactBatch {
    facts: Vec<String>,
    success: bool,
}

fn validate_dog_facts(body: String, shard_size: usize) -> Result<Shard, AppError> {
    match serde_json::from_str::<DogFactBatch>(&body) {
        Ok(batch) => {
            if !batch.success {
                return Err(AppError::InvalidData(
                    "The 'success' flag is false".to_string(),
                ));
            } else if batch.facts.len() != shard_size {
                return Err(AppError::InvalidData(format!(
                    "Unexpected number of dog facts received: {} instead of {}",
                    batch.facts.len(),
                    shard_size
                )));
            }
            Ok(Shard::new(batch.facts))
        }
        Err(e) => Err(AppError::JsonParsingError(e)),
    }
}

#[cfg(test)]
fn fake_raw_dog_facts(shard_size: usize) -> String {
    let batch = DogFactBatch {
        facts: vec!["a dog fact".to_string(); shard_size],
        success: true,
    };
    serde_json::to_string(&batch).unwrap()
}

// Optional fields are omitted; some irrelevant ones could have been
// check more thoroughly (e.g. the `updatedAt` isn't just a string),
// but it doesn't seem usefull.
#[derive(Deserialize, Debug)]
#[cfg_attr(test, derive(Serialize, Clone))]
#[allow(non_snake_case, dead_code)]
struct CatFact {
    _id: String,
    text: String,
    updatedAt: String,
    deleted: bool,
}

fn validate_cat_facts(body: String, shard_size: usize) -> Result<Shard, AppError> {
    match serde_json::from_str::<Vec<CatFact>>(&body) {
        Ok(batch) => {
            if batch.len() != shard_size {
                return Err(AppError::InvalidData(format!(
                    "Unexpected number of cat facts received: {} instead of {}",
                    batch.len(),
                    shard_size
                )));
            }
            // One may exclude some facts (e.g. from untrustworthy authors) here.
            Ok(Shard::new(batch.into_iter().map(|f| f.text).collect()))
        }
        Err(e) => Err(AppError::JsonParsingError(e)),
    }
}

#[cfg(test)]
fn fake_raw_cat_facts(shard_size: usize) -> String {
    let fact = CatFact {
        _id: "fake it".into(),
        text: "a cat fact".into(),
        updatedAt: "timestamp".into(),
        deleted: false,
    };
    let batch = vec![fact; shard_size];
    serde_json::to_string(&batch).unwrap()
}
