# SPECS for ForkMonitor

Responsibility of this Fork monitor will be to detect forks in BTC chain. Please ensure that
following requirements are met.

## Requirements:

- The system will run and maintain at least 3 BTC nodes with different versions.^
    - Create sh files to run nodes.^
- Each node maintained by the system will also have a mirror node.^
- System should also be able to connect with other nodes randomly. (not the ones managed by
    the system)
- A SQL DB shall be maintained (Schema diagram shared with this document).^
- An API to add and retrieve data (details below)^
- System shall have a service for bootstrapping. (Details below)^

## Block

A block is a bitcoin core block obtained from a full node

## Struct

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

**Parent_ID:** Blocks will maintain a parent child relationship using the parent_id field in above
struct.

**Block Header:** If we only get a block header from the node this field is marked as True. Otherwise
if we get a complete block it is set to False.

## ChainTip

A Chaintip is a bitcoin core chaintip object returned by calling the getchaintip Json-RPC call.


### Struct

bigint "node_id"
bigint "block_id"
bigint "parent_chaintip_id"
string "status"
datetime "created_at"
datetime "updated_at"

**Parent_ID:** Chaintips will maintain a parent child relationship using the parent_chaintip_id field in
above struct.

**NODE_ID:** Node id is foreign key from the Node struct used to maintain a relation between chain
tips and node it came from.

## Node

A full bitcoin core node that runs the different bitcoin core versions. This struct It keeps a record
of all the Bitcoin Nodes available to Forkscanner for RPC connections.

### Struct

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

**Mirror Node**

A replica of the current state of the block headers maintained by the local full node of bitcoin core.
The mirror node follows the schema for the Bitcoin Full node. Information about the mirror node is
saved in mirror_* variables in above struct.

## Peer Info


This manages a list of peers that the Bitcoin Full Nodes are connects to download block info. We
use the below RPC call
**rpc::getpeerinfo**

**Struct**
string "name"
integer "version"
datetime "created_at"
datetime "updated_at"
string "rpchost"
integer_array = ["node_id's connected to this peer"]

## API Spec

Following API endpoints are needed.

- An API end point to add/delete/update new nodes info in the DB.^
- An API end point to get a list of blocks from DB, or get by hash or height.^
- An API endpoint to get the latest chain tip form DB.^

Data must be passed as request body not as query string. The data to be sent in the API request
is the same as the Node and block struct shared above.

**Auth**
Node API endpoint needs authentication. For that we can either decide to create and maintain a
user table or just use a JWT token. That is on the developer to decide

## Bootstrapping Forkscanner

For Bootstrapping we will create a service that will query the nodes and update the DB. For this
we use the **rpc::getchaintips** Json-Rpc call. We query all nodes to share their latest chain tips
and we save those in the DB

**Service**
The service will perform the following tasks.

- First check any old inactive and unreachable chain tips in the DB and remove them.
- Then Query all nodes using the getchaintip RPC call.
- Run a loop on chain tips.
    - Get the latest tip block for the chaintip (Hence forth mentioned as the block )^
    - The chain tip returned by the RPC call will have a status.^
       **- Active**
       **- Invalid**
       **- Valid fork**
       **- Valid headers**
       **- Headers-only**
       - We process each scenario differently (explained below).^
    - After processing the chain tips we match children, check_parent, match_parent for each
       node and chain tip combination. (All 3 processes are explained below)

**Active:**

- update the mark_valid_by field in the block table by the node id of the chaintip object.^
- Find or create the chain tip in DB.^
- If chain tip exists and the latest tip block is not equal to the block we got earlier.^
    - Update the tip block^
    - set parent_chaintip_id to null^
    - set each childs parent_chaintip_id to null^


**Valid-forks:**

- Find or create a the block and its ancestors.^
    - Get the block by hash form DB.^
    - If we don’t find the block^
       - Get the block from node and add in the DB^
    - Once we have the block find its ancestors.^
       - We run a loop and break if a certain height is achieved.^
          - Get block from DB^
          - Set parent = current block’s parent^
          - If parent is null^
             - Get current block from node or just block headers from node if block is not found.^
             - Set parent = as currents block’s previous block hash.^
             - Update current blocks parent_id DB with parent.^
          - If we have a parent in DB^
             - Break if parent is connected.^
          - Else^
             - We get the parent block from Node^
          - Update parent by this new block and add the parent block in DB. And set mark_valid
             field.
          - Update current’s block’s parent_id by parent.^
          - Set block as the parent and re run the loop for parent to find ancestors.^

**Invalid:**

**-** its the same as valid-forks except that we set the mark_invalid field.^

**Valid-headers/Headers-only:**

- Return if the block height is less then minimum height.^
- Return if block is present in DB^
- Otherwise create an entry in DB and mark the headers-only field as True.^

**Match Children:**
Check if any of the other nodes are behind us. If they don't have a parent,
mark us their parent chaintip, unless they consider us invalid.

- Retrieve candidate-tips by joining chain tip, block and node table where chain tip status is
    active, parent_chaintip is null. And height is greater than minimum height.
- Run a loop on all candidate tips.^
    - Set parent = block^
    - Run another loop while parent is present and parent.height > candidate_tip.height.^
       - Break if we find the chain tip in DB marked as invalid^
       - if Parent == candidate.block.^
          - Update candidates’ parent chain tip to self.and break^
       - Parent = parent’s parent^
       - End of inner loop.^
    - Break if candidate has parent chain tip^

**Check Parent:**
If chaintip has a parent, find all invalid chaintips above it, traverse down to see if it descends from
us. If so, disconnect parent.

- If parent chain tip is present.^
    - Retrieve candidate-tips by joining chain tip and block table where chain tip status is invalid
       And height is greater than minimum height.
    - Run a loop on candidate tips.^
       - Parent = candidate-tip.block^
       - Run another loop while parent is present and parent height > block height^
          - If parent == block^
             - Update parent_chaintip to null and break^
          - Parent = parent.parent^


**Match Parent:**
If we don't already have a parent, check if any of the other nodes are ahead of us. Use their
chaintip instead unless we consider it invalid

- Steps are same as match children except we replace Update candidates’ parent chain tip to self
    by Update parent chain tip to candidate tip

## Detecting Fork

Pending

## DB SCHEMA
https://github.com/twilight-project/forkscanner/blob/spec/spec/img/DB_schema.png