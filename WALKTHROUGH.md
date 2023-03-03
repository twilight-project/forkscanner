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
git checkout scanner-walkthrough
docker-compose up -d postgres
```

Run the migrations:

```console
diesel migration run
psql postgres://forkscanner:forkscanner@localhost:5432/forkscanner -f scripts/setup.sql
```

## setup.sql contents
```sql
 (node, rpc_host, rpc_port, rpc_user, rpc_pass, archive, mirror_host, mirror_rpc_port)
 ```
 
 - node a string name for the node
 - rpc_host hostname/IP
 - rpc_port rpc port
 - rpc_user username for btc node
 - rpc_pass password
 - archive boolean, false indicates it's a pruned node
 - mirror_host optional, used for rollback checks
 - mirror_port port number for mirror node

```console
docker-compose up -d scanner
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
