use crate::Chaintip;
use diesel::prelude::PgConnection;
use jsonrpc_core::*;
use jsonrpc_http_server::*;
use r2d2_diesel::ConnectionManager;
use r2d2::PooledConnection;
use std::net::SocketAddr;
use std::str::FromStr;
use serde_json::json;

type Conn = PooledConnection<ConnectionManager<diesel::PgConnection>>;


fn get_forks(conn: Conn, params: Params) -> Result<Value> {
    if let Ok(it_works) = Chaintip::list_active(&conn) {
        println!("TJDEBUG stuff {:?}", it_works);
        Ok(json!(&it_works))
    } else {
        let err = types::error::Error {
            code: types::error::ErrorCode::InternalError,
            message: "Something's wrong".into(),
            data: None,
        };
        Err(err)
    }
}

pub fn run_server(listen: &str, db_url: &str) {
    let manager = ConnectionManager::<PgConnection>::new(db_url);
    let pool = r2d2::Pool::builder().build(manager).expect("Connection pool");

    let mut io = IoHandler::new();
    let p = pool.clone();
    io.add_sync_method("getforks", move |params: Params| {
        let conn = p.get().unwrap();
        get_forks(conn, params)
    });

    let server = ServerBuilder::new(io)
        .start_http(&SocketAddr::from_str(listen).unwrap())
        .expect("Failed to start RPC server");

    server.wait();
}
