use bitcoincore_rpc::bitcoincore_rpc_json::GetBlockHeaderResult;
use chrono::prelude::*;
use diesel::prelude::*;
use diesel::result::QueryResult;
use serde::{Deserialize, Serialize};

use crate::schema::{
    blocks, chaintips, double_spent_by, invalid_blocks, nodes, peers, rbf_by, stale_candidate,
    stale_candidate_children, transaction, valid_blocks,
};

#[derive(Clone, Deserialize, Serialize, Debug, AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "chaintips"]
pub struct Chaintip {
    pub id: i64,
    pub node: i64,
    pub status: String,
    pub block: String,
    pub height: i64,
    pub parent_chaintip: Option<i64>,
}

impl Chaintip {
    /// Update an entry in the chaintips table
    pub fn update(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        diesel::update(chaintips.filter(id.eq(self.id)))
            .set(self)
            .execute(conn)
    }

    /// Get the active chaintip, given a node_id
    pub fn get_active(conn: &PgConnection, node_id: i64) -> QueryResult<Chaintip> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(node.eq(node_id).and(status.eq("active")))
            .first(conn)
    }

    /// Check if a block hash is marked invalid in the chaintips database
    pub fn get_invalid(conn: &PgConnection, hash: &String) -> QueryResult<Chaintip> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(block.eq(hash).and(status.eq("invalid")))
            .first(conn)
    }

    /// Fetch all invalid chaintips that are ahead of a particular height.
    pub fn list_invalid_gt(conn: &PgConnection, tip_height: i64) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(height.gt(tip_height).and(status.eq("invalid")))
            .load(conn)
    }

    /// List all active tips.
    pub fn list_active(conn: &PgConnection) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips.filter(status.eq("active")).load(conn)
    }

    /// List all active tips.
    pub fn list(conn: &PgConnection) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips.load(conn)
    }

    /// List active tips that are ahead of a given height.
    pub fn list_active_gt(conn: &PgConnection, tip_height: i64) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(height.gt(tip_height).and(status.eq("active")))
            .load(conn)
    }

    /// List active tips that are behind a given height.
    pub fn list_active_lt(conn: &PgConnection, tip_height: i64) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(height.lt(tip_height).and(status.eq("active")))
            .load(conn)
    }

    /// Delete all chaintip entries that are not active.
    pub fn purge(conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        use diesel::dsl::not;
        diesel::delete(chaintips)
            .filter(not(status.eq("active")))
            .execute(conn)
    }

    /// Create an entry for an invalid fork.
    pub fn set_invalid_fork(
        conn: &PgConnection,
        block_height: i64,
        hash: &String,
        node_id: i64,
    ) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        diesel::insert_into(chaintips)
            .values((
                node.eq(node_id),
                block.eq(hash),
                height.eq(block_height),
                status.eq("invalid"),
            ))
            .execute(conn)
    }

    /// Create entry for a valid fork.
    pub fn set_valid_fork(
        conn: &PgConnection,
        block_height: i64,
        hash: &String,
        node_id: i64,
    ) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        diesel::insert_into(chaintips)
            .values((
                node.eq(node_id),
                block.eq(hash),
                height.eq(block_height),
                status.eq("valid-fork"),
            ))
            .execute(conn)
    }

    /// Update or create the active tip entry for a node.
    pub fn set_active_tip(
        conn: &PgConnection,
        block_height: i64,
        hash: &String,
        node_id: i64,
    ) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        let tip = chaintips
            .filter(status.eq("active").and(node.eq(node_id)))
            .first::<Chaintip>(conn);

        match tip {
            Ok(tip) => {
                if &tip.block != hash {
                    // remove parent chaintip references....
                    diesel::update(chaintips.filter(parent_chaintip.eq(tip.id)))
                        .set(parent_chaintip.eq::<Option<i64>>(None))
                        .execute(conn)?;

                    diesel::update(chaintips.filter(id.eq(tip.id)))
                        .set((
                            block.eq(hash),
                            height.eq(block_height),
                            parent_chaintip.eq::<Option<i64>>(None),
                        ))
                        .execute(conn)
                } else {
                    Ok(0)
                }
            }
            Err(diesel::result::Error::NotFound) => diesel::insert_into(chaintips)
                .values((
                    node.eq(node_id),
                    status.eq("active"),
                    block.eq(hash),
                    height.eq(block_height),
                ))
                .execute(conn),
            Err(e) => Err(e),
        }
    }
}

