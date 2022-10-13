## Block

A block is a bitcoin core block obtained from a full node

**Struct**
```
string "block_hash"
integer "height"
integer "timestamp"
string "work"
datetime "created_at"
datetime "updated_at"
bigint "parent_id"
integer "mediantime"
bigint "first_seen_by_id"
integer "version"
integer "coin"
string "pool"
integer "tx_count"
integer "size"
boolean "connected"
integer Array "marked_invalid_by"
integer Array "marked_valid_by"
string "coinbase_message"
boolean "headers_only"
```

**Parent_ID:** Blocks will maintain a parent child relationship using the parent_id field in above
struct.

**Block Header:** If we only get a block header from the node this field is marked as True. Otherwise
if we get a complete block it is set to False.

**Height:**

**Headers-Only:**


## ChainTip

A Chaintip is a bitcoin core chaintip object returned by calling the getchaintip Json-RPC call.

**Struct**
```
bigint "node_id"
bigint "block_id"
bigint "parent_chaintip_id"
string "status"
datetime "created_at"
datetime "updated_at"
```

**Parent_ID:** Chaintips will maintain a parent child relationship using the parent_chaintip_id field in
above struct.

**NODE_ID:** Node id is foreign key from the Node struct used to maintain a relation between chain
tips and node it came from.

## Node

A full bitcoin core node that runs the different bitcoin core versions. This struct It keeps a record
of all the Bitcoin Nodes available to Forkscanner for RPC connections.

**Struct**
```
string "name"
integer "version"
bigint "block_id"
datetime "created_at"
datetime "updated_at"
datetime "unreachable_since"
string "rpchost"
string "rpcuser"
string "rpcpassword"
integer "peer_count"
integer "client_type"
integer "rpcport"
string "version_extra"
boolean "enabled"
string "mirror_rpchost"
integer "mirror_rpcport"
bigint "mirror_block_id"
boolean "txindex"
datetime "mirror_rest_until"
datetime "polled_at"
integer "sync_height"
datetime "mirror_unreachable_since"
```

**Mirror Node**

A replica of the current state of the block headers maintained by the local full node of bitcoin core.
The mirror node follows the schema for the Bitcoin Full node. Information about the mirror node is
saved in mirror_* variables in above struct.


## Peers

This manages a list of peers that the Bitcoin Full Nodes are connects to download block info. We
use the below RPC call
**rpc::getpeerinfo**

**Struct**
```
string "name"
integer "version"
datetime "created_at"
datetime "updated_at"
string “rpchost"
```

## Stale Candidate.

This is to store blocks which have a probability to go stale. Also we use this to detect double
spent and replace by fee transactions. The Schema is explained below.

**Struct**
```
integer "height"
datetime "notified_at"
datetime "created_at"
datetime "updated_at"
integer "coin"
string "confirmed_in_one_branch"
decimal "confirmed_in_one_branch_total"
string "double_spent_in_one_branch"
decimal "double_spent_in_one_branch_total"
integer "n_children"
string "rbf"
decimal "rbf_total"
integer "height_processed"
string “double_spent_by"
string “rbf_by"
boolean “missing_transactions"
```

## Stale Candidate Children.

This model is used to maintain the child of stale candidate, we don not store the children rather
the root of the chain tip (which would be the id of the corresponding block) branchlen and the
corresponding chain tip id, the schema is defined below,

**Struct**
```
bigint "stale_candidate_id"
bigint "root_id"
bigint "tip_id"
integer "length"
datetime "created_at"
datetime "updated_at"
```

## Invalid Block:

This model is to keep a record of the blocks which have been marked invalid by a node. The
schema for this is mentioned below.


**Struct**
```
bigint "block_id"
bigint "node_id"
datetime "notified_at"
datetime "created_at", null: false
datetime "updated_at", null: false
datetime “dismissed_at"
```

## Transaction:

This model is used to save the transactions for every block. Please refer to the schema below

**Struct**
```
bigint "block_id"
string "tx_id"
boolean "is_coinbase"
datetime "created_at"
datetime "updated_at"
decimal "amount"
binary "raw"
```


## Inflated blocks:

This model is used to save the information for inflated block. Please refer to the schema below

**Struct**
```
bigint "block_id"
decimal "max_inflation",
decimal "actual_inflation
datetime "notified_at"
datetime "created_at", null: false
datetime "updated_at", null: false
bigint "node_id"
datetime "dismissed_at"
```


## Tx outsets:

This model is used to save the transaction's tx outset. Please refer to the schema below

**Struct**
```
string "block_hash"
integer "txouts"
decimal "total_amount”
datetime "created_at", null: false
datetime "updated_at", null: false
bigint "node_id"
boolean "inflated", default: false, null: false
```

## Block Template:

This model is used to saves the block template shared by the bitcoin core to the miner, this template is used by miner to generate a block. Please refer to the schema below

**Struct**
```
bigint "parent_block_id"
bigint "node_id"
decimal "fee_total"
datetime "timestamp"
integer "height"
datetime "created_at",
datetime "updated_at"
integer "n_transactions”
binary "tx_ids"
integer "coin", null: false
integer "tx_fee_rates"
integer "lowest_fee_rate"
```

## Soft Forks :

This model is used to keep a record of all the soft forks. Please refer to the schema below

**Struct**
```
create_table "softforks
integer "coin"
bigint "node_id"
integer "fork_type"
string "name"
integer "bit"
integer "status"
integer "since"
datetime "created_at”
datetime "updated_at"
datetime "notified_at"
```

## Inflated_blocks:
**Struct**
```
bigint "block_id"
decimal "max_inflation",
decimal "actual_inflation
datetime "notified_at"
datetime "created_at", null: false
datetime "updated_at", null: false
bigint "node_id"
datetime "dismissed_at"
```


## Pool:
the bitcoin miner pool info.

**Struct**
```
string "tag"
string "name"
string "url"
datetime "created_at"
datetime "updated_at"
```


 ## lags
 to keep track of bitcoin nodes that are lagging.
 
 **Struct**
 ```
 bigint "node_id"
 datetime "created_at",
 datetime "updated_at",
 datetime “deleted_at ”
 ```
 
 ## watched
 to keep track of address under watch.
 
 **Struct**
 ```
 string address
 datetime created_at
 datetime watch_until
 ```
 
 
 ## Double_spent_by

 To keep track of the transaction with double spents

 **Struct**
 ```
 Bigint candidate_height
 string Tx_id
 ```
 
 
 ## Fee_rates

 To keep track of fee paid to the miners.

 **Struct**
 ```
 string parent_block_hash
 bigint node_id
 int  fee_rate
 bool omitted
 ```
 
 

