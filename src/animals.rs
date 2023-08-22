// This module contains the code requesting facts about different animals,
// validating the responses, etc.

use serde::Deserialize;

use crate::errors::AppError;
use crate::{Shard, SHARD_SIZE};

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

pub fn url(animal: &Animal) -> String {
    match animal {
        Animal::Dog => format!(
            "https://dog-api.kinduff.com/api/facts?number={}",
            SHARD_SIZE
        ),
        Animal::Cat => format!(
            "https://cat-fact.herokuapp.com/facts/random?type=cat&amount={}",
            SHARD_SIZE
        ),
    }
}

pub fn validate_batch(body: String, animal: &Animal) -> Result<Shard, AppError> {
    match animal {
        Animal::Dog => validate_dog_facts(body),
        Animal::Cat => validate_cat_facts(body),
    }
}

#[derive(Deserialize, Debug)]
struct DogFactBatch {
    facts: Vec<String>,
    success: bool,
}

fn validate_dog_facts(body: String) -> Result<Shard, AppError> {
    match serde_json::from_str::<DogFactBatch>(&body) {
        Ok(batch) => {
            if !batch.success {
                return Err(AppError::InvalidData(
                    "The 'success' flag is false".to_string(),
                ));
            } else if batch.facts.len() != SHARD_SIZE {
                return Err(AppError::InvalidData(format!(
                    "Unexpected number of dog facts received: {} instead of {}",
                    batch.facts.len(),
                    SHARD_SIZE
                )));
            }
            // One may add other data checks here. E.g. it might make sense to exclude
            // too long facts from the batch so as to control the amount of memory
            // used per batch, especially if the fact source can't be trusted.
            Ok(Shard::new(batch.facts))
        }
        Err(e) => Err(AppError::JsonParsingError(e)),
    }
}

// Optional fields are omitted; some irrelevant ones could have been
// check more thoroughly (e.g. the `updatedAt` isn't just a string),
// but it doesn't seem usefull.
#[derive(Deserialize, Debug)]
#[allow(non_snake_case, dead_code)]
struct CatFact {
    _id: String,
    __v: u128,
    text: String,
    updatedAt: String,
    deleted: bool,
}

fn validate_cat_facts(body: String) -> Result<Shard, AppError> {
    match serde_json::from_str::<Vec<CatFact>>(&body) {
        Ok(batch) => {
            if batch.len() != SHARD_SIZE {
                return Err(AppError::InvalidData(format!(
                    "Unexpected number of cat facts received: {} instead of {}",
                    batch.len(),
                    SHARD_SIZE
                )));
            }
            // One may add other data checks here.
            Ok(Shard::new(batch.into_iter().map(|f| f.text).collect()))
        }
        Err(e) => Err(AppError::JsonParsingError(e)),
    }
}
