-- Your SQL goes here
ALTER TABLE transaction_addresses DROP CONSTRAINT transaction_addresses_pkey;
ALTER TABLE transaction_addresses ADD PRIMARY KEY (block, receiving_txid, sending_txid, sending_vout, receiving_vout);
