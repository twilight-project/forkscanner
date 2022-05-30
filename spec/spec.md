 PDF To Markdown Converter
Debug View
Result View
specs_v2
# SPECS for ForkScanner

Responsibility of this Fork Scanner will be to detect forks in BTC chain as well and check for
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


## Inflated blocks:

This model is used to save the information for inflated block. Please refer to the schema below

**Struct**

bigint "block_id"
decimal "max_inflation",
decimal "actual_inflation
datetime "notified_at"
datetime "created_at", null: false
datetime "updated_at", null: false
bigint "node_id"
datetime "dismissed_at"



## Tx outsets:

This model is used to save the transaction's tx outset. Please refer to the schema below

**Struct**

bigint "block_id"
integer "txouts"
decimal "total_amount”
datetime "created_at", null: false
datetime "updated_at", null: false
bigint "node_id"
boolean "inflated", default: false, null: false


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
    - Also create entries for stale candidate children. Pick up blocks (children of stale candidate)
       from the blocks table and add an entry in stale candidate table.

## Validating Forks and Rollbacks

This service validates folks and makes roll backs where needed. This service is highly dependent
on the mirror nodes and the following Json RPC calls.

**rpc::submitblock
rpc::InvalidateBlock**


**rpc::Getblockfrompeer
rpc::ReconsiderBlock**

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
    - Else we traverse back and find the branch starting point (explained below) we pass the
       current block in this method.
    - Add all the all children to invalidate list.^
    - If the invalidate list is empty call unable to roll back error and break loop^
    - Otherwise run a loop on the list and call **rpc::invalidateblock** on the mirror node. For
       every entry in the blocks to invalidate list
    - Increase counter by 1.^
    - Also maintain a list of the blocks we have invalidated. And every time we invalidate a block
       add it in the invalidated blocks list.

**Undo_rollback:**

- If the invalidated blocks list (from make active on mirror) is empty return null.^
- Run a loop on the list and Call **rpc::reconsiderBlock** on mirror node for each block.^

**Get branch start:**

- Lets call the block passed into this method original block and the block we get in the loop
    current block


- Run an infinite loop starting from tip height block.^
- Return current block, if original block is a descendant current block^
- Else set current block == parent of current block^
- Run the loop until we find the ancestor of the original block or if there are no more blocks^

## Double Spent detection:

This service will check for double spend transactions and the replace by fee transactions. A few
considerations are

- The service will check the last 3 stale candidates.
- Will look for stale blocks in the last 100 blocks (approx. one day)
- Service will check double spend in last 30 blocks. Which means we check for 30 blocks before
    the stale candidate block. So we check for stale blocks in last 100 blocks and once we find a
    stale block we look for double spent in 30 blocks after that.

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



**Specification For Inflation Checks**

This part of the specs monitors inflation and keeps a track of miner rewards for each block mined. this should update our inflated blocks table and should add the blocks where inflation happened. This feature will be implemented in 2 steps mentioned below.

\1. A service which will pick up all nodes which are available and are up to date.
\2. Running inflation checks on each node independently, preferably in threads.

we will also need to add 2 more data models. Schema for which will be shared later in the document

\1. Inflated Blocks.
\2. Tx\_outsets.



**Service to get a list of nodes.**


` `We are maintaining a list of nodes we pick up all nodes from there and run a loop with below mentioned specifications.

1. First step Is to check if the node is currently available and is reachable. Following below steps, we can manage a list of nodes which are currently active and from time to time check up on inactive nodes to update our list.
   1. We maintain an unreachable\_since field (datetime type) in the Nodes table in DB.
   1. If this field is null or the value is more then 10 min ago. we query the node with a getbestblockhash RPC call.
   1. If we get a result back, we unpdate the DB and set unreachable\_since to null
   1. If we get a timeout, we set the current datetime as the value for unreachable\_since.
1. Then we filter the nodes by excluding any node which is in initial block download phase
   1. This can be achieved by creating a bool field in nodes table, named IBD.
   1. Than we compare getblockchaininfo RPC from this node and another fully running
   1. If the difference in block height is more then 10 we can mark the node as IBD.
1. Then we run a new thread for each node and call check\_inflation\_for\_node (explained below) method





**Check Inflation on each Node.**

