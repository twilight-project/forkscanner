# SPECS for ForkScanner

Responsibility of this Fork monitor will be to detect forks in BTC chain as well and check for
double spends. Please ensure that following requirements are met.

## Requirements:

- The system will run and maintain at least 3 BTC nodes with different versions.^
    - Create sh files to run nodes.^
- Each node maintained by the system will also have a mirror node.^
- System should also be able to connect with other nodes randomly. (not the ones managed by
    the system)
- A SQL DB shall be maintained (Schema diagram shared with this document).^
- An API to add and retrieve data (details below)^
- System shall have a service for bootstrapping. (Details below)^
- System will have a service for fork validation. (Explained below)^
- System will have a service for double spend detection. (Explained below)^

## Block

A block is a bitcoin core block obtained from a full node

**Struct**

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

**Height:**

**Headers-Only:**


## ChainTip

A Chaintip is a bitcoin core chaintip object returned by calling the getchaintip Json-RPC call.

**Struct**

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

**Struct**

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
string “rpchost"

## Stale Candidate.

This is to store blocks which have a probability to go stale. Also we use this to detect double
spent and replace by fee transactions. The Schema is explained below.

**Struct**

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

## Stale Candidate Children.

This model is used to maintain the child of stale candidate, we don not store the children rather
the root of the chain tip (which would be the id of the corresponding block) branchlen and the
corresponding chain tip id, the schema is defined below,

**Struct**

bigint "stale_candidate_id"
bigint "root_id"
bigint "tip_id"
integer "length"
datetime "created_at"
datetime "updated_at"

## Invalid Block:

This model is to keep a record of the blocks which have been marked invalid by a node. The
schema for this is mentioned below.


**Struct**

bigint "block_id"
bigint "node_id"
datetime "notified_at"
datetime "created_at", null: false
datetime "updated_at", null: false
datetime “dismissed_at"

## Transaction:

This model is used to save the transactions for every block. Please refer to the schema below

**Struct**

bigint "block_id"
string "tx_id"
boolean "is_coinbase"
datetime "created_at"
datetime "updated_at"
decimal "amount"
binary "raw"

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

**Tip height/Chain tip height:**
Tip height is the height of the block at the tip of a chain.

**Service**
The service will perform the following tasks.

- First check any old inactive and unreachable chain tips in the DB and remove them.
- Then Query all nodes using the getchaintip RPC call.
- Run a loop on chain tips.
    - Get the latest tip block for the chaintip (Hence forth mentioned as the block )^
    - The chain tip returned by the RPC call will have a status.^


**- Active
- Invalid
- Valid fork
- Valid headers
- Headers-only**
- We process each scenario differently (explained below).^
- After processing the chain tips we match children, check_parent, match_parent for each
node and chain tip combination. (All 3 processes are explained below)
- After creating block entries we check for invalid blocks (explained below)
- The last step in this service is to detect the blocks which have the tendency to go stale.

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
If we don't already have a parent, check if any of the other nodes are ahead of us. Use their chain
tip instead unless we consider it invalid
- Steps are same as match children except we replace Update candidates’ parent chain tip to self
by Update parent chain tip to candidate tip

**Check Invalid Blocks:**
In this part we check the blocks and see if they are marked invalid by a node. If so we add that in
the invalid blocks table (Schema Shared above).

- Retrieve blocks from the DB where the block is marked valid and in valid by at least 1 node.
- Run a loop on the retrieved blocks
    - Get the first node which marked the block invalid.^
    - Create an entry in the invalid_block table for this block.^

**Check for Stale Candidates:**
After populating the DB with the blocks from the nodes and maintaining the parent child
relationships we determine the blocks which have a tendency to become stale blocks. We will call
them stale candidates.

- Tip height is the the max height block in DB.
- Return if tip height is null.
- Get blocks where there are more than one block at the same height.
- Run a loop on the blocks we get.
    - Continue if there are more than 1 blocks at the previous height.^
    - Continue loop if we have that block in invalid_block table.^
    - Find or create an entry in DB for stale candidate.^
    - Also create entries for stale candidate children.^

## Validating Forks and Rollbacks

This service validates folks and makes roll backs where needed. This service is highly dependent
on the mirror nodes and the following Json RPC calls.

**rpc::submitblock
rpc::InvalidateBlock
rpc::Getblockfrompeer**


**rpc::ReconsiderBlock**

**Service:**
This service will perform the following tasks.

- Find missing blocks by querying mirror nodes and other regular nodes.
- Chaintip level Validate fork and roll back. Run this on all nodes from the DB.

**Missing Blocks:**
Missing blocks are referred as full-block data missing from **rpc::getchaintips.** where status is
Headers only/ valid-headers status.

**Find Missing Blocks:**

