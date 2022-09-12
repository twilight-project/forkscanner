const WebSocket = require('websocket').w3cwebsocket;

var subscriptions = {};

const ws = new WebSocket('ws://my-coin-server:8340');
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
