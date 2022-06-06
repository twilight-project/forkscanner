# Setting up

## Postgres
```
CREATE USER forkscanner WITH ENCRYPTED PASSWORD 'forkscanner';
CREATE DATABASE forkscanner;
GRANT ALL PRIVILEGES ON DATABASE forkscanner TO forkscanner;
```

## Insert nodes into node table
Edit setup_nodes.sql with credentials and rpc endpoints for your nodes, then run:
```
psql -f setup_nodes.sql postgres://forkscanner:forkscanner@localhost/forkscanner
```

Do the above with user forktester as well to run the tests.

## Install diesel cli-tool
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

- `get_block_from_peer`: params { node_id: int, hash: string, peer_id: int } 
  Fetch a block from a specified node.

- `tx_is_active`: params: { id: string }
  Query whether transaction is in active branch.
```

### WS notification endpoints
- `validation_checks`: subscribe to this to get difference info between active tip and stale blocks.
- `forks`: subscribe to this to get notifications of a new fork.

### POST example:
`get_tips`: POST '{"method": "get_tips", "params": { "active_only": false }, "jsonrpc": "2.0", "id" 1}'
`add_node`: POST '{"method": "add_node", "params": { "name": "east-us", "rpc_host": "123.4.4.1", "rpc_port": 8333, "mirror_rpc_port": 8334, "user": "btc_user", "pass": "my-pass" }, "jsonrpc": "2.0", "id" 1}'
`get_block`: POST '{"method": "get_block", "params": { "hash": "F000DEAADEDEDABC345124" }, "jsonrpc": "2.0", "id" 1}'
`get_block`: POST '{"method": "get_block", "params": { "height": 1234 }, "jsonrpc": "2.0", "id" 1}'
