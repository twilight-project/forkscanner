use bitcoin::{Amount, BlockHash};
use bitcoincore_rpc::Client;
use diesel::prelude::PgConnection;
use diesel::Connection;
use forkscanner::{
    Block, BtcClient, JsonTransaction, ScriptPubKey, Transaction, TransactionAddress, Vout, Watched,
};
use jsonrpc::simple_http::SimpleHttpTransport;
use log::{debug, error, info, trace};
use std::{
    collections::{HashMap, HashSet},
    iter::FromIterator,
    str::FromStr,
};

const HASHES: [&str; 10] = [
    "000000000000000000044714b7b17de0aef5f8bea4707bc19c7dbd0709e7738e",
    "00000000000000000004b0c42968368a24d411e52785ec763abb70bd279122d5",
    "00000000000000000001bc9b3f5d7930a42118878ae26b327314d464abc8eb5b",
    "00000000000000000002a096422cd44a3023a1a6e1dc11fc65182d4185f25a35",
    "0000000000000000000424df1f75a3555b81a2307277449a112d18db555669d1",
    "00000000000000000000740fd6ced56dd698fe2bdfaa715b80cfbc4698df0011",
    "00000000000000000003ed82223b429b084a3ea9d4fcf75b949369e3d9ccf76c",
    "00000000000000000000e02da916e2b4bc6c53d61e352d22dc53d2fdcd155f49",
    "000000000000000000006d367f9221dfdcbf9c0247357f9eb76ec07c0d4dce62",
    "00000000000000000004bad963c5ae2543f03c4221df3ae586b8f6552f4c6d78",
];

const BTC_URL: &str = "http://localhost:8088";
const DB_URL: &str = "postgres://forkscanner:forkscanner@localhost:5433/forkscanner";

fn main() {
    env_logger::init();
    let db_conn = PgConnection::establish(DB_URL).expect("DB conn failed");

    let transport = SimpleHttpTransport::builder()
        .url(BTC_URL)
        .expect("BAD stuff")
        .build();
    let jsonrpc = jsonrpc::client::Client::with_transport(transport);

    let client = Client::from_jsonrpc(jsonrpc);

    for hash in HASHES {
        fetch_transactions(&client, &db_conn, &hash.to_string(), true);
    }

    println!("DONE");
}

fn fetch_transactions<BC: BtcClient>(
    node: &BC,
    db_conn: &PgConnection,
    block_hash: &String,
    fetch_inputs: bool,
) {
    let processed = Transaction::block_processed(db_conn, block_hash);
    let inputs = TransactionAddress::inputs_processed(db_conn, block_hash.clone());

    if !fetch_inputs && (processed.is_err() || processed.unwrap()) {
        return;
    } else if fetch_inputs && (inputs.is_err() || inputs.unwrap()) {
        return;
    }

    let watchlist = match Watched::load(db_conn) {
        Ok(list) => HashSet::<String>::from_iter(list.into_iter().map(|l| l.address)),
        Err(e) => {
            error!("Watchlist load error {:?}", e);
            Default::default()
        }
    };

    let block_info = match node.get_block_verbose(block_hash.to_string()) {
        Ok(bi) => bi,
        Err(e) => {
            error!("RPC call failed {:?}", e);
            return;
        }
    };

    let bh = match node.get_block_header_info(&BlockHash::from_str(block_hash).expect("Bad hash")) {
        Ok(h) => h,
        Err(e) => {
            error!("Header info error {:?}", e);
            return;
        }
    };

    if let Err(e) = Block::get_or_create(db_conn, false, -1, &bh) {
        error!("Failed inserting header info to db {:?}", e);
        return;
    }

    let mut tx_addrs = Vec::new();
    let num_txs = block_info.tx.len();

    info!("Fetching transactions for {}", block_hash);
    for (idx, tx) in block_info.tx.into_iter().enumerate() {
        if idx % 100 == 0 {
            trace!("Fetching {} of {} txs", idx, num_txs);
        }

        let JsonTransaction {
            hex,
            txid,
            vin,
            vout,
            ..
        } = tx;

        let value = vout.iter().fold(0.0, |a, amt| a + amt.value);

        if fetch_inputs {
            let mut cache = HashMap::new();
            let out_addrs: HashSet<_> = vout
                .iter()
                .map(|out| out.script_pub_key.address.clone().unwrap_or_default())
                .collect();

            if watchlist.intersection(&out_addrs).count() == 0 {
                trace!("No watched addrs in this tx {:?}", txid);
                continue;
            }

            for out in vout {
                let Vout {
                    script_pub_key: ScriptPubKey { address, hex, .. },
                    value,
                    n,
                } = out;

                let to_address = address.unwrap_or(hex);

                let mut inputs = Vec::with_capacity(vin.len());

                for v in &vin {
                    if v.txid.is_none() {
                        info!("Vin has no txid! {:?}", txid);
                        continue;
                    }
                    let txid = v.txid.clone().unwrap();
                    let vout = v.vout.unwrap();

                    let x = if cache.contains_key(&txid) {
                        cache.get(&txid).unwrap()
                    } else {
                        let tx = match node.get_transaction(&txid) {
                            Ok(tx) => tx,
                            Err(e) => {
                                error!("Connection to archive node failed {:?}", e);
                                continue;
                            }
                        };
                        cache.insert(tx.txid.clone(), tx.clone());
                        cache.get(&tx.txid).unwrap()
                    };
                    let addr = x.vout[vout].script_pub_key.address.clone().unwrap();
                    let sats = Amount::from_btc(x.vout[vout].value).unwrap().to_sat();

                    inputs.push((txid, vout, n, addr, sats as i64));
                }

                tx_addrs.push((
                    txid.clone(),
                    to_address,
                    inputs,
                    Amount::from_btc(value).unwrap().to_sat(),
                ));
            }
        }

        if let Err(e) = Transaction::create(
            db_conn,
            false,
            block_hash.clone(),
            idx,
            &txid,
            &hex.unwrap(),
            value,
        ) {
            error!("Could not insert transaction {:?}", e);
        }
    }
    debug!("inserting {} txs", tx_addrs.len());

    if let Err(e) =
        TransactionAddress::insert(db_conn, block_hash.clone(), block_info.height, tx_addrs)
    {
        error!("Database update failed: {:?}", e);
    }
}