- Get the maximum height block from the DB.
- Then retrieve headers from the DB. (Filter by height > chain tip height - max depth). According
    to our DB schema headers will be blocks with field headers-only set to true.
- If we have no new headers we return null.
- Otherwise if we have headers available we run a loop on all headers and perform below tasks
    - Query DB to get regular nodes we are connected with. Let’s call this list connected_nodes.^
    - We see which node picked up the headers first. We can use the ```first_seen_by``` field in the
       Block table for this. Let’s call it original node.
    - If the headers is older than 10 blocks. No need to find older blocks. (Getting the 10 blocks
       cap is inspired from fork monitor, need to check why)
    - Run a loop on connected_nodes.^
       - Query node for block via json-RPC call.^
       - If block is found.^
          - Get raw block data from the node as well.^
          - Update block in DB and set header-only to false.^
          - Break the loop on connected nodes.^
       - If block is not found. Continue the connected node loop.^
    - If we have the block data for block headers we send it to the original node using
       **rpc::SubmitBlock** and continue the loop on headers.
    - If we don’t find the block from the nodes we are connected with, we query peers of our
       connected nodes for this block.
    - First we get the blockheaders from the mirror node. (Using mirror nodes so regular nodes
       are not )
    - If mirror node does not have the block headers yet. (multiple reasons for this.) we get it from
       the regular node and then submit them to the mirror node using **rpc::Submitblockheaders.**
    - Find peers of the mirror node using **rpc::getpeerinfo.**^
    - Create a list of blocks we need to retrieve from mirror. And add all blocks(not found in the
       connected list above.) in to the list
    - End block headers loop.^
- At this point we should have updated all the block we found in regular nodes and should have a
    list of blocks we need to query from mirror nodes.
- Run a loop on peers:
    - Get block from peer using **rpc::getfrompeer** RPC call.^
- Run a new loop on the get blocks from mirror nodes list.
    - Query mirror node for block via json-RPC call.^
    - If block is found.^
       - Get raw block data from the mirror node as well.^
       - Update block in DB and set header-only to false.^
       - Break the loop on connected nodes.^
    - If block is not found. Continue the node with next node.^

**Chain tip level Validate fork and rollback:**
Validation of forks and roll backs where applicable and performed as mentioned below.


- We need to run below steps on all the nodes in a loop.^
    - Get chain tips from a node using **rpc::getchaintip**.^
    - Filter the chain tips with active status and pick retrieve the active tip height.^
    - Filter the chain tips with valid headers status and run a loop on them.^
       - Break the loop if current chain tip height > active tip height - max depth.^
       - Retrieve the tip block from DB.^
       - Break the loop if block is not found.^
       - And run Block level validation on it (explained below).^

**Block Level Validation:**

- If the block is marked valid or invalid return null^
- Get block from mirror node.^
- If mirror node does not have the block get it from regular node and submit back to mirror
    node.
- Close all p2p networking on mirror node using **rpc::setnetworkactive.**^
- Run make active on mirror (explained below)^
- Get chain tips from the mirror node.^
- If the active tip height == block height.^
    - Mark the block valid by current node id.^
    - Also call undo rollback (explained below)^
- Else if the block is not at the tip of active chain tip.^
    - Run a loop on invalid chain tips^
       - Call **rpc::reconsiderBlock** on mirror node for every chain tip hash.^
- Check if the block is still considered invalid.^
    - If we get a block for conditions: chain tip is invalid and where tip hash == block hash^
       - Mark the block invalid by current node id.^
- Resume all p2p networking on mirror node using **rpc::setnetworkactive.**^

**Make Active on mirror:**

- Run an infinite loop^
    - Get active chain tips from the mirror node using.^
    - Break the loop if we don’t have any chain tips.^
    - Also break if block hash is equal to the active chain tip hash.^
    - Maintain a counter and if counter goes greater than 100. Call throw unable to rollback and
       break loop.
    - If counter is greater the 0 bootstrap data from mirror node (this will follow same steps as
       mentioned above).
    - Get tip block for the active chain tip from the DB.^
    - Maintain a list of blocks we need to invalidate^
    - If block height == active tip height^
       - Add block to blocks to invalidate list^
    - Now add all the all children to invalidate list.^
    - If the invalidate list is empty call unable to roll back error and break loop^
    - Otherwise run a loop on the list and call **rpc::invalidateblock** on the mirror node. For
       every entry in the blocks to invalidate list
    - Increase counter by 1.^
    - Also maintain a list of the blocks we have invalidated. And every time we invalidate a block
       add it in the invalidated blocks list.

**Undo_rollback:**

- If the invalidated blocks list (from make active on mirror) is empty return null.^
- Run a loop on the list and Call **rpc::reconsiderBlock** on mirror node for each block.^

## Double Spent detection:

This service will check for double spend transactions and the replace by fee transactions. A few
considerations are

- The service will check the last 3 stale candidates.


