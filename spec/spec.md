#                                                                 SPECS for ForkScanner

As the name suggests Forkscanner is responsible to scan the bitcoin chain continously. It monitors the forks in the BTC blockchian, and at any given moment it should be able to give us the current active chaintip. Forkscanner relies on multiple bitcoin nodes, these nodes are geographically split and use different btc core versions. this configuration of nodes ensures that we detect all forks. 

Apart from monitoring forks, forkscanner also checks for any potential double spends in the chain and keeps track of inflation. there is also an address watcher incorporated in the forkscanner, which keeps an eye out for the addresses we want to monitor and notifies when there is an inflow or outflow of funds from any of addresses under watch.

Please go through the DB schema [here](https://github.com/twilight-project/forkscanner/blob/spec-fixes/spec/db_schema.md). as this tool relies heavily on SQL DB.

## Requirements:

- The system will run and maintain at least 3 BTC nodes with different versions.
- One of these 3 nodes will also have a mirror node.
- A Postgres DB shall be maintained (Schema diagram shared with this document).
- An API to add and retrieve data (details below)
- System shall have a service for bootstrapping. (Details below)
- System will have a service for fork validation. (Explained below)
- System will have a service for double spend detection. (Explained below)


## API Spec

Forkscanner exposes a variety of API endpoint to allow other application to make use of its features. these API endpoints consist of JSON-RPC as well as Web Socket endpoint which follow a pub/sub apprach, examples can be found in the readme file

### RPC endpoints
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

### pub/sub endpoints

- active_fork
returns a single true active chaintip

- forks
returns active chaintip for each node

- validation_checks
returns the stale candidates and the height difference between active tip and the candidate

- invalid_block_checks
returns any new invalid block

- lagging_nodes_checks
returns a lagging nodes.

##                                                             Fork Detection

To begin with Forkscanner connects with the bitcoin nodes and uses **rpc::getchaintips** to get the chaintips from all connected bitcoin nodes. This information is saved in the Postgres DB and is later used to deduce the True chaintip.

## bootstrapping the system

When the system starts, It cleans old/inactive chaintips, this is accomplished by removing any chaintips entries from the DB. Then forkscanner starts querying the connected nodes using the **rpc::getchaintips**. 

example of result returned by the **rpc::getchaintips** can be viewed [here](https://developer.bitcoin.org/reference/rpc/getchaintips.html). As described in the example this endpoint returns a list of chaintips seen by that specific node. Each of these chaintips has one of the below listed status

- active
- valid-fork
- invalid
- headers only
- Valid headers

Also please keep in mind that chaintip is the blockhash of the latest block of the btc chain and that a bitcoin node can only have one chaintip which it considers active. Each chaintip status mentioned above is processed differently.

### Active
Active means that node considers this chaintip to be the latest one. when the forkscanner gets active chaintip from the node, it follows below steps

- Create an entry for this block in blocks table, if it does not exist already.
- Update the mark_valid_by field in the block table by the node id of the chaintip object.
- Find or create the chain tip in DB (chaintips table).
- If chain tip exists and the latest tip block is not equal to the block we got earlier.
    - Update the tip block.
    - set parent_chaintip_id to null.
    - set each childs parent_chaintip_id to null.

### Valid-forks
valid fork means that the node has validated that a fork occured with this chaintip. since valid-fork means that its a fork, forkscanner needs to maintain a hierarchy to find the block where the fork occured. when the forkscanner gets valid-fork chaintip from the node, it follows below steps

- Find or create a the block and its ancestors.
    - Get the block by hash form DB.
    - If we don’t find the block
       - Get the block from node and add in the DB.
    - Once we have the block find its ancestors.
       - We run a loop and break if a certain height is achieved (height check is to limit computing).
          - Get block from DB.
          - Set parent = current block’s parent.
          - If parent is null.
             - Get current block from node or just block headers from node if block is not found.
             - Set parent = as currents block’s previous block hash.
             - Update current blocks parent_id DB with parent.
          - If we have a parent in DB.
             - Break if parent is connected.
          - Else
             - We get the parent block from Node.
          - Update parent by this new block and add the parent block in DB. And set mark_valid
             field.
          - Update current’s block’s parent_id by parent.
          - Set block as the parent and re run the loop for parent to find ancestors.



- We process each scenario differently (explained below).^
- After processing the chain tips we match children, check_parent, match_parent for each
node and chain tip combination. (All 3 processes are explained below)
- After creating block entries we check for invalid blocks (explained below)
- The last step in this service is to detect the blocks which have the tendency to go stale.

### Invalid:
invalid means that there isa chaintips which this nodes considers invalid. processing is as follow

-its the same as valid-forks except that we set the mark_invalid field.

### Valid-headers/Headers-only:
Headers only means that the node only have the header for this chaintip where as valid-headers means that the node only has headers but considers these headers valid. the processing of these statuses are a under.

- Return if the block height is less then minimum height.
- Return if block is present in DB
- Otherwise create an entry in DB and mark the headers-only field as True.

### Match Children:
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

### Check Parent:
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
### Match Parent:
If we don't already have a parent, check if any of the other nodes are ahead of us. Use their chain
tip instead unless we consider it invalid
- Steps are same as match children except we replace Update candidates’ parent chain tip to self
by Update parent chain tip to candidate tip

### Check Invalid Blocks:
In this part we check the blocks and see if they are marked invalid by a node. If so we add that in
the invalid blocks table (Schema Shared above).

- Retrieve blocks from the DB where the block is marked valid and in valid by at least 1 node.
- Run a loop on the retrieved blocks
    - Get the first node which marked the block invalid.^
    - Create an entry in the invalid_block table for this block.^

### Check for Stale Candidates:
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



## Specification For Inflation Checks

This part of the specs monitors inflation and keeps a track of miner rewards for each block mined. this should update our inflated blocks table and should add the blocks where inflation happened. This feature will be implemented in 2 steps mentioned below.

- A service which will pick up all nodes which are available and are up to date.
- Running inflation checks on each node independently, preferably in threads.

we will also need to add 2 more data models. Schema for which will be shared later in the document

- Inflated Blocks.
- Tx_outsets.



### Service to get a list of nodes.


We are maintaining a list of nodes we pick up all nodes from there and run a loop with below mentioned specifications.

- First step Is to check if the node is currently available and is reachable. Following below steps, we can manage a list of nodes which are currently active and from time to time check up on inactive nodes to update our list.
   - We maintain an unreachable_since field (datetime type) in the Nodes table in DB.
   - If this field is null or the value is more then 10 min ago. we query the node with a getbestblockhash RPC call.
   - If we get a result back, we unpdate the DB and set unreachable_since to null
   - If we get a timeout, we set the current datetime as the value for unreachable_since.
- Then we filter the nodes by excluding any node which is in initial block download phase
   - This can be achieved by creating a bool field in nodes table, named IBD.
   - Than we compare getblockchaininfo RPC from this node and another fully running
   - If the difference in block height is more then 10 we can mark the node as IBD.
- Then we run a new thread for each node and call check_inflation_for_node (explained below) method


### Check Inflation on each Node.

This method is where we check inflation on each node, please note that it is supposed to be run as a separate thread for each node to ensure real-time calculations. This method also relies on gettxoutsetinfo RPC call.  

- First query the DB and check if we have tx_outsets for latest block and this Node.
- If we have the tx_outsets, it means node does not have the latest block yet or we already have done the calculation for latest block. Either way we need to wait for next block. So, we can sleep for some time and exit the thread.
- If we don’t have the tx_outsets. We start by stopping p2p networking on the node.
- We need to maintain a list of blocks we need to check. Let’s refer to it as blocks_to_check. Add the current block in this list.
- Run an infinite loop.
   - Add a check to only go to a depth of 10 blocks to calculate inflation. If it exceeds 10 blocks break the loop
   - We need the current block and parent block on each loop iteration. Let refer to them as current_block and parent_block.
   - If we cannot find the parent block throw an error stating that unable to calculate inflation due to missing blocks.
   - Look for tx_outsets for the parent block
   - If we have the tx_outsets for parent block. Break the loop otherwise add the parent block in the blocks_to_check list. Please make sure that the new blocks are added at the start to maintain an order.
- When the loop ends, we should have a list of blocks we need to check. That should not be more then 10 blocks. Going above that would be costly hence just 10 blocks should be enough.
- Run another loop on the blocks to check list.
   - Get the UTXO balance at the height by using RPC call gettxoutsetinfo.
   - Perform the undo_rollback. The process is explained previously.
   - Make sure we have the right tx_outset by comparing tx_outset [ blockhash ] and best block hash.
   - Add this new tx_outset into the DB according to the DB schema for this table.
   - Retrieve the tx_outsets from DB for the parent block.
   - Throw exception if parent block not found.
   - Now inflation can be calculated as. Lets call it current_inflation

      inflation = current_block.total_amount – parent_block.total_amount
   - Now we calculate max inflation.
      - interval = current_height / 210,000 (because after every 210,000 blocks miner reward changes)
      - reward = 50 * 100,000,000 (to get satoshies)
      - max_inflation = reward >> interval (divide by 2 every time the miner reward changes to get the current reward)

- now we compare max inflation. if current_inflation < max_inflation. Continue to next iteration of loop. Also please make sure that both current and max inflation have the same unit of measure.
- Otherwise add the current_block in the inflated blocks table. 
- Turn the p2p networking back on.


## Specification For Fee Calculation.

This part of the fork scanner will investigate which miner pool mined a block and how much fee was charged. It’s a 2 step process.

- Update the pool table in DB
- Perform calculation and update blocks in DB.


### Update the pool table in DB.

We will need to create a new DB table called pool for this. Schema is shared below.

- In the main service for forkscanner, send a get request to below URL to get a json of active pools and save them in DB according to schema provided.

https://raw.githubusercontent.com/bitcoin-data/mining-pools/generated/pools.json

### Perform Calculations.


To begin with we will need a list of blocks current height to a specific depth (suggested limit is 10). Run a loop on each block and perform the below mentioned steps.

- We will be using getrawtransaction json-RPC call. This call was not available before bitcoin version 0.16. so the first check will be to ensure that the node is running a version newer then 0.16. we save the version in out nodes table, and will be a simple check
- Then we need coinbase transaction and list of tx_ids 
   - Get the block info by calling RPC getblock with verbose set to 1.
   - Return none if no tx in block or if the block height is 0.
   - The first transaction in each block is coinbase tx, so get its id and use in next step.
   - Use RPC getrawtrasaction to get coinbase raw transaction.
   - Return raw coinbase tx and list of tx ids.
- If there is no coinbase transaction return none
- Remove the first coinbase tx id from the list of tx ids.
- Converts tx id hashes to binary and add the list of binary ids to block.
- Next step is to detect the miner pool from coinbase transaction.
   - The coinbase transaction details will have a ‘coinbase’ field under ‘vin’. If coinbase field does not exist throw an error 
   - Convert the hash from coinbase field into corresponding ASCII representation and encode it UTF-8.
   - Retrieve all pools from the pool table and run a loop on it and check if any pool.tag is included in coinbase ( retrieved in step 6.b ).
- Assign the pool retrieved in step 6 as block.pool
- Calculate total fee
   - From coinbase tx, sum up all values.
   - Multiply by 100_000_000 to get satoshies.
   - Subtract block.max_inflation (calculated and assigned to each block during inflation checks) 
   - Divide by 100_000_000 to get bitcoins.
- If we are unable to find a pool we can just add the complete coinbase field from step 6 to block.pool
- Update the block in DB.


## Specification For Process Template.

Template processing is a two-step process

- Retrieve block templates.
- Process block templates.

### Retrieve block templates.

This is a straightforward service which simply queries the Node using getblocktemplate RPC call and retrieves block template from the node and saves it in the block template table in the DB, (Schema is shared below). Please not that we retrieve template repeatedly from all nodes we are connected to. 

Consider the following while adding template in DB

- We can retrieve the parent block from our DB using template[“previousblockhash”] in our query.
- While storing tx_ids convert the hashes into binary.
- tx_fee_rates is a list of rates which can be calculated as below
   - run a loop on template[“transactions”]
   - for each transaction divide it’s fee by  (it’s weight  / 4)
   - return a list of tx_fee_rates.
- Fee_total  =  template[“ coinbasevalue”] – max_inflation at this template[“height”]. Convert to satoshies before saving. Also max inflation can be calculated using the steps mentioned above (under **check inflation for each node**)


### Process Block Templates.

This is where we process the templates we retrieved in previous step. 

- get the minimum height from the block templates table.
- Get blocks from blocks table where following conditions are met 
   - Block height >= minimum height retrieved from block templates table.
   - template_txs_fee_diff is null
   - total_fee is not null
- for each block retrieved in step 2, run the following steps.
   - Get the latest block template where template height = block height and tx_ids are not null.
   - Retrieve tx_ids from block template
   - Retrieve tx_ids from block
   - Compare both the tx_ids and figure out the omitted ones and unexpected ones.
   - Update the blocks table accordingly.


### Soft Fork Processing

This part of the code simply keeps a track of soft forks that have occurred uptil now. Simply query the bitcoin node with get_block_chain_info RPC call. In the result we will have information for all softforks. Create or update the entries in DB accordingly. Do this for each connected node.

### Lagging Nodes
 
We need to add functionality to keep tracks of nodes which lag behind. This will help us ignore such nodes while we are querying active chaintips.
after polling the nodes for blocks and chaintips. Follow the below steps.

1.	Query the nodes table and get a list of nodes.
2.	Run this for all nodes.
3.	Compare the active chaintip block from this node to the active chaintip block from other nodes if its equal the node is nor behind.
4.	If the node is not lagging behind. Delete the entry (if any) for this node
5.	Otherwise check the work for the latest block on this node and the latest block on current active tip. If the active chain tips work is less than the work of this node. It means the node is lagging behind.
6.	Also check if the height of current node is less than the height of active chaintip. If the difference is more than one block. Then it means that the node is lagging behind.
7.	Add the entry in Lag table.

We will also need a pubsub which would give us the current lagging nodes. 

Also filter lagging nodes out of the get_forks pubsub as well. So that at any given time get_forks only gives us one chaintip.


### PubSub for invalid blocks.

We need an RSS feed (pubsub) for invalid blocks. If we have a block which is marked valid by one node and invalid by another node, that means there is an issue with the consensus, which will be fixed by the chain.

we need to be notified of such occurrence in the fork scanner so that we can make decisions based on this information.

We have valid blocks and invalid blocks table. We have nodes column in both these tables. we can pick up blocks which are marked valid by on node and invalid by another node by using these tables. 

we will have to create a new column created_at. This will allow us to limit our search to past 15 min.

### Address watcher
A Pubsub API endpoint which would take an address upon subscription and will publish back if there is a transaction made against that address.

To do that forkscanner should maintain a DB table ‘watched’ schema provided below. 

With every new block mined. Fork scanner will pick it up and check each transaction in this block. And see if the active addresses in the watched tables are in any transaction. (TxIn and TxOut)

if we find a transaction save it in a Database table ‘watched transactions’(Schema below) and return the complete list of transactions (old and the new one) back to subscriber. 

Moreover, when an entity subscribes it will share an address and a time limit till which address watcher will keep that address under watch. 

If the time limit field is left empty, it means that this address needs to be under watch indefinitely.


### Submit Block to Fork Scanner.

A JSON RPC endpoint, which will be used by fork oracle to submit a block to fork scanner. For instance, fork oracle gets a confirmation of a block form one instance of fork scanner. It should be able to submit that block to another instance of fork scanner.

for this purpose we require a JSON endpoint which will take the block hash as an input. The fork scanner will send a get block from peers RPC call to all connected bitcoin nodes and pass the block hash and the peer id as arguments. If any node manages to get hold of the block the fork scanner should add this information in the blocks table in the DB.
