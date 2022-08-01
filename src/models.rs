use bigdecimal::BigDecimal;
use bitcoincore_rpc::bitcoincore_rpc_json::{GetBlockHeaderResult, Softfork};
use chrono::prelude::*;
use diesel::prelude::*;
use diesel::result::QueryResult;
use diesel::sql_types;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;

use crate::schema::{
    block_templates, blocks, chaintips, double_spent_by, fee_rates, inflated_blocks,
    invalid_blocks, lags, nodes, peers, pool, rbf_by, softforks, stale_candidate,
    stale_candidate_children, transaction, tx_outsets, valid_blocks, watched,
};
use crate::MinerPoolInfo;

pub fn serde_bigdecimal<S>(decimal: &Option<BigDecimal>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match *decimal {
        Some(ref d) => s.serialize_some(&d.to_string()),
        None => s.serialize_none(),
    }
}

#[derive(
    Clone, Deserialize, Serialize, Debug, AsChangeset, QueryableByName, Queryable, Insertable,
)]
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

    pub fn list_non_lagging(conn: &PgConnection) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        use crate::schema::lags::dsl as ldsl;

        let laggers: Vec<i64> = ldsl::lags.select(ldsl::node_id).load::<i64>(conn)?;

        chaintips.filter(node.ne_all(laggers)).load(conn)
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

