use crate::Chaintip;
use diesel::prelude::PgConnection;
use jsonrpc_core::*;
use jsonrpc_http_server::*;
use r2d2::PooledConnection;
use r2d2_diesel::ConnectionManager;
use std::net::SocketAddr;
use std::str::FromStr;

type Conn = PooledConnection<ConnectionManager<diesel::PgConnection>>;

fn get_forks(conn: Conn, _params: Params) -> Result<Value> {
    if let Ok(it_works) = Chaintip::list_active(&conn) {
        println!("TJDEBUG stuff {:?}", it_works);
        Ok(serde_json::Value::String(
            serde_json::to_string(&it_works).unwrap(),
        ))
    } else {
        let err = types::error::Error {
            code: types::error::ErrorCode::InternalError,
            message: "Something's wrong".into(),
            data: None,
        };
        Err(err)
    }
}

/// RPC service endpoints for users of forkscanner.
pub fn run_server(listen: &str, db_url: &str) {
    let manager = ConnectionManager::<PgConnection>::new(db_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Connection pool");

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
