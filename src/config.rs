use clap::Parser;
use std::collections::HashSet;
use std::ops::RangeInclusive;
use tracing;

use crate::animals::Animal;

#[derive(Clone, Parser)]
pub struct ServerConfig {
    #[arg(short, long, default_value_t = 3000)]
    pub port: u16,

    /// Number of shards per animal type
    #[arg(long, default_value_t = 1)]
    pub shard_num: usize,

    // For the sake of simplicity shards of facts concerning different animals
    // use the same shard size.
    /// Number of animal facts per shard
    #[arg(long, default_value_t = 50, value_parser = validate_shard_size)]
    pub shard_size: usize,

    // The time of refreshing itself is NOT included
    /// Frequency of shard refreshing (sec)
    #[arg(long, default_value_t = 2)]
    pub shard_refresh_sec: u64,

    /// Maximal normal age of a shard (sec)
    #[arg(long, default_value_t = 10)]
    pub shard_staleness_sec: i64,

    #[arg(short, long, default_value_t = tracing::Level::INFO)]
    pub verbosity: tracing::Level,

    /// Animals you are interested in (comma-separated)
    #[arg(long, value_parser, value_delimiter = ',', default_values_t = vec![Animal::Cat, Animal::Dog])]
    pub animals: Vec<Animal>,
}

// Ideally, this range should have been fetched for APIs of fact providers.
// Alas, it's currently impossible and hardcode is required.
// Technically, 1 is also a valid shard_size, but the cat fact API
// changes the response format if only one fact was requested,
// and exlusion of this number allows not to support an extra output format.
const SHARD_SIZE_RANGE: RangeInclusive<usize> = 2..=100;

fn validate_shard_size(s: &str) -> Result<usize, String> {
    let size: usize = s.parse().map_err(|_| format!("`{s}` isn't a usize"))?;
    if SHARD_SIZE_RANGE.contains(&size) {
        Ok(size)
    } else {
        Err(format!(
            "shard size not in range {}-{}",
            SHARD_SIZE_RANGE.start(),
            SHARD_SIZE_RANGE.end()
        ))
    }
}

impl ServerConfig {
    pub fn deduplicate_animals(&mut self) {
        let mut set = HashSet::new();
        self.animals = self
            .animals
            .iter()
            .filter_map(|a| {
                if set.contains(&a.to_string()) {
                    None
                } else {
                    set.insert(a.to_string());
                    Some(*a)
                }
            })
            .collect();
    }
}