This method is where we check inflation on each node, please note that it is supposed to be run as a separate thread for each node to ensure real-time calculations. This method also relies on gettxoutsetinfo RPC call.  

1. First query the DB and check if we have tx\_outsets for latest block and this Node.
1. If we have the tx\_outsets, it means node does not have the latest block yet or we already have done the calculation for latest block. Either way we need to wait for next block. So, we can sleep for some time and exit the thread.
1. If we don’t have the tx\_outsets. We start by stopping p2p networking on the node.
1. We need to maintain a list of blocks we need to check. Let’s refer to it as blocks\_to\_check. Add the current block in this list.
1. Run an infinite loop.
   1. Add a check to only go to a depth of 10 blocks to calculate inflation. If it exceeds 10 blocks break the loop
   1. We need the current block and parent block on each loop iteration. Let refer to them as current\_block and parent\_block.
   1. If we cannot find the parent block throw an error stating that unable to calculate inflation due to missing blocks.
   1. Look for tx\_outsets for the parent block
   1. If we have the tx\_outsets for parent block. Break the loop otherwise add the parent block in the blocks\_to\_check list. Please make sure that the new blocks are added at the start to maintain an order.
1. When the loop ends, we should have a list of blocks we need to check. That should not be more then 10 blocks. Going above that would be costly hence just 10 blocks should be enough.
1. Run another loop on the blocks to check list.
   1. Get the UTXO balance at the height by using RPC call gettxoutsetinfo.
   1. Perform the undo\_rollback. The process is explained previously.
   1. Make sure we have the right tx\_outset by comparing tx\_outset [ blockhash ] and best block hash.
   1. Add this new tx\_outset into the DB according to the DB schema for this table.
   1. Retrieve the tx\_outsets from DB for the parent block.
   1. Throw exception if parent block not found.
   1. Now inflation can be calculated as. Lets call it current\_inflation

      inflation = current\_block.total\_amount – parent\_block.total\_amount
   1. Now we calculate max inflation.
      1. interval = current\_height / 210,000 (because after every 210,000 blocks miner reward changes)
      1. reward = 50 \* 100,000,000 (to get satoshies)
      1. max\_inflation = reward >> interval (divide by 2 every time the miner reward changes to get the current reward)

1. now we compare max inflation. if current\_inflation < max\_inflation. Continue to next iteration of loop. Also please make sure that both current and max inflation have the same unit of measure.
1. Otherwise add the current\_block in the inflated blocks table. 
1. Turn the p2p networking back on.


**Schema for DB tables.**

**Inflated\_blocks:**

**bigint "block\_id"**

`    	`**decimal "max\_inflation",**

`    	`**decimal "actual\_inflation**

`    	`**datetime "notified\_at"**

`    	`**datetime "created\_at", null: false**

`    	`**datetime "updated\_at", null: false**

`    	`**bigint "node\_id"**

`    	`**datetime "dismissed\_at"

tx\_outsets:**

`	`**bigint "block\_id"**

`   	`**integer "txouts"**

`    	`**decimal "total\_amount”**

`    	`**datetime "created\_at", null: false**

`    	`**datetime "updated\_at", null: false**

`    	`**bigint "node\_id"**

`    	`**boolean "inflated", default: false, null: false**







**Specification For Fee Calculation**

This part of the fork scanner will investigate which miner pool mined a block and how much fee was charged. It’s a 2 step process.

1. Update the pool table in DB
1. Perform calculation and update blocks in DB.



**Update the pool table in DB.**

We will need to create a new DB table called pool for this. Schema is shared below.

1. In the main service for forkscanner, send a get request to below URL to get a json of active pools and save them in DB according to schema provided.

https://raw.githubusercontent.com/0xB10C/known-mining-pools/master/pools.json


**Perform Calculations.**


To begin with we will need a list of blocks current height to a specific depth (suggested limit is 10). Run a loop on each block and perform the below mentioned steps.

1. We will be using getrawtransaction json-RPC call. This call was not available before bitcoin version 0.16. so the first check will be to ensure that the node is running a version newer then 0.16. we save the version in out nodes table, and will be a simple check
1. Then we need coinbase transaction and list of tx\_ids 
   1. Get the block info by calling RPC getblock with verbose set to 1.
   1. Return none if no tx in block or if the block height is 0.
   1. The first transaction in each block is coinbase tx, so get its id and use in next step.
   1. Use RPC getrawtrasaction to get coinbase raw transaction.
   1. Return raw coinbase tx and list of tx ids.