- Will look for stale blocks in the last 100 blocks (approx. one day)
- Service will check double spend in last 30 blocks.

**Service:**
The service will first pick up the shortest and longest branches then will pick up transactions that
are confirmed in one branch but not in the other. We then query the nodes and get the complete
transaction info from the node, if the input in transaction is the same that means the transaction is
a double spent or RBF. RBF part can be confirmed by checking the pk_script in the transaction
info. If the pk_script is the same it means its a replace by fee transaction and not a double spent.

The algorithm for this service is explained below. Please keep in considerations that the below
algorithm will run for each of the 3 latest stale candidates mentioned above.

- Find transactions and descendants.
- When a new blocks comes in calculate branch length and scan for duplicate transactions.

**Find descendants and transactions.**

- Get all blocks at the same height as the stale candidate.^
- For each of the blocks we retrieved above fetch transactions.^
    - If we don’t already have transactions for this block we proceed as below. Keep in mind
       that if the block is marked as headers only we will not have the transactions and in this
       case we don’t have to retrieve the transactions from the node.
          - We check which node saw this block first (using first_seen_by field in schema).^
          - Send a get block json-rpc call with verbosity set to 2, to retrieve the transaction info for
             this block.
          - Create entry in the transaction table in the DB.^
- Get the descendants off this block and until the double spent range (30 blocks). And use the
    above steps to retrieve the transactions for the descendants as well.

**Scan for duplicate transactions:**

- Set tip height to maximum height.^
- If stale candidate have no children and if the height is in between the stale block window we
    proceed
       - We set children for the stale block (explained below).^
       - Then we set conflicting transactions.^

**Set Stale block’s children:**

- Remove all entries from the stale_candidate children table. (We populate it for each stale
    candidate and then clean it up for the next candidate, bio need to save this data after the
    processing)
- Get the blocks at the same height as the stale candidate from the blocks table. And run a loop
    on them
       - For each block we generate a chain of descendants and order them by work.^
       - We create an entry in the stale_candidate_children table. Please note that the
          stale_candidate children table does not store the actual block it just keeps track of the
          following info
             - Root (the block where the fork started. Stale candidate block in this case).^
             - Tip (latest block in this branch)^
             - Branch length^

**Setting Conflicting transactions:**

- Get transactions confirmed in one branch (explained below)
- Sum up total amount spent in one branch
- Get the coins spent with transactions (explained below),
- Filter coins that are spent with a different tx id in the longest chain and get the double spent
    inputs. (explained below)


- Get double spent in one branch, which should be a list of transactions ids for the shortest
    branch retrieved from the get double spent method.
- Get double spent by, which should be a list of transactions ids for the longest branch retrieved
    from the get double spent method.
- Check for replace by fee transactions (RBF)

**Confirmed in one branch:**

- We query the DB and retrieve the longest and shortest entry by the branch length field.
- Ensure that the root for shortest and longest is full block not just headers only entry.
- Also ensure that we have transactions for the root block and the descendants.
- Get the transaction ids for the root and descendants for each branch.
- Return unique transactions for shortest branch which are in shortest but not in longest. If both
    longest and shortest length is the same then return unique for both branches.

**Get spent coin with transactions:**

- We query the DB and retrieve the longest and shortest entry by the branch length field.
- Ensure that the root for shortest and longest is full block not just headers only entry.
- Also ensure that we have transactions for the root block and the descendants.
- Get the transactions for the root and descendants for each branch.
- For each transaction in the list we get above we do the below
    - Query the node with a get data request to retrieve the tx details.^
    - Then for each tx.in object in the tx message. we run a loop and maintain a hash map where
       key is parent_tx_id concatenated with parent_tx_vout and the value is the original
       transaction.
- We should have 2 hash maps one for longest branch and one for shortest. Return both of these.

**Get double spent inputs:**

- Filter the shortest hash map list based on below criteria. These hash maps are returned by the
    Get spent coin with transactions part.
       - That the same key exists in the longest chain hash map but the transaction id is different in
          longest and shortest chain.
- Then return the transpose of unique entries. Should give us two list, one for shortest branch and
    one for longest branch

**Replace by fee:**

- Filter coins that are spent with a different tx in the longest chain
- Run a loop on shortest branch hash map. We will get a key and the transaction for each entry.
- Return false if the same key does not exists in the longest chain hash map or the transaction id
    is same in longest and shortest chain.
- Otherwise pick the transaction with the same key from the longest branch. That transaction will
    be the replacement.
- Sort the outputs with script_pk in the original transaction from shortest branch.
- Do the same with replacement transaction.
- If length for the above 2 script_pk outputs is same continue, otherwise its not an RBF
    transaction.
- Ensure that the pk script for both output transaction lists is the same, this indicates that the
    transaction is RBF.

## DB SCHEMA