#[derive(QueryableByName, Queryable, Insertable, Debug)]
#[table_name = "blocks"]
pub struct Height {
    pub height: i64,
}

#[derive(Serialize, AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "blocks"]
pub struct Block {
    pub hash: String,
    pub height: i64,
    pub parent_hash: Option<String>,
    pub connected: bool,
    pub first_seen_by: i64,
    pub headers_only: bool,
    pub work: String,
}

impl Block {
    pub fn get(conn: &PgConnection, block_hash: &String) -> QueryResult<Block> {
        use crate::schema::blocks::dsl::*;
        blocks.find(block_hash).first(conn)
    }

    /// Look up a parent block.
    pub fn parent(&self, conn: &PgConnection) -> QueryResult<Block> {
        use crate::schema::blocks::dsl::*;

        if let Some(parent) = &self.parent_hash {
            blocks.find(parent).first(conn)
        } else {
            Err(diesel::result::Error::NotFound)
        }
    }

    pub fn count_at_height(conn: &PgConnection, block_height: i64) -> QueryResult<usize> {
        use crate::schema::blocks::dsl::*;

        blocks.filter(height.eq(block_height)).execute(conn)
    }

    pub fn get_at_height(conn: &PgConnection, block_height: i64) -> QueryResult<Vec<Block>> {
        use crate::schema::blocks::dsl::*;

        blocks.filter(height.eq(block_height)).load(conn)
    }

    pub fn find_stale_candidates(conn: &PgConnection, height: i64) -> QueryResult<Vec<Height>> {
        let raw_query = format!(
            "
            SELECT height FROM blocks
            WHERE height > {}
            GROUP BY height
            HAVING count(height) > 1
            ORDER BY height ASC
        ",
            height
        );

        diesel::sql_query(raw_query).load(conn)
    }

    pub fn num_transactions(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::transaction::dsl::*;

        transaction.filter(block_id.eq(&self.hash)).execute(conn)
    }

    pub fn block_and_descendant_transactions(
        &self,
        conn: &PgConnection,
        limit: i64,
    ) -> QueryResult<Vec<Transaction>> {
        use crate::schema::transaction::dsl::*;

        let block_ids: Vec<_> = self
            .descendants(conn, Some(limit))?
            .iter()
            .map(|b| b.hash.clone())
            .collect();

        transaction
            .filter(is_coinbase.eq(false).and(block_id.eq_any(block_ids)))
            .load(conn)
    }

    /// Fetch the entire list of descendants for the current block.
    pub fn descendants(&self, conn: &PgConnection, limit: Option<i64>) -> QueryResult<Vec<Block>> {
        let limit = if limit.is_some() {
            format!("WHERE height <= {}", self.height + limit.unwrap())
        } else {
            "".into()
        };

        let raw_query = format!(
            "
            WITH RECURSIVE rec_query AS (
                SELECT * FROM blocks WHERE hash = '{}'
                UNION ALL
                SELECT b.* FROM blocks b INNER JOIN rec_query r ON r.hash = b.parent_hash
            ) SELECT * FROM rec_query
			{}
			ORDER BY height ASC;
        ",
            self.hash, limit
        );

        diesel::sql_query(raw_query).load(conn)
    }

    /// Fetch the list of descendants for the current block ordered by work.
    pub fn descendants_by_work(&self, conn: &PgConnection, limit: i64) -> QueryResult<Vec<Block>> {
        let raw_query = format!(
            "
            WITH RECURSIVE rec_query AS (
                SELECT * FROM blocks WHERE hash = '{}'
                UNION ALL
                SELECT b.* FROM blocks b INNER JOIN rec_query r ON r.hash = b.parent_hash
            ) SELECT * FROM rec_query
			WHERE height < {}
			ORDER BY height,work ASC;
        ",
            self.hash, limit
        );

        diesel::sql_query(raw_query).load(conn)
    }
    /// Fetch all blocks that point to the block with a given hash.
    pub fn children(conn: &PgConnection, block_hash: &String) -> QueryResult<Vec<Block>> {
        use crate::schema::blocks::dsl::*;

        blocks.filter(parent_hash.eq(block_hash)).load(conn)
    }

