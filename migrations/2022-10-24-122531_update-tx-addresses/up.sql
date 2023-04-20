-- Your SQL goes here

DROP TABLE transaction_addresses;

CREATE TABLE transaction_addresses (
    created_at timestamptz not null,
	notified_at timestamptz,
	block varchar not null,
	receiving_txid varchar not null,
	sending_txid varchar not null,
	sending_vout bigint not null,
	receiving_vout bigint not null,
	sending_amount bigint not null,
	receiving varchar not null,
	sending varchar not null,
	satoshis bigint not null,
	height bigint not null,
	PRIMARY KEY(block, receiving, sending, sending_vout)
);

ALTER TABLE chaintips
ADD COLUMN parent_block varchar;
