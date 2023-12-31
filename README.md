## Intro

My name is Xaverij Florek and it's my solution of the take-home; the task is described [here](https://you.ashbyhq.com/shuttle/assignment/5d83b9d6-eb8e-41c1-9f61-b51ff5a10306).
As it's an exam code, it contains additional documentation concerning some design choices I've made. Normally such questions are discussed either behorhand or during code reviews.


### Server design

A naïve version of the server would work roughly as follows:
- get a client's request;
- retrieve a single animal fact;
- return it.
It's relatively slow, it makes the server especially vulnerable to DDoS-attacks and totally dependent on the providers of animal facts (which are not guaranteed to be constantly available).

Thus, the server should request batches of animal facts routinely and cache them; one fact from the cache is to be chosen for each request. I suppose that there's no need in removing a fact from the cache once it's requested; this is predicated upon some guesses on the (fictitious) purposes of the server:
1. it's designed for users who don't request facts too often (i.e. we don't expect an honest user to exhaust the cache before it's refreshed);
2. it's OK for different people using the server in the same time (or for different threads of the same application) to receive sometimes the same facts (by the way, I'm not sure if the animal fact providers are good at avoiding collisions in this case).

The cache can be implemented in different ways.
1. One can use read-write locks; `tokio::sync::watch` is based on them and seems to fit the task well. Some of the rw-lock-related problems are indicated, for instance, [here](https://docs.rs/tokio/1.32.0/tokio/sync/watch/struct.Receiver.html#method.borrow).
2. One can use `Mutex`. It can easily become a bottle-neck for the server, so sharding is required.
3. One can use one of numerous less classical solutions; e.g. the [left-right](https://lib.rs/crates/left-right) primitive seems suitable for the task (but pay attention to the "Trade-offs" section).

My server implementation uses the second option, sharded `Mutex`.


## Getting started

Build and run server:
```
cd shuttle-test
cargo build
./target/debug/shuttle-test
```

Use `./target/debug/shuttle-test --help` to get command line argument list and `cargo test` to run tests.


### API

`GET /fact`: returns a fact about an animal.
`GET /health`: checks if the server is OK.