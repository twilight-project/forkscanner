insert into nodes
    (node, rpc_host, rpc_port, rpc_user, rpc_pass, archive, mirror_host, mirror_rpc_port)
values
    ('archive', '143.244.138.170', 8332, 'bitcoin', 'Persario_1', true, NULL, NULL),
    ('archive', '137.184.186.227', 8332, 'bitcoin', 'Persario_1', false, '167.71.141.175', 8332),
    ('archive', '143.244.136.166', 8332, 'bitcoin', 'Persario_1', false, NULL, NULL);
