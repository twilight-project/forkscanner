use bitcoincore_rpc::bitcoincore_rpc_json::GetBlockHeaderResult;
use chrono::prelude::*;
use diesel::prelude::*;
use diesel::result::QueryResult;

use crate::schema::{blocks, chaintips, invalid_blocks, nodes, valid_blocks};

#[derive(Debug, AsChangeset, QueryableByName, Queryable, Insertable)]
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
    pub fn update(&self, conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        diesel::update(chaintips.filter(id.eq(self.id)))
            .set(self)
            .execute(conn)
    }

    pub fn get_active(conn: &PgConnection, node_id: i64) -> QueryResult<Chaintip> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(node.eq(node_id).and(status.eq("active")))
            .first(conn)
    }

    pub fn get_invalid(
        conn: &PgConnection,
        hash: &String,
    ) -> QueryResult<Chaintip> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(block.eq(hash).and(status.eq("invalid")))
            .first(conn)
    }

    pub fn list_invalid_gt(
        conn: &PgConnection,
        tip_height: i64,
    ) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(height.gt(tip_height).and(status.eq("invalid")))
            .load(conn)
    }

    pub fn list_active_gt(
        conn: &PgConnection,
        tip_height: i64,
    ) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(height.gt(tip_height).and(status.eq("active")))
            .load(conn)
    }

    pub fn list_active_lt(
        conn: &PgConnection,
        tip_height: i64,
    ) -> QueryResult<Vec<Chaintip>> {
        use crate::schema::chaintips::dsl::*;
        chaintips
            .filter(height.lt(tip_height).and(status.eq("active")))
            .load(conn)
    }

    pub fn purge(conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        use diesel::dsl::not;
        diesel::delete(chaintips)
            .filter(not(status.eq("active")))
            .execute(conn)
    }

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

#[derive(QueryableByName, Queryable, Insertable)]
#[table_name = "blocks"]
pub struct Block {
    pub hash: String,
    pub height: i64,
    pub parent_hash: Option<String>,
    pub connected: bool,
}

impl Block {
    pub fn get(conn: &PgConnection, block_hash: &String) -> QueryResult<Block> {
        use crate::schema::blocks::dsl::*;
        blocks.find(block_hash).first(conn)
    }

    pub fn get_or_create(conn: &PgConnection, header: &GetBlockHeaderResult) -> QueryResult<Block> {
        use crate::schema::blocks::dsl::*;
        let block = blocks.find(header.hash.to_string()).first::<Block>(conn);

        if let Err(diesel::result::Error::NotFound) = block {
            let prev_hash = header.previous_block_hash.map(|h| h.to_string());

            let block = Block {
                hash: header.hash.to_string(),
                height: header.height as i64,
                parent_hash: prev_hash,
                connected: false,
            };

            conn.transaction::<usize, diesel::result::Error, _>(|| {
                let _ = block.insert(conn)?;
                diesel::update(blocks.filter(parent_hash.eq(header.hash.to_string())))
                    .set(connected.eq(true))
                    .execute(conn)
            })?;

            Ok(block)
        } else {
            block
        }
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

    pub fn marked_invalid_by(conn: &PgConnection, block_hash: &String, node_id: i64) -> QueryResult<bool> {
        use crate::schema::invalid_blocks::dsl::*;

        let rows = invalid_blocks
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
    pub rpc_user: String,
    pub rpc_pass: String,
    pub rpc_port: i32,
    pub unreachable_since: Option<DateTime<Utc>>,
}

impl Node {
    pub fn list(conn: &PgConnection) -> QueryResult<Vec<Node>> {
        nodes::dsl::nodes.load(conn)
    }
}

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