#[derive(Debug, AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "pool"]
pub struct Pool {
    pub tag: String,
    pub name: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Pool {
    pub fn create_or_update_batch(
        conn: &PgConnection,
        pool_info: MinerPoolInfo,
    ) -> QueryResult<usize> {
        use crate::schema::pool::dsl::*;

        let MinerPoolInfo { coinbase_tags, .. } = pool_info;
        let mut pools = Vec::new();

        for (key, value) in coinbase_tags.into_iter() {
            pools.push(Pool {
                tag: key,
                name: value.name,
                url: value.link,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
        }
        diesel::insert_into(pool)
            .values(pools)
            .on_conflict((tag, name, url))
            .do_update()
            .set(updated_at.eq(Utc::now()))
            .execute(conn)
    }

    pub fn list(conn: &PgConnection) -> QueryResult<Vec<Pool>> {
        use crate::schema::pool::dsl::*;
        pool.load(conn)
    }
}

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "inflated_blocks"]
pub struct InflatedBlock {
    block_hash: String,
    max_inflation: BigDecimal,
    actual_inflation: BigDecimal,
    notified_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    node_id: i64,
    dismissed_at: Option<DateTime<Utc>>,
}

impl InflatedBlock {
    pub fn create(
        conn: &PgConnection,
        id: i64,
        block: &Block,
        max: BigDecimal,
        actual: BigDecimal,
    ) -> QueryResult<usize> {
        use crate::schema::inflated_blocks::dsl::*;
        let ib = InflatedBlock {
            block_hash: block.hash.clone(),
            max_inflation: max,
            actual_inflation: actual,
            notified_at: Utc::now(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            node_id: id,
            dismissed_at: None,
        };
        diesel::insert_into(inflated_blocks)
            .values(ib)
            .execute(conn)
    }
}

#[derive(Clone, AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "fee_rates"]
pub struct FeeRate {
    pub parent_block_hash: String,
    pub node_id: i64,
    pub fee_rate: i32,
    pub omitted: bool,
}

impl FeeRate {
    pub fn list_by(conn: &PgConnection, parent: String, node: i64) -> QueryResult<Vec<FeeRate>> {
        use crate::schema::fee_rates::dsl::*;

        fee_rates
            .filter(parent_block_hash.eq(parent).and(node_id.eq(node)))
            .load(conn)
    }

    pub fn update(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::fee_rates::dsl::*;
        diesel::update(
            fee_rates.filter(
                parent_block_hash
                    .eq(&self.parent_block_hash)
                    .and(node_id.eq(self.node_id))
                    .and(fee_rate.eq(self.fee_rate)),
            ),
        )
        .set(self)
        .execute(conn)
    }
}

#[derive(Debug, AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "block_templates"]
pub struct BlockTemplate {
    pub parent_block_hash: String,
    pub node_id: i64,
    pub fee_total: BigDecimal,
    pub ts: DateTime<Utc>,
    pub height: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub n_transactions: i32,
    pub tx_ids: Vec<u8>,
    pub lowest_fee_rate: i32,
}

impl BlockTemplate {
    pub fn create(
        conn: &PgConnection,
        parent: String,
        node: i64,
        total: BigDecimal,
        block_height: i64,
        n_txs: i32,
        txids: Vec<u8>,
        rates: Vec<i32>,
    ) -> QueryResult<usize> {
        use crate::schema::block_templates::dsl as btd;
        use crate::schema::fee_rates::dsl as frd;

        let lowest = *rates.iter().min().expect("No transaction fees!");
        let fee_rates: Vec<_> = rates
            .into_iter()
            .map(|rate| FeeRate {
                parent_block_hash: parent.clone(),
                node_id: node,
                fee_rate: rate,
                omitted: false,
            })
            .collect();

        let tpl = BlockTemplate {
            parent_block_hash: parent,
            node_id: node,
            fee_total: total,
            ts: Utc::now(),
            height: block_height,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            n_transactions: n_txs,
            tx_ids: txids,
            lowest_fee_rate: lowest,
        };

        diesel::insert_into(btd::block_templates)
            .values(tpl)
            .on_conflict_do_nothing()
            .execute(conn)?;

        diesel::insert_into(frd::fee_rates)
            .values(fee_rates)
            .on_conflict_do_nothing()
            .execute(conn)
    }

    pub fn purge(conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::block_templates::dsl::*;

        diesel::delete(block_templates).execute(conn)
    }

    pub fn get_min(conn: &PgConnection) -> QueryResult<Option<i64>> {
        use crate::schema::block_templates::dsl::*;
        use diesel::dsl::min;

        block_templates.select(min(height)).first(conn)
    }

    pub fn get_with_txs(conn: &PgConnection, block_height: i64) -> QueryResult<BlockTemplate> {
        use crate::schema::block_templates::dsl::*;

        block_templates.filter(height.eq(block_height)).first(conn)
    }
}

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "tx_outsets"]
pub struct TxOutset {
    pub block_hash: String,
    pub node_id: i64,
    pub txouts: i64,
    pub total_amount: BigDecimal,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub inflated: bool,
}

impl TxOutset {
    pub fn get(conn: &PgConnection, block: &String, node: i64) -> QueryResult<Option<TxOutset>> {
        use crate::schema::tx_outsets::dsl::*;
        let result = tx_outsets
            .filter(block_hash.eq(block).and(node_id.eq(node)))
            .get_result(conn);

        match result {
            Err(diesel::result::Error::NotFound) => Ok(None),
            Ok(ans) => Ok(Some(ans)),
            Err(e) => Err(e),
        }
    }

    pub fn create(
        conn: &PgConnection,
        tx_outs: u64,
        amount: BigDecimal,
        block: &String,
        node: i64,
    ) -> QueryResult<TxOutset> {
        use crate::schema::tx_outsets::dsl::*;
        let outset = TxOutset {
            block_hash: block.clone(),
            node_id: node,
            txouts: tx_outs as i64,
            total_amount: amount,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            // TODO: this
            inflated: false,
        };
        diesel::insert_into(tx_outsets)
            .values(outset)
            .get_result(conn)
    }

    pub fn update(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::tx_outsets::dsl::*;
        diesel::update(
            tx_outsets.filter(
                block_hash
                    .eq(&self.block_hash)
                    .and(node_id.eq(self.node_id)),
            ),
        )
        .set(self)
        .execute(conn)
    }
}

#[derive(Clone, AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "softforks"]
pub struct SoftForks {
    pub node_id: i64,
    pub fork_type: i32,
    pub name: String,
    pub bit: Option<i32>,
    pub status: i32,
    pub since: Option<i64>,
    pub notified_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SoftForks {
    pub fn update_or_insert(
        conn: &PgConnection,
        node: i64,
        forks: HashMap<String, Softfork>,
    ) -> QueryResult<()> {
        use crate::schema::softforks::dsl::*;

        for (key, info) in forks.into_iter() {
            let b = if let Some(b9) = info.bip9 {
                b9.bit.map(|b| b as i32)
            } else {
                None
            };

            let sf = SoftForks {
                node_id: node,
                fork_type: info.type_ as i32,
                name: key,
                bit: b,
                status: info.active as i32,
                since: info.height.map(|k| k as i64),
                notified_at: Utc::now(),
                updated_at: Utc::now(),
                created_at: Utc::now(),
            };
            diesel::insert_into(softforks)
                .values(&sf)
                .on_conflict((node_id, fork_type, name))
                .do_update()
                .set(&sf)
                .execute(conn)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "blocks"]
pub struct Block {
    pub hash: String,
    pub height: i64,
    pub parent_hash: Option<String>,
    pub connected: bool,
    pub first_seen_by: i64,
    pub headers_only: bool,
    pub work: String,
    pub txids: Option<Vec<u8>>,
    pub txids_added: Option<Vec<u8>>,
    pub txids_omitted: Option<Vec<u8>>,
    pub pool_name: Option<String>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub template_txs_fee_diff: Option<BigDecimal>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub tx_omitted_fee_rates: Option<BigDecimal>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub lowest_template_fee_rate: Option<BigDecimal>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub total_fee: Option<BigDecimal>,
    pub coinbase_message: Option<Vec<u8>>,
}

impl Block {
    pub fn get_latest(conn: &PgConnection) -> QueryResult<Block> {
        use crate::schema::blocks::dsl::*;
        blocks.order_by(height.desc()).first(conn)
    }

    pub fn get_with_fee_no_diffs(conn: &PgConnection, min_height: i64) -> QueryResult<Vec<Block>> {
        use crate::schema::blocks::dsl::*;

        blocks
            .filter(
                height
                    .gt(min_height)
                    .and(template_txs_fee_diff.is_null())
                    .and(total_fee.is_not_null()),
            )
            .load(conn)
    }

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
            SELECT height FROM blocks as b
			LEFT JOIN invalid_blocks as ib
			ON b.hash = ib.hash
            WHERE height > {}
			AND ib.node IS NULL
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
                    template_txs_fee_diff: None,
                    txids: None,
                    txids_added: None,
                    txids_omitted: None,
                    pool_name: None,
                    total_fee: None,
                    coinbase_message: None,
                    tx_omitted_fee_rates: None,
                    lowest_template_fee_rate: None,
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
            created_at: Some(Utc::now()),
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
            created_at: Some(Utc::now()),
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

#[derive(Clone, Serialize, Deserialize, QueryableByName, Queryable, Insertable)]
#[table_name = "transaction"]
pub struct Transaction {
    pub block_id: String,
    pub txid: String,
    pub is_coinbase: bool,
    pub hex: String,
    pub amount: f64,
    pub address: String,
}

impl Transaction {
    pub fn create(
        conn: &PgConnection,
        addr: String,
        block: String,
        idx: usize,
        tx_id: &String,
        tx_hex: &String,
        tx_amount: f64,
    ) -> QueryResult<usize> {
        use crate::schema::transaction::dsl::*;

        let tx = Transaction {
            address: addr,
            block_id: block,
            is_coinbase: idx == 0,
            txid: tx_id.clone(),
            hex: tx_hex.clone(),
            amount: tx_amount,
        };

        diesel::insert_into(transaction)
            .values(tx)
            .on_conflict_do_nothing()
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
    pub created_at: DateTime<Utc>,
}

impl StaleCandidate {
    pub fn list_ge(conn: &PgConnection, block_height: i64) -> QueryResult<Vec<StaleCandidate>> {
        use crate::schema::stale_candidate::dsl::*;
        stale_candidate.filter(height.ge(block_height)).load(conn)
    }

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
            created_at: Utc::now(),
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

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
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
    pub last_polled: Option<DateTime<Utc>>,
    pub initial_block_download: bool,
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

    pub fn update(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::nodes::dsl::*;
        diesel::update(nodes.filter(id.eq(self.id)))
            .set(self)
            .execute(conn)
    }

    pub fn get_active_reachable(conn: &PgConnection) -> QueryResult<Vec<Node>> {
        use crate::schema::nodes::dsl::*;
        nodes
            .filter(
                mirror_rpc_port.is_not_null().and(
                    initial_block_download
                        .eq(false)
                        .and(unreachable_since.is_null()),
                ),
            )
            .load(conn)
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
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, QueryableByName)]
pub struct ConflictingBlock {
    #[sql_type = "sql_types::Text"]
    pub hash: String,
    #[sql_type = "sql_types::Array<sql_types::BigInt>"]
    pub valid_by: Vec<i64>,
    #[sql_type = "sql_types::Array<sql_types::BigInt>"]
    pub invalid_by: Vec<i64>,
}

impl InvalidBlock {
    pub fn get_recent_conflicts(conn: &PgConnection) -> QueryResult<Vec<ConflictingBlock>> {
        let raw_query = format!(
            "
			SELECT hash, array_agg(distinct valid_by) as valid_by, array_agg(distinct invalid_by) as invalid_by
			FROM (
				SELECT
					ivb.hash as hash,
					vb.node as valid_by,
					ivb.node as invalid_by
				FROM valid_blocks as vb
				INNER JOIN invalid_blocks as ivb
				ON vb.hash = ivb.hash
				WHERE ivb.created_at > now() - interval '15 minutes'
			) q
			GROUP BY hash
        ",
        );

        diesel::sql_query(raw_query).load(conn)
    }
}

#[derive(QueryableByName, Queryable, Insertable)]
#[table_name = "valid_blocks"]
pub struct ValidBlock {
    pub hash: String,
    pub node: i64,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, QueryableByName, Queryable, Insertable)]
#[table_name = "watched"]
pub struct Watched {
    pub address: String,
    pub created_at: DateTime<Utc>,
    pub watch_until: DateTime<Utc>,
}

impl Watched {
    pub fn insert(
        conn: &PgConnection,
        addresses: Vec<String>,
        duration: DateTime<Utc>,
    ) -> QueryResult<usize> {
        use crate::schema::watched::dsl::*;

        let watch_list: Vec<_> = addresses
            .into_iter()
            .map(|addr| Watched {
                address: addr,
                created_at: Utc::now(),
                watch_until: duration.clone(),
            })
            .collect();

        diesel::insert_into(watched)
            .values(watch_list)
            .on_conflict_do_nothing()
            .execute(conn)
    }

    pub fn clear(conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::watched::dsl::*;
        let utc_now = Utc::now();

        diesel::delete(watched)
            .filter(watch_until.lt(utc_now))
            .execute(conn)
    }

    pub fn fetch(conn: &PgConnection) -> QueryResult<Vec<Transaction>> {
        use crate::schema::transaction::dsl as tdsl;
        use crate::schema::watched::dsl as wdsl;
        use diesel::dsl::any;

        let watched: Vec<_> = wdsl::watched.load(conn)?;
        let watched: Vec<_> = watched.into_iter().map(|w: Watched| w.address).collect();

        let transactions: Vec<_> = tdsl::transaction
            .filter(tdsl::address.eq(any(watched)))
            .load(conn)?;

        Ok(transactions)
    }
}

#[derive(Clone, Serialize, QueryableByName, Queryable, Insertable)]
#[table_name = "lags"]
pub struct Lags {
    pub node_id: i64,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl Lags {
    pub fn purge(conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::lags::dsl::*;
        diesel::delete(lags).execute(conn)
    }

    pub fn insert(conn: &PgConnection, id: i64) -> QueryResult<usize> {
        use crate::schema::lags::dsl::*;

        let lag = Lags {
            node_id: id,
            created_at: Utc::now(),
            deleted_at: None,
            updated_at: Utc::now(),
        };

        diesel::insert_into(lags).values(lag).execute(conn)
    }

    pub fn list(conn: &PgConnection) -> QueryResult<Vec<Lags>> {
        use crate::schema::lags::dsl::*;
        lags.load(conn)
    }
}
