use bitcoincore_rpc::bitcoincore_rpc_json::GetBlockHeaderResult;
use chrono::prelude::*;
use diesel::prelude::*;
use diesel::result::QueryResult;
use serde::Serialize;

use crate::schema::{blocks, chaintips, invalid_blocks, nodes, peers, valid_blocks};

#[derive(Serialize, Debug, AsChangeset, QueryableByName, Queryable, Insertable)]
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

#[derive(AsChangeset, QueryableByName, Queryable, Insertable)]
#[table_name = "blocks"]
pub struct Block {
    pub hash: String,
    pub height: i64,
    pub parent_hash: Option<String>,
    pub connected: bool,
    pub first_seen_by: i64,
    pub headers_only: bool,
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

    /// Fetch the entire list of descendants for the current block.
    pub fn descendants(&self, conn: &PgConnection) -> QueryResult<Vec<Block>> {
        let raw_query = format!(
            "
            WITH RECURSIVE rec_query AS (
                SELECT * FROM blocks WHERE hash = '{}'
                UNION ALL
                SELECT b.* FROM blocks b INNER JOIN rec_query r ON r.hash = b.parent_hash
            ) SELECT * FROM rec_query ORDER BY height ASC;
        ",
            self.hash
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
