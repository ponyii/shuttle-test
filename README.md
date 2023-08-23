## Intro

My name is Xaverij Florek and it's my solution of the take-home; the task is described [here](https://you.ashbyhq.com/shuttle/assignment/5d83b9d6-eb8e-41c1-9f61-b51ff5a10306).
As it's an exam code, it contains additional documentation concerning some design choices I've made. Normally such questions are discussed either behorhand or during code reviews.


### A few words on the server design

A naïve version of the server would work roughly as follows:
- get a client's request;
- retrieve a single animal fact;
- return it.
It's relatively slow, it makes the server especially vulnerable to DDoS-attacks and totally dependent on the providers of animal facts (which are not guaranteed to be constantly available).

Thus, the server should request batches of animal facts routinely and cache them; one fact from the cache is to be chosen for each request. I suppose that there's no need in removing a fact from the cache once it's requested; this is predicated upon some guesses on the (fictitious) purposes of the server:
1. it's designed for users who don't request facts too often (i.e. we don't expect an honest user to exhaust the cache before it's refreshed);
2. it's OK for different people using the server in the same time (or for different threads of the same application) to receive sometimes the same facts (by the way, I'm not sure if the animal fact providers are good at avoiding collisions in this case).


## Getting started

Build and run server:
```
cd shuttle-test
cargo build
./target/debug/shuttle-test
```

Command line arguments:
```
$ ./target/debug/shuttle-test --help
Usage: shuttle-test [OPTIONS]

Options:
  -p, --port <PORT>
          [default: 3000]
      --shard-num <SHARD_NUM>
          Number of shards per animal type [default: 1]
      --shard-size <SHARD_SIZE>
          Number of animal facts per shard [default: 50]
      --shard-refresh-sec <SHARD_REFRESH_SEC>
          Frequency of shard refreshing (sec) [default: 2]
  -v, --verbosity <VERBOSITY>
          [default: INFO]
      --animals <ANIMALS>
          Animals you are interested in (comma-separated) [default: cat dog] [possible values: dog, cat]
  -h, --help
          Print help
```