    /// Highest block height.
    pub fn max_height(conn: &PgConnection) -> QueryResult<Option<i64>> {
        use crate::schema::blocks::dsl::*;
        use diesel::dsl::max;

        blocks.select(max(height)).first(conn)
    }

    /// Which blocks do we only have headers for?
    pub fn headers_only(conn: &PgConnection, max_depth: i64) -> QueryResult<Vec<Block>> {
        use crate::schema::blocks::dsl::*;

        blocks
            .filter(headers_only.eq(true).and(height.gt(max_depth)))
            .order(height.asc())
            .load(conn)
    }

    /// Fetch block if we have it, or create.
    pub fn get_or_create(
        conn: &PgConnection,
        headers_only: bool,
        first_seen_by: i64,
        header: &GetBlockHeaderResult,
    ) -> QueryResult<Block> {
        use crate::schema::blocks::dsl as bs;
        let block = bs::blocks
            .find(header.hash.to_string())
            .first::<Block>(conn);

        match block {
            Err(diesel::result::Error::NotFound) => {
                let prev_hash = header.previous_block_hash.map(|h| h.to_string());

                let block = Block {
                    hash: header.hash.to_string(),
                    height: header.height as i64,
                    parent_hash: prev_hash,
                    connected: false,
                    headers_only,
                    first_seen_by,
                    work: hex::encode(&header.chainwork),
                };

                conn.transaction::<usize, diesel::result::Error, _>(|| {
                    let _ = block.insert(conn)?;
                    diesel::update(bs::blocks.filter(bs::parent_hash.eq(header.hash.to_string())))
                        .set(bs::connected.eq(true))
                        .execute(conn)
                })?;

                Ok(block)
            }
            Ok(mut block) => {
                block.headers_only &= headers_only;
                block.update(&conn)?;

                Ok(block)
            }
            e => e,
        }
    }

    /// Update the database with current block info.
    pub fn update(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::blocks::dsl::*;
        diesel::update(blocks.filter(hash.eq(&self.hash)))
            .set(self)
            .execute(conn)
    }

    /// Node has marked block valid.
    pub fn set_valid(conn: &PgConnection, block_hash: &String, node_id: i64) -> QueryResult<usize> {
        use crate::schema::valid_blocks::dsl::*;

        let block = ValidBlock {
            hash: block_hash.to_string(),
            node: node_id,
        };

        diesel::insert_into(valid_blocks)
            .values(block)
			.on_conflict((hash, node))
			.do_nothing()
            .execute(conn)
    }

    /// Has block_hash been marked invalid by this node?
    pub fn marked_invalid_by(
        conn: &PgConnection,
        block_hash: &String,
        node_id: i64,
    ) -> QueryResult<bool> {
        use crate::schema::invalid_blocks::dsl::*;

        let rows = invalid_blocks
            .filter(hash.eq(block_hash).and(node.eq(node_id)))
            .execute(conn)?;
        Ok(rows > 0)
    }

    /// Has block_hash been marked valid by this node?
    pub fn marked_valid_by(
        conn: &PgConnection,
        block_hash: &String,
        node_id: i64,
    ) -> QueryResult<bool> {
        use crate::schema::valid_blocks::dsl::*;

        let rows = valid_blocks
            .filter(hash.eq(block_hash).and(node.eq(node_id)))
            .execute(conn)?;
        Ok(rows > 0)
    }

    /// Fetch number of nodes that marked invalid.
    pub fn marked_invalid(conn: &PgConnection, block_hash: &String) -> QueryResult<usize> {
        use crate::schema::invalid_blocks::dsl::*;

        invalid_blocks.filter(hash.eq(block_hash)).execute(conn)
    }

    /// Node has marked block invalid.
    pub fn set_invalid(
        conn: &PgConnection,
        block_hash: &String,
        node_id: i64,
    ) -> QueryResult<usize> {
        use crate::schema::invalid_blocks::dsl::*;

        let block = InvalidBlock {
            hash: block_hash.to_string(),
            node: node_id,
        };

        diesel::insert_into(invalid_blocks)
            .values(block)
            .execute(conn)
    }

