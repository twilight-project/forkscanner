insert into nodes
    (node, rpc_host, rpc_port, rpc_user, mirror_rpc_port, rpc_pass)
values
    ('local', '127.0.0.1', 8332, 'bitcoin', 9332, 'pass');

insert into nodes
    (node, rpc_host, rpc_port, rpc_user, rpc_pass)
values
    ('local', '127.0.0.1', 8332, 'bitcoin', '127.0.0.1', 'pass');
