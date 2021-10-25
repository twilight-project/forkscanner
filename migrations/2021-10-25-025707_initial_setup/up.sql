-- Your SQL goes here

CREATE TABLE blocks (
	hash varchar primary key,
	height bigint not null,
	parent_hash varchar,
	connected boolean not null default false
);