    pub fn insert(&self, conn: &PgConnection) -> QueryResult<usize> {
        diesel::insert_into(blocks::dsl::blocks)
            .values(self)
            .execute(conn)
    }
}

#[derive(QueryableByName, Queryable, Insertable)]
#[table_name = "transaction"]
pub struct Transaction {
    pub block_id: String,
    pub txid: String,
    pub is_coinbase: bool,
    pub hex: String,
    pub amount: f64,
}

impl Transaction {
    pub fn create(
        conn: &PgConnection,
        block: String,
        idx: usize,
        tx_id: &String,
        tx_hex: &String,
        tx_amount: f64,
    ) -> QueryResult<usize> {
        use crate::schema::transaction::dsl::*;

        let tx = Transaction {
            block_id: block,
            is_coinbase: idx == 0,
            txid: tx_id.clone(),
            hex: tx_hex.clone(),
            amount: tx_amount,
        };

        diesel::insert_into(transaction)
            .values(tx)
            .on_conflict(txid)
            .do_nothing()
            .execute(conn)
    }

    pub fn amount_for_txs(conn: &PgConnection, txids: &Vec<String>) -> QueryResult<f64> {
        use crate::schema::transaction::dsl::*;
        use diesel::dsl::max;

        let results: Vec<Option<f64>> = transaction
            .filter(txid.eq_any(txids))
            .group_by(txid)
            .select(max(amount))
            .load(conn)?;
        Ok(results
            .into_iter()
            .fold(0.0, |a, b| a + b.unwrap_or_default()))
    }

    pub fn tx_block_and_descendants(conn: &PgConnection, id: String) -> QueryResult<Vec<Block>> {
        use crate::schema::blocks::dsl as bdsl;
        use crate::schema::transaction::dsl::*;

        let blocks: Vec<(Transaction, Block)> = transaction
            .inner_join(bdsl::blocks)
            .filter(txid.eq(id))
            .load(conn)?;

        let descendants: Vec<Block> = blocks
            .into_iter()
            .filter_map(|b| {
                if let Ok(desc) = b.1.descendants(conn, None) {
                    Some(desc)
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        Ok(descendants)
    }
}

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "stale_candidate_children"]
pub struct StaleCandidateChildren {
    pub candidate_height: i64,
    pub root_id: String,
    pub tip_id: String,
    pub len: i32,
}

impl StaleCandidateChildren {
    pub fn create(
        conn: &PgConnection,
        root: &Block,
        tip: &Block,
        branch_len: i32,
    ) -> QueryResult<usize> {
        use crate::schema::stale_candidate_children::dsl::*;

        let c = StaleCandidateChildren {
            candidate_height: root.height,
            root_id: root.hash.clone(),
            tip_id: tip.hash.clone(),
            len: branch_len,
        };

        diesel::insert_into(stale_candidate_children)
            .values(c)
            .execute(conn)
    }

    pub fn purge(conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::stale_candidate_children::dsl::*;

        diesel::delete(stale_candidate_children).execute(conn)
    }
}

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "rbf_by"]
pub struct RbfBy {
    pub candidate_height: i64,
    pub txid: String,
}

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "double_spent_by"]
pub struct DoubleSpentBy {
    pub candidate_height: i64,
    pub txid: String,
}

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "stale_candidate"]
pub struct StaleCandidate {
    pub height: i64,
    pub n_children: i32,
    pub confirmed_in_one_branch_total: f64,
    pub double_spent_in_one_branch_total: f64,
    pub rbf_total: f64,
    pub height_processed: Option<i64>,
}

impl StaleCandidate {
    pub fn get(conn: &PgConnection, candidate: i64) -> QueryResult<StaleCandidate> {
        use crate::schema::stale_candidate::dsl::*;
        stale_candidate.find(candidate).first(conn)
    }

    pub fn update(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::stale_candidate::dsl::*;
        diesel::update(stale_candidate.filter(height.eq(self.height)))
            .set(self)
            .execute(conn)
    }

    pub fn update_rbf_by(&self, conn: &PgConnection, txids: &Vec<String>) -> QueryResult<usize> {
        use crate::schema::rbf_by::dsl::*;

        let rbfs: Vec<RbfBy> = txids
            .iter()
            .map(|tx| RbfBy {
                candidate_height: self.height,
                txid: tx.clone(),
            })
            .collect();

        diesel::insert_into(rbf_by).values(rbfs).execute(conn)
    }

