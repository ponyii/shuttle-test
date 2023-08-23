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

#[derive(Clone, Copy, ValueEnum, Debug)]
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
    let shard = match animal {
        Animal::Dog => validate_dog_facts(body, shard_size)?,
        Animal::Cat => validate_cat_facts(body, shard_size)?,
    };
    validate_shard(shard, animal)
}

// Animal-agnostic fact validation. Almost empty now, but more checks can be added later.
pub fn validate_shard(shard: Shard, animal: &Animal) -> Result<Shard, AppError> {
    if shard.facts.contains(&String::from("")) {
        // Such facts could just have been excluded, but it requires some
        // additional logic concerning minimum shard size and its replenishment.
        // Currently this code just helps to notice empty facts in responses
        // (and it hasn't noticed any such fact yet).
        return Err(AppError::InvalidData(
            format!("An empty {:?} fact received", animal)
        ));
    };
    // It might make sense to exclude too long facts from the batches so as
    // to control the amount of memory used, the fact providers can't be really trusted.
    Ok(shard)
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
        // All the fake raw facts generated here should be valid, as
        // invalid fake raw facts can be fed directly into validators.
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
                    "Upstream API reported error".to_string(),
                ));
            }
            if batch.facts.len() != shard_size {
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

// Irrelevent fields are omitted, checking them doesn't seem useful.
// They can be added later for the sake of fact filtering.
#[derive(Deserialize, Debug)]
#[cfg_attr(test, derive(Serialize, Clone))]
struct CatFact {
    text: String,
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
        text: "a cat fact".into(),
    };
    let batch = vec![fact; shard_size];
    serde_json::to_string(&batch).unwrap()
}
