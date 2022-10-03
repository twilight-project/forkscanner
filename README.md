# Setting up

## Postgres

Postgres database is required to run the forkscanner. For a simple setup, run this:

```
CREATE USER forkscanner WITH ENCRYPTED PASSWORD 'forkscanner';
CREATE DATABASE forkscanner;
GRANT ALL PRIVILEGES ON DATABASE forkscanner TO forkscanner;
```

## Insert nodes into node table
setup_nodes.sql provides an example of how to add bitcoin nodes to the list that forkscanner will monitor.
At least one mirror node is necessary. The mirror node will be used to run rollback checks,
which will interrupt the operation of the mirror node p2p.

Edit setup_nodes.sql with credentials and rpc endpoints for your nodes, then run:
```
psql -f setup_nodes.sql postgres://forkscanner:forkscanner@localhost/forkscanner
```

Do the above with user forktester as well to run the tests.

## Install diesel cli-tool
Initialize the database with diesel migrations tool:

`cargo install diesel_cli --no-default-features --features postgres`

`diesel migration run`

## Test program
This needs to be run on a node with bitcoin running.
`cargo run`

## RPC endpoints
```
- `get_tips`: params { active_only: bool }
  Fetch the list of current chaintips, if active_only is set it will be only the active tips.

- `add_node`: params { name: string, rpc_host: string, rpc_port: int, mirror_rpc_port: int, user: string, pass: string }
  Add a node to forkscanner's list of nodes to query.

- `remove_node`: { id: int }
  Removes a node from forkscanner's list.

- `get_block`: params { hash: string } OR { height: int } 
  Get a block by hash or height.

- `submit_block`: params { block: block_json, node: int }
  Upload a block to the given node.

- `get_block_from_peer`: params { node_id: int, hash: string, peer_id: int } 
  Fetch a block from a specified node.

- `tx_is_active`: params: { id: string }
  Query whether transaction is in active branch.

- `get_peers`: params: { "id": 8 }
   Query a nodes active peer list.

- `update_watched_addresses`: params: { "remove": [ string ], "add": [ (string, date) ] }
   Query a nodes active peer list.
```

### WS notification endpoints
Example usage of these endpoints can be found in `./scripts/subscribe-test`:

- `validation_checks`: subscribe to this to get difference info between active tip and stale blocks.
- `subscribe_forks`: subscribe to this to get notifications of a new fork.
- `invalid_block_checks`: subscribe to this to get notifications of invalid blocks.
- `lagging_nodes_checks`: subscribe to this to get notifications of lagging nodes.


### POST examples:
`get_tips`:

POST 
```json
    {"method": "get_tips", "params": { "active_only": false }, "jsonrpc": "2.0", "id": 1}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [
    {
      "block": "0000000000000000000328ba3e72951addfc7dae27aca112daf3a8de4553430e",
      "height": 743576,
      "id": 4838,
      "node": 14,
      "parent_chaintip": null,
      "status": "active"
    },
    {
      "block": "0000000000000000000328ba3e72951addfc7dae27aca112daf3a8de4553430e",
      "height": 743576,
      "id": 4839,
      "node": 15,
      "parent_chaintip": null,
      "status": "active"
    },
    {
      "block": "00000000000000000006ead1cff09f279f7beb31a7290c2a603b0776d98dc334",
      "height": 733430,
      "id": 5203,
      "node": 15,
      "parent_chaintip": null,
      "status": "valid-fork"
    }
  ],
  "id": 1
}

```

`get_block`:

POST
```json
    {"method": "get_block", "params": { "hash": "00000000000000000006ead1cff09f279f7beb31a7290c2a603b0776d98dc334" }, "jsonrpc": "2.0", "id" 1}
```

or by block height:
```json
    {"method": "get_block", "params": { "height": 733430 }, "jsonrpc": "2.0", "id" 1}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [
    {
      "coinbase_message": [ 123, 45, 67 ],
      "connected": true,
      "first_seen_by": 13,
      "hash": "00000000000000000006ead1cff09f279f7beb31a7290c2a603b0776d98dc334",
      "headers_only": false,
      "height": 733430,
      "lowest_template_fee_rate": null,
      "parent_hash": "000000000000000000082af6a6db0e71d72f25dcfb513aeda1a1cb4044253030",
      "pool_name": "Foundry USA",
      "template_txs_fee_diff": null,
      "total_fee": "0.09797872",
      "tx_omitted_fee_rates": null,
      "txids": [
        "d6187e533fffece5c502e8a05242dba6e94a7eb9cdde241250f3ed16c31242eb",
        "b76c3a88d50ff3b8a03fc623098f86d6872b3748d6cce956138fef8fa6f6c412",
        "......"
      ],
      "txids_added": null,
      "txids_omitted": null,
      "work": "00000000000000000000000000000000000000002ca1bca6e028e261a6019f07"
    }
  ],
  "id": 1
}

`get_block_from_peer`:

POST
```json
    {"method": "get_block_from_peer", "params": { "node_id": 14, "hash": "000000000000000000082af6a6db0e71d72f25dcfb513aeda1a1cb4044253030", "peer_id": 23454 }, "jsonrpc": "2.0", "id" 1}
```

Response is a block similar to `get_block` response.

`add_node`:

POST
```json
  {"name": "node_name", "rpc_host": "hostname", "rpc_port": 1234, "mirror_rpc_port": null, "user": "username", "pass": "pass", "archive": false}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [ "id": 14 ],
  "id": 1
}
```

`remove_node`:

POST
```json
  { "id": 14 }
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [ "OK" ],
  "id": 1
}
```

`tx_is_active`:

POST
```json
  { "id": "tx_hash" }
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [ true ],
  "id": 1
}
```

`get_peers`:

POST
```json
  { "id": 8 }
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [
      { "id": "node_id", "address": "127.0.0.1" },
  ],
  "id": 1
}
```

`update_watched_addresses`:

POST
```json
  { "remove": ["cdef9ae998abe7d1c287d741ab9007de848294c0"], "add": [] }
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [
    "OK"
  ],
  "id": 1
}
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": [
      { "id": "node_id", "address": "127.0.0.1" },
  ],
  "id": 1
}
```

`submit_block`:

POST
```json
  { 
    node_id: 15,
    block: {
      "coinbase_message": [ 123, 45, 67 ],
      "connected": true,
      "first_seen_by": 13,
      "hash": "00000000000000000006ead1cff09f279f7beb31a7290c2a603b0776d98dc334",
      "headers_only": false,
      "height": 733430,
      "lowest_template_fee_rate": null,
      "parent_hash": "000000000000000000082af6a6db0e71d72f25dcfb513aeda1a1cb4044253030",
      "pool_name": "Foundry USA",
      "template_txs_fee_diff": null,
      "total_fee": "0.09797872",
      "tx_omitted_fee_rates": null,
      "txids": [
        "d6187e533fffece5c502e8a05242dba6e94a7eb9cdde241250f3ed16c31242eb",
        "b76c3a88d50ff3b8a03fc623098f86d6872b3748d6cce956138fef8fa6f6c412",
        "...snip..."
      ],
      "txids_added": null,
      "txids_omitted": null,
      "work": "00000000000000000000000000000000000000002ca1bca6e028e261a6019f07"
    }
  }
````


Response:
```json
{
  "jsonrpc": "2.0",
  "result": [
    "OK"
  ],
  "id": 1
}
```