    pub fn update_double_spent_by(
        &self,
        conn: &PgConnection,
        txids: &Vec<String>,
    ) -> QueryResult<usize> {
        use crate::schema::double_spent_by::dsl::*;
        let double_spends: Vec<DoubleSpentBy> = txids
            .iter()
            .map(|tx| DoubleSpentBy {
                candidate_height: self.height,
                txid: tx.clone(),
            })
            .collect();

        diesel::insert_into(double_spent_by)
            .values(double_spends)
            .execute(conn)
    }

    pub fn children(&self, conn: &PgConnection) -> QueryResult<Vec<StaleCandidateChildren>> {
        use crate::schema::stale_candidate_children::dsl::*;

        stale_candidate_children
            .filter(candidate_height.eq(self.height))
            .order_by(len)
            .load(conn)
    }

    pub fn purge_children(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::stale_candidate_children::dsl::*;

        diesel::delete(stale_candidate_children)
            .filter(candidate_height.eq(self.height))
            .execute(conn)
    }

    pub fn create(conn: &PgConnection, candidate_height: i64, children: i32) -> QueryResult<usize> {
        use crate::schema::stale_candidate::dsl::*;

        let candidate = StaleCandidate {
            n_children: children,
            height: candidate_height,
            confirmed_in_one_branch_total: 0.,
            double_spent_in_one_branch_total: 0.,
            rbf_total: 0.,
            height_processed: None,
        };

        diesel::insert_into(stale_candidate)
            .values(candidate)
            .on_conflict(height)
            .do_nothing()
            .execute(conn)
    }

    pub fn top_n(conn: &PgConnection, n: i64) -> QueryResult<Vec<StaleCandidate>> {
        use crate::schema::stale_candidate::dsl::*;

        stale_candidate.order_by(height.desc()).limit(n).load(conn)
    }
}

#[derive(QueryableByName, Queryable, Insertable)]
#[table_name = "nodes"]
pub struct Node {
    pub id: i64,
    pub node: String,
    pub rpc_host: String,
    pub rpc_port: i32,
    pub mirror_rpc_port: Option<i32>,
    pub rpc_user: String,
    pub rpc_pass: String,
    pub unreachable_since: Option<DateTime<Utc>>,
}

impl Node {
    pub fn list(conn: &PgConnection) -> QueryResult<Vec<Node>> {
        nodes::dsl::nodes.load(conn)
    }

    /// Fetch nodes that also have a mirror.
    pub fn get_mirrors(conn: &PgConnection) -> QueryResult<Vec<Node>> {
        use crate::schema::nodes::dsl::*;
        nodes.filter(mirror_rpc_port.is_not_null()).load(conn)
    }

    pub fn remove(conn: &PgConnection, node_id: i64) -> QueryResult<usize> {
        use crate::schema::nodes::dsl::*;
        diesel::delete(nodes).filter(id.eq(node_id)).execute(conn)
    }

    pub fn insert(
        conn: &PgConnection,
        name: String,
        host: String,
        port: i32,
        mirror: Option<i32>,
        user: String,
        pass: String,
    ) -> QueryResult<Node> {
        use crate::schema::nodes::dsl::*;
        diesel::insert_into(nodes)
            .values((
                node.eq(name),
                rpc_host.eq(host),
                rpc_port.eq(port),
                mirror_rpc_port.eq(mirror),
                rpc_user.eq(user),
                rpc_pass.eq(pass),
            ))
            .get_result(conn)
    }
}

#[derive(QueryableByName, Queryable, Insertable)]
#[table_name = "peers"]
pub struct Peer {
    pub id: i64,
    pub node_id: i64,
    pub peer_id: i64,
    pub address: String,
    pub version: i64,
}

impl Peer {}

#[derive(QueryableByName, Queryable, Insertable)]
#[table_name = "invalid_blocks"]
pub struct InvalidBlock {
    pub hash: String,
    pub node: i64,
}

#[derive(QueryableByName, Queryable, Insertable)]
#[table_name = "valid_blocks"]
pub struct ValidBlock {
    pub hash: String,
    pub node: i64,
}
