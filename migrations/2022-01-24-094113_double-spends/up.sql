-- Your SQL goes here
CREATE TABLE stale_candidate (
	hash varchar not null,
	n_children int not null,
	height bigint not null,
	confirmed_in_one_branch_total float(53) not null,
	double_spent_in_one_branch_total float(53) not null,
	rbf_total float(53) not null,
	PRIMARY KEY(hash)
);

CREATE TABLE transaction (
	txid varchar not null,
	is_coinbase boolean not null,
	hex varchar not null,
	amount float(53) not null,
	PRIMARY KEY(txid)
)

--CREATE TABLE CONFIRMED_IN_ONE_BRANCH (
--	id bigint generated always as identity,
--	PRIMARY KEY(id)
--)
-- CONSTRAINT fk_node
--   FOREIGN KEY(node)
--     REFERENCES nodes(id)
--     ON DELETE CASCADE
