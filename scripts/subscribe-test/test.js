const WebSocket = require('websocket').w3cwebsocket;

var subscriptions = {};

const ws = new WebSocket('ws://localhost:8340');
ws.addEventListener('open', () => {
  console.log('Sending request');

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "active_fork",
    method: "subscribe_active_fork",
    params: null,
  }));

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "forks",
    method: "subscribe_forks",
    params: null,
  }));

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "validation_checks",
    method: "validation_checks",
    params: null,
  }));

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "invalid_block_checks",
    method: "invalid_block_checks",
    params: null,
  }));

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "lagging_nodes_checks",
    method: "lagging_nodes_checks",
    params: null,
  }));

  ws.send(JSON.stringify({
      jsonrpc: "2.0",
      id: "watched_address_checks",
      method: "watched_address_checks",
      params: {
          watch: [
              "cdef9ae998abe7d1c287d741ab9007de848294c0",
              "db0bda0eed1402f76e4a34602928e3ad8238394c",
              "cf1c64c811344a2f7788009c507e09116f53f156",
              "d40025b5afb37835bc59801338d805da96c512b8",
              "9ea88edcc8dd267bee4f5dda004dc441f49f2e3c",
              "cdd05206f338464b8588f313ce317189c91313f8",
              "10296848fff61bc2f50d95d70463f593cfab413c",
              "c840aad8664db1ebfae136c4858c3399b6c099ca",
              "74ce5e962f03adb4ba931dbfb304c317c0474f25",
              "c2e8e3e71aa423c5b72aced98c1985676c9355dc",
              "359fe406ae587618bf72da817fbffd50b20c1026",
              "2f31802c5b3adf8ddeeebfd1f6db283d78d95a47",
              "91a1e7bea08f334ff02d4c339b96d1671cfb44ee",
              "66b86d715288530bdaadade31eea5fe6aac2983f",
              "79e652b17217f6373e75bff99795d968c0869565",
              "dc93087ba95211a518349ef51d391aaf00ac34ba",
          ],
          watch_until: "2030-09-30T00:00:00.0Z",
      }
  }));
});


process.on('SIGINT', function () {
    console.log("Cancelling subscriptions");
    for ( const [key, id] of Object.entries(subscriptions)) {
        console.log(`Unsubscribe ${key} with id ${id}`);

        const method = `unsubscribe_${key}`;

        ws.send(JSON.stringify({
            jsonrpc: "2.0",
            id: method,
            method: method,
            params: { id: id },
        }));
    }
    process.exit();
});


ws.addEventListener('message', (message) => {
  const obj = JSON.parse(message.data);

  if (obj.id !== undefined) {
      console.log('Subscription id: ', obj.result);
      subscriptions[obj.id] = obj.result;
  } else {
      if (obj.method == "forks") {
          console.log(`Got forks method: ${JSON.stringify(obj.params)}`);
      } else if (obj.method == "watched_address_checks") {
          console.log(`Got watched address method: ${JSON.stringify(obj.params)}`);
      } else if (obj.method == "active_fork") {
          console.log(`Got active fork method: ${JSON.stringify(obj.params)}`);
      } else if (obj.method == "validation_checks") {
          console.log(`Got checks method: ${JSON.stringify(obj.params)}`);
      } else if (obj.method == "invalid_block_checks") {
          console.log(`Got invalid block checks method: ${JSON.stringify(obj.params)}`);
      } else if (obj.method == "lagging_nodes_checks") {
          console.log(`Got lagging node checks method: ${JSON.stringify(obj.params)}`);
      }
  }
});

console.log('Starting');
