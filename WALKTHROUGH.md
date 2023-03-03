# Forkscanner walk-through

## Requirements
- a laptop
- git
- rust
- docker, docker-compose
- nodejs

## Components

### postgres
Storage of block data, transaction info, bitcoin node connection info, etc. 
See `migrations/` for the schema

### scanner
The main component. This runs periodically, fetching chaintip information from bitcoin nodes specified in the database. Chaintips will be tracked in the database, tracking which node has the latest view of the chain, tracking expected transactions, etc. This will also watch for transactions on specified addresses, if requested.

### RPC/websocket
Runs on top of the scanner, when scanner detects various conditions (e.g. chaintip updates, nodes are lagging behind) it will notify websocket subscribers.

## Set up
- make sure you have rust installed, with diesel_cli
- get rust here: https://rustup.rs
- Install diesel cli tool:

```console
cargo install diesel_cli --no-default-features --features postgres
```

## Get it running
```console
git clone https://github.com/twilight-project/forkscanner.git
cd forkscanner
docker-compose up -d postgres
```

Run the migrations:

```console
diesel migration run
psql postgres://forkscanner:forkscanner@localhost:5432/forkscanner -f scripts/setup.sql
```

```console
docker-compose up -d forkscanner
```

This should bring up postgres, and then the scanner.

## Connect to service via js
```console
cd forkscanner/scripts/subscribe-test
node test.js
```

You'll see output like this:
```console
Starting
Sending request
Subscription id:  250660754853743700
Subscription id:  14759835894397469000
Subscription id:  6629232225781554000
Subscription id:  10457110670220192000
Subscription id:  8034008713957604000
Got forks method: []
Subscription id:  10962735622574234000
```