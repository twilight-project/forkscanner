import argparse
import os
from bitcoinrpc.authproxy import AuthServiceProxy, JSONRPCException
import logging
import json


from werkzeug.wrappers import Request, Response
from werkzeug.serving import run_simple

from jsonrpc import JSONRPCResponseManager, dispatcher
from jsonrpc.utils import DatetimeDecimalEncoder


logging.basicConfig()
logging.getLogger("BitcoinRPC").setLevel(logging.INFO)


class RpcHandlers:
    def __init__(self, rpc_conn):
        self.rpc_conn = rpc_conn

    def get_blockchain_info(self):
        return self.rpc_conn.getblockchaininfo()

    def get_chain_tips(self):
        return self.rpc_conn.getchaintips()

    def get_block_from_peer(self, block_hash, peer_id):
        return self.rpc_conn.getblockfrompeer(block_hash, peer_id)

    def get_block_header(self, block_hash, verbosity):
        return self.rpc_conn.getblockheader(block_hash, bool_thing)

    def get_block_template(self, mode, rules, capabilities):
        return self.rpc_conn.getblocktemplate(mode, rules, capabilities)

    def get_block(self, block_hash, verbosity):
        return self.rpc_conn.getblock(block_hash, verbosity)

    def get_peer_info(self):
        return self.rpc_conn.getpeerinfo()

    def get_raw_transaction(self, txid, verbose, block_hash):
        return self.rpc_conn.getrawtransaction(txid, verbose, block_hash)

    def get_tx_out_set_info(self, hash_type):
        return self.rpc_conn.gettxoutsetinfo(hash_type)


if __name__ == '__main__':
    btc_ip = os.environ.get('BTC_IP', '66.42.108.221')
    rpc_user = os.environ.get('BITCOIN_RPC_USER', 'bitcoin')
    rpc_password = os.environ.get('BITCOIN_RPC_PASSWORD', 'pass')

    rpc_connection = AuthServiceProxy(f"http://{rpc_user}:{rpc_password}@{btc_ip}:8332")

    handlers = RpcHandlers(rpc_connection)

    @Request.application
    def application(request):
        dispatcher["getblockchaininfo"] = lambda: handlers.get_blockchain_info()
        dispatcher["getchaintips"] = lambda: handlers.get_chain_tips()
        dispatcher["getblockfrompeer"] = lambda block_hash, peer_id: handlers.get_block_from_peer(block_hash, peer_id)
        dispatcher["getblockheader"] = lambda block_hash, verbosity: handlers.get_block_header(block_hash, verbosity)
        dispatcher["getblocktemplate"] = lambda mode, rules, capabilites: handlers.get_block_template(mode, rules, capabilities)
        dispatcher["getblock"] = lambda block_hash, verbosity: handlers.get_block(block_hash, verbosity)
        dispatcher["getpeerinfo"] = lambda: handlers.get_peer_info()
        dispatcher["getrawtransaction"] = lambda txid, verbose, block_hash: handlers.get_raw_transaction(txid, verbose, block_hash)
        dispatcher["gettxoutsetinfo"] = lambda hash_type: handlers.get_tx_out_set_info(hash_type)

        response = JSONRPCResponseManager.handle(
            request.data, dispatcher)
        response = json.dumps(response.data, cls=DatetimeDecimalEncoder)
        return Response(response, mimetype='application/json')


    run_simple('localhost', 4000, application)
