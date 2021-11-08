use bitcoincore_rpc::bitcoincore_rpc_json::GetBlockHeaderResult;
use diesel::prelude::*;
use diesel::result::QueryResult;

use crate::schema::{blocks, chaintips};


#[derive(Debug, QueryableByName, Queryable, Insertable)]
#[table_name = "chaintips"]
pub struct Chaintip {
    pub id: i64,
    pub node: String,
    pub status: String,
    pub block: String,
    pub height: i64,
    pub parent_chaintip: Option<i64>,
}

impl Chaintip {
    pub fn purge(conn: &PgConnection) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        use diesel::dsl::not;
        diesel::delete(chaintips)
            .filter(not(status.eq("active")))
            .execute(conn)
    }

    pub fn set_invalid_fork(conn: &PgConnection, block_height: i64, hash: &String, node_id: &String) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        diesel::insert_into(chaintips)
            .values((
                node.eq(node_id),
                block.eq(hash),
                height.eq(block_height),
                status.eq("invalid")
            ))
            .execute(conn)
    }

    pub fn set_valid_fork(conn: &PgConnection, block_height: i64, hash: &String, node_id: &String) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        diesel::insert_into(chaintips)
            .values((
                node.eq(node_id),
                block.eq(hash),
                height.eq(block_height),
                status.eq("valid-fork")
            ))
            .execute(conn)
    }

    pub fn set_active_tip(conn: &PgConnection, block_height: i64, hash: &String, node_id: &String) -> QueryResult<usize> {
        use crate::schema::chaintips::dsl::*;
        let tip = chaintips.filter(
            status.eq("active").and(node.eq(node_id))
        ).first::<Chaintip>(conn);

        match tip {
            Ok(tip) => {
                if &tip.block != hash {
                    // remove parent chaintip references....
                    diesel::update(
                        chaintips.filter(parent_chaintip.eq(tip.id))
                    ).set(parent_chaintip.eq::<Option<i64>>(None))
                        .execute(conn)?;

                    diesel::update(
                        chaintips.filter(id.eq(tip.id))
                    ).set((
                        block.eq(hash),
                        height.eq(block_height),
                        parent_chaintip.eq::<Option<i64>>(None)
                    ))
                          .execute(conn)
                } else {
                    Ok(0)
                }
            }
            Err(diesel::result::Error::NotFound) => {
                diesel::insert_into(chaintips)
                    .values((
                        node.eq(node_id),
                        status.eq("active"),
                        block.eq(hash),
                        height.eq(block_height)
                    ))
                    .execute(conn)
            }
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
    pub marked_valid: Option<String>,
    pub marked_invalid: Option<String>,
}

impl Block {
    pub fn get_or_create(conn: &PgConnection, header: &GetBlockHeaderResult) -> QueryResult<Block> {
        use crate::schema::blocks::dsl::*;
        let block = blocks
            .find(header.hash.to_string())
            .first::<Block>(conn);

        if let Err(diesel::result::Error::NotFound) = block {
            let prev_hash = header.previous_block_hash.map(|h| h.to_string());

            let block = Block {
                hash: header.hash.to_string(),
                height: header.height as i64,
                parent_hash: prev_hash,
                marked_valid: None,
                marked_invalid: None,
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

    pub fn set_valid(conn: &PgConnection, block_hash: &String, node: &String) -> QueryResult<usize> {
        use crate::schema::blocks::dsl::*;
        diesel::update(
            blocks.filter(hash.eq(block_hash).and(marked_valid.is_null())
        )).set(marked_valid.eq(node))
            .execute(conn)
    }

    pub fn set_invalid(conn: &PgConnection, block_hash: &String, node: &String) -> QueryResult<usize> {
        use crate::schema::blocks::dsl::*;
        diesel::update(
            blocks.filter(hash.eq(block_hash).and(marked_invalid.is_null())
        )).set(marked_invalid.eq(node))
            .execute(conn)
    }

    pub fn insert(&self, conn: &PgConnection) -> QueryResult<usize> {
        diesel::insert_into(blocks::dsl::blocks)
            .values(self)
            .execute(conn)
    }

    pub fn find_fork(conn: &PgConnection) -> QueryResult<Vec<(Option<String>, i64)>> {
        use crate::schema::blocks::dsl as bd;
        let forks = bd::blocks
            .filter(bd::parent_hash.is_not_null())
            .select((bd::parent_hash, diesel::dsl::sql("count(*)")))
            .group_by(bd::parent_hash)
            .load::<(Option<String>, i64)>(conn)?;
        Ok(forks.into_iter().filter(|f| f.1 > 1).collect())
    }

    pub fn find_tips(conn: &PgConnection, hash: &String) -> QueryResult<Vec<(i64, String)>> {
        use crate::schema::blocks::dsl as bd;
        let mut parents = bd::blocks
            .filter(bd::parent_hash.is_not_null().and(bd::parent_hash.eq(hash)))
            .select((bd::height, bd::hash))
            .load::<(i64, String)>(conn)?;

        loop {
            let mut changed = false;
            let mut next_parents = vec![];
            for parent in parents.drain(..) {
                let items = bd::blocks
                    .filter(bd::parent_hash.is_not_null().and(bd::parent_hash.eq(parent.1.clone())))
                    .select((bd::height, bd::hash))
                    .load::<(i64, String)>(conn)?;
                if items.len() > 0 {
                    next_parents.extend(items);
                    changed = true;
                } else {
                    next_parents.push(parent);
                }
            }

            parents = next_parents;

            if !changed {
                break;
            }
        }
        Ok(parents)
    }
}
