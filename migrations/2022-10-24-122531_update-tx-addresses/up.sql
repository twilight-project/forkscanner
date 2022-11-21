-- Your SQL goes here

DROP TABLE transaction_addresses;

CREATE TABLE transaction_addresses (
    created_at timestamptz not null,
	txid varchar not null,
	incoming varchar not null,
	outgoing varchar not null,
	satoshis bigint not null,
	PRIMARY KEY(txid, incoming, outgoing)
);

ALTER TABLE chaintips
ADD COLUMN parent_block varchar;
