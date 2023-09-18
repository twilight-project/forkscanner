-- This file should undo anything in `up.sql`
ALTER TABLE transaction_addresses DROP CONSTRAINT transaction_addresses_pkey;
ALTER TABLE transaction_addresses ADD PRIMARY KEY (block, receiving, sending, sending_vout);