1. If there is no coinbase transaction return none
1. Remove the first coinbase tx id from the list of tx ids.
1. Converts tx id hashes to binary and add the list of binary ids to block.
1. Next step is to detect the miner pool from coinbase transaction.
   1. The coinbase transaction details will have a ‘coinbase’ field under ‘vin’. If coinbase field does not exist throw an error 
   1. Convert the hash from coinbase field into corresponding ASCII representation and encode it UTF-8.
   1. Retrieve all pools from the pool table and run a loop on it and check if any pool.tag is included in coinbase ( retrieved in step 6.b ).
1. Assign the pool retrieved in step 6 as block.pool
1. Calculate total fee
   1. From coinbase tx, sum up all values.
   1. Multiply by 100\_000\_000 to get satoshies.
   1. Subtract block.max\_inflation (calculated and assigned to each block during inflation checks) 
   1. Divide by 100\_000\_000 to get bitcoins.
1. If we are unable to find a pool we can just add the complete coinbase field from step 6 to block.pool
1. Update the block in DB.


**DB schema for tables.**

Pool:

string "tag"

`    	`string "name"

`    	`string "url"

`    	`datetime "created\_at"

`    	`datetime "updated\_at"
























**Specification For Process Template**

Template processing is a two-step process

1. Retrieve block templates.
1. Process block templates.

**Retrieve block templates.**

This is a straightforward service which simply queries the Node using getblocktemplate RPC call and retrieves block template from the node and saves it in the block template table in the DB, (Schema is shared below). Please not that we retrieve template repeatedly from all nodes we are connected to. 

Consider the following while adding template in DB

1. We can retrieve the parent block from our DB using template[“previousblockhash”] in our query.
1. While storing tx\_ids convert the hashes into binary.
1. tx\_fee\_rates is a list of rates which can be calculated as below
   1. run a loop on template[“transactions”]
   1. for each transaction divide it’s fee by  (it’s weight  / 4)
   1. return a list of tx\_fee\_rates.
1. Fee\_total  =  template[“ coinbasevalue”] – max\_inflation at this template[“height”]. Convert to satoshies before saving. Also max inflation can be calculated using the steps mentioned above (under **check inflation for each node**)


**Process Block Templates.**

This is where we process the templates we retrieved in previous step. 

1. get the minimum height from the block templates table.
1. Get blocks from blocks table where following conditions are met 
   1. Block height >= minimum height retrieved from block templates table.
   1. template\_txs\_fee\_diff is null
   1. total\_fee is not null
1. for each block retrieved in step 2, run the following steps.
   1. Get the latest block template where template height = block height and tx\_ids are not null.
   1. Retrieve tx\_ids from block template
   1. Retrieve tx\_ids from block
   1. Compare both the tx\_ids and figure out the omitted ones and unexpected ones.
   1. Update the blocks table accordingly.

**DB Schema.**

`  `create\_table "block\_templates”

`    `bigint "parent\_block\_id"

`    `bigint "node\_id"

`    `decimal "fee\_total"

`    `datetime "timestamp"

`    `integer "height"

`    `datetime "created\_at",

`    `datetime "updated\_at"

`    `integer "n\_transactions”

`    `binary "tx\_ids"

`    `integer "coin", null: false

`    `integer "tx\_fee\_rates"

`    `integer "lowest\_fee\_rate"


**Soft Fork Processing**

This part of the code simply keeps a track of soft forks that have occurred uptil now. Simply query the bitcoin node with get\_block\_chain\_info RPC call. In the result we will have information for all softforks. Create or update the entries in DB accordingly. Do this for each connected node.

**DB Schema:**

`  `create\_table "softforks

`    `integer "coin"

`    `bigint "node\_id"

`    `integer "fork\_type"

`    `string "name"

`    `integer "bit"

`    `integer "status"

`    `integer "since"

`    `datetime "created\_at”

`    `datetime "updated\_at"

`    `datetime "notified\_at"




## DB SCHEMA

This is a offline tool, your data stays locally and is not send to any server!
Feedback & Bug Reports